// BRAINLLM production server (INTERNALS §11.1, §22, §31). Node ≥ 22, zero dependencies (node:http).
// `brain build` copies this file to dist/server.mjs when the app has `load` or `server:` functions; `brain start`
// (INTERNALS §34) runs it with the listening socket inherited as BRAIN_LISTEN_FD (else PORT/HOST) and with the
// environment it resolved from `.brain_env` (§36): this file reads no env file itself. It prints `brain:ready <pid>`
// once it accepts and drains on SIGTERM.
//
// It serves, in this order:
//   GET  /_health                → 200 {"ok":true,"build":"<sig>"}
//   GET  /health[/live|/ready]   → forwarded to the native backend, or 200 {"ok":true} without one
//   GET  /_brain/health[/live|/ready] → the same probe (mapped to /health…), never forwarded as a /_brain call
//   POST /_r/<hash>/<fn>         → RPC to a `server:` function (same origin + X-Brain-RPC; body, rate and time limits from brain.json `server`)
//   GET  routes in the manifest  → SSR: load(ctx) → render the view on the DOM shim → page with props and SEO head
//   GET  files in dist/          → static (the smallest accepted .zst/.br/.gz sibling; /_b/* immutable), clean URLs
//   anything else                → 404 page
// Every response carries the security headers; every HTML response gets a fresh CSP nonce on each <script>.
//
// dist/_server/manifest.json (emitted by brain build):
//   { v, build, core, routes: [{ path, view, layouts, server, page }], rpc: { <hash>: <module> }, notFound }
// `build` is the build signature (§35.7): sent as X-Brain-Build on every HTML response and in /_health.
// A page template (`page`) is the route's final HTML with `<!--brain:ssr-->` inside #brain-view and
// `<!--brain:props-->` inside <script type="application/json" id="brain-props">.
import { createServer as http, STATUS_CODES, request as httpRequest, Agent as HttpAgent } from 'node:http'
import { AsyncLocalStorage } from 'node:async_hooks'
import net from 'node:net'
import { installNativeSockets } from './native-sockets.mjs'
import { llmClient } from './llm.mjs'
import { BlockList, isIP } from 'node:net'
import { Readable } from 'node:stream'
import { pipeline } from 'node:stream/promises'
import zlib from 'node:zlib'
import { createCipheriv, createDecipheriv, createHash, createHmac, hkdfSync, randomBytes, randomUUID, timingSafeEqual } from 'node:crypto'
import { constants as FS, existsSync, readFileSync, realpathSync, statSync, lstatSync } from 'node:fs'
import { open } from 'node:fs/promises'
import * as Module from 'node:module'
import { basename, dirname, extname, join, normalize, resolve, sep } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'

const MIME = {
  '.html': 'text/html; charset=utf-8', '.js': 'text/javascript; charset=utf-8', '.mjs': 'text/javascript; charset=utf-8',
  '.brain': 'text/html; charset=utf-8', '.jsbrain': 'text/javascript; charset=utf-8', '.cssbrain': 'text/css; charset=utf-8',
  '.css': 'text/css; charset=utf-8', '.json': 'application/json', '.svg': 'image/svg+xml', '.png': 'image/png',
  '.jpg': 'image/jpeg', '.jpeg': 'image/jpeg', '.webp': 'image/webp', '.avif': 'image/avif', '.gif': 'image/gif',
  '.ico': 'image/x-icon', '.woff2': 'font/woff2', '.woff': 'font/woff', '.txt': 'text/plain; charset=utf-8',
  '.xml': 'application/xml', '.webmanifest': 'application/manifest+json', '.map': 'application/json',
  '.mp4': 'video/mp4', '.webm': 'video/webm', '.pdf': 'application/pdf', '.wasm': 'application/wasm',
  '.m3u8': 'application/vnd.apple.mpegurl', '.m3u': 'application/vnd.apple.mpegurl', '.m4s': 'video/iso.segment',
  '.mpd': 'application/dash+xml', '.ts': 'video/mp2t', '.vtt': 'text/vtt; charset=utf-8', '.mp3': 'audio/mpeg',
  '.md': 'text/markdown; charset=utf-8', // llms.txt copies of the pages (§33), as brain start's static server sends them
}
// feeds are .xml files with their own type (SEO, §32)
const FEEDS = { 'rss.xml': 'application/rss+xml; charset=utf-8', 'atom.xml': 'application/atom+xml; charset=utf-8' }
const SECURITY = {
  'strict-transport-security': 'max-age=63072000; includeSubDomains',
  'x-content-type-options': 'nosniff',
  'referrer-policy': 'strict-origin-when-cross-origin',
  'x-frame-options': 'DENY',
  'permissions-policy': 'camera=(), microphone=(), geolocation=(), payment=(), usb=()',
  'cross-origin-opener-policy': 'same-origin',
  // what serves the app and where it lives, for crawlers and LLMs that read headers (no version: that stays in the HTML)
  'x-generated-by': 'BRAINLLM (https://chrisifm.github.io/BRAINLLM/; https://github.com/chrisifm/BRAINLLM)',
}
// The build's policy (manifest `csp`, without hashes: the same one static hosting uses) plus this response's nonce in
// script-src; without it, a default that lets api() reach HTTPS APIs of other origins.
const CSP0 =
  "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob: https:; " +
  "font-src 'self' data:; connect-src 'self' https: wss:; frame-ancestors 'none'; base-uri 'self'; form-action 'self'; object-src 'none'"
const withNonce = (policy, nonce) =>
  /(^|;)\s*script-src\b/.test(policy)
    ? policy.replace(/((?:^|;)\s*script-src\b[^;]*)/, `$1 'nonce-${nonce}'`)
    : `${policy}; script-src 'self' 'nonce-${nonce}'`

const { env: ENV } = process // the only source of configuration
const json = v => JSON.stringify(v)
// JSON safe inside <script>: no `</script>`, no HTML comment openers, no line separators
const jsonInHtml = v => json(v ?? null).replace(/</g, '\\u003c').replace(/\u2028/g, '\\u2028').replace(/\u2029/g, '\\u2029')
const esc = s => String(s).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;')
const b64u = b => Buffer.from(b).toString('base64url')
const tag = s => createHash('sha1').update(s).digest('base64url').slice(0, 22)

// Errors a server function may throw on purpose: { errors: { field: msg } } → 422; { status, message } with 4xx → that
const LIST_OPTIONS = ['search','limit','offset','filter','sort','cursor','after','include','aggregate','count','fields','owner']
const FILTER_OPS = ['eq','ne','gt','gte','lt','lte','between','in','exists','prefix','iprefix','has']
export class HttpError extends Error {
  constructor(status, message, extra) {
    super(message)
    this.status = status
    Object.assign(this, extra)
  }
}
const bridgeBusy = message => new HttpError(503, message, { code: 'BRIDGE_BUSY', retryAfter: 1, fix: 'The server is at its limit of concurrent backend requests; retry after the Retry-After delay' })
// Plain deliberate 4xx signals are portable from server-only application modules; exception messages stay private.
function intendedStatus(error) {
  if (!error || typeof error !== 'object') return null
  const descriptor = Object.getOwnPropertyDescriptor(error, 'status')
  const status = descriptor && descriptor.value
  const plain = Object.getPrototypeOf(error) === Object.prototype || Object.getPrototypeOf(error) === null
  return Number.isInteger(status) && STATUS_CODES[status] && (error instanceof HttpError && status >= 400 && status <= 599 || plain && status >= 400 && status < 500) ? status : null
}
// A deliberate public failure → { status, message } for f.error: a plain `{ message }` without a status (400) or an
// author's Error subclass with a 4xx `status`. Plain `{ status, message }` keeps its message private (driver detail);
// built-in errors and bare `Error` messages stay private too.
const BUILTIN_ERRORS = new Set([Error, TypeError, RangeError, SyntaxError, ReferenceError, EvalError, URIError, AggregateError].map(c => c.prototype))
function publicError(error) {
  if (!error || typeof error !== 'object' || error instanceof HttpError) return null
  const proto = Object.getPrototypeOf(error), plain = proto === Object.prototype || proto === null
  const typed = error instanceof Error && !BUILTIN_ERRORS.has(proto)
  const status = plain ? (error.status === undefined ? 400 : 0) : error.status, message = error.message
  if (!(plain || typed) || typeof message !== 'string' || !message || message.length > 500) return null
  return Number.isInteger(status) && status >= 400 && status < 500 && STATUS_CODES[status] ? { status, message } : null
}
const REDIRECT_CODES = new Set([301, 302, 303, 307, 308])
const NOINDEX = 'noindex, nofollow'
function safeLocation(value) {
  if (typeof value !== 'string' || value.length > 8192 || !value.startsWith('/') || value.startsWith('//') || /[\\\x00-\x20\x7f]/.test(value) || /^\/%2f/i.test(value) || /%(?:0[0-9a-f]|1[0-9a-f]|5c|7f)/i.test(value) || /%20/i.test(value.split(/[?#]/,1)[0])) throw new HttpError(500, 'Invalid same-site redirect location')
  return value
}
// Internal links keep the page's language (§37): `route.hrefs` is the table of a page outside the default language, the
// one brain-links.js applies in the browser; `b` is the base path, added later here.
function linkTo(route, path) {
  const t = route && route.hrefs
  if (!t || typeof path !== 'string' || path[0] !== '/' || path[1] === '/') return path
  const m = /^[^?#]*/.exec(path)[0], key = m.replace(/\/+$/, '') || '/'
  if (key === t.p || key.startsWith(t.p + '/')) return path
  let n = t.s[key] || (new RegExp(t.re).test(key) && (key === '/' ? t.p : t.p + key))
  if (n && key !== '/' && m.endsWith('/') && !n.endsWith('/')) n += '/'
  return n ? n + path.slice(m.length) : path
}
function localizeLinks(route, html) {
  if (!route.hrefs) return html
  const b = (globalThis.__brainBase || '/').slice(0, -1) // render() set it for this page
  return html.replace(/<(?:a|form)\s[^>]*>/g, tag => /\shreflang=/.test(tag) ? tag : tag.replace(/(\s(?:href|action)=")(\/[^"]*)"/, (all, at, h) => {
    const a = b && (h + '/').startsWith(b + '/') ? h.slice(b.length) || '/' : h
    return at + (a === h ? linkTo(route, a) : b + linkTo(route, a)) + '"'
  }))
}

// `throw redirect('/login')` (or return it) from load/guards; same-site paths only.
export const redirect = (url, status = 303) => {
  if (!REDIRECT_CODES.has(status)) throw new HttpError(500, 'Invalid redirect status')
  return { redirect: safeLocation(url), status }
}
function mediaQuality(header, kind) {
  let specificity = -1, quality = 0
  for (const part of String(header).slice(0, 8192).split(',').slice(0, 64)) {
    const [raw, ...params] = part.trim().split(';'), name = raw.toLowerCase()
    const rank = name === kind ? 2 : name === kind.split('/')[0] + '/*' ? 1 : name === '*/*' ? 0 : -1
    if (rank < 0) continue
    let q = 1
    for (const param of params) {
      const [key, value] = param.trim().split('=')
      if (key.toLowerCase() === 'q') { q = Number(value); if (!value || !Number.isFinite(q) || q < 0 || q > 1) q = 0 }
    }
    if (rank > specificity || rank === specificity && q > quality) { specificity = rank; quality = q }
  }
  return quality
}
function wantsHtml(req) {
  const path = req.$brainPath || String(req.url || '/').split('?')[0]
  if (/^\/(?:api|_brain|_r)(?:\/|$)/.test(path)) return false
  const accept = req.headers.accept
  if (!accept) return true
  const quality = mediaQuality(accept, 'text/html')
  return quality > 0 && quality >= mediaQuality(accept, 'application/json')
}
function defaultError(status, rid, home = '/') {
  const title = STATUS_CODES[status] || 'Request failed'
  return `<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><meta name="robots" content="noindex,nofollow"><title>${status} — ${esc(title)}</title></head><body><main><h1>${status} — ${esc(title)}</h1><p>The request could not be completed. Try again or contact support with this reference.</p><p>Reference: <code>${esc(rid)}</code></p><a href="${esc(home)}">Return home</a></main></body></html>`
}

// Coding weights are transport preferences, not substring matches (br;q=0 explicitly forbids Brotli). Ties keep the
// server order: static files prefer their max-level .br, dynamic bodies prefer zstd (fastest at a Brotli-like ratio).
// zstd is offered only where node:zlib has it (Node 22.15+).
const ZSTD = typeof zlib.zstdCompressSync === 'function'
export function acceptedEncodings(header = '', order = ['br', 'zstd', 'gzip']) {
  const values = new Map()
  for (const part of String(header).split(',')) {
    const [name, ...params] = part.trim().split(';')
    let q = 1
    for (const p of params) {
      const [k,v] = p.trim().split('=')
      if (k && k.trim().toLowerCase() === 'q') { q = v && v.trim() ? Number(v) : NaN; if (!Number.isFinite(q) || q < 0 || q > 1) q = 0 }
    }
    values.set(name.toLowerCase(),q)
  }
  return order.filter(name => name !== 'zstd' || ZSTD).map(name => [name,values.get(name) ?? values.get('*') ?? 0]).filter(([,q]) => q > 0).sort((a,b) => b[1] - a[1]).map(([name]) => name)
}
// Response compression (docs/PERFORMANCE.md): text bodies of at least 1 KiB, negotiated per request; a body that
// already has an encoding, a `no-transform` cache policy or a status without content is sent as is.
const COMPRESSIBLE = /^(?:text\/|image\/svg\+xml|application\/(?:json|javascript|xml|[\w.-]+\+(?:json|xml)))/i
const DYNAMIC = ['zstd', 'br', 'gzip']
// Each Vary token once: a relayed backend header and the additions here must not repeat one (tokens are case-insensitive).
const varyOnce = v => {
  const seen = new Set()
  return String(v).split(',').map(t => t.trim()).filter(t => t && !seen.has(t.toLowerCase()) && seen.add(t.toLowerCase())).join(', ')
}
// Parameters that do not make a different page: the list of `<meta seo query>` (brain-check seo.rs, B0503).
const TRACKING = /^(?:utm_.*|gclid|fbclid|msclkid)$/
const cacheSearch = url => {
  const kept = new URLSearchParams([...url.searchParams].filter(([key]) => !TRACKING.test(key)))
  kept.sort()
  return kept.size ? '?' + kept : ''
}
// Messages this file writes itself, safe to log as the cause; anything else in an error message may hold private data.
const OWN_CAUSES = new Set(['Backend request unavailable', 'Backend request timed out', 'Backend response exceeds its byte budget', 'Backend request capacity is unavailable', 'Backend stream capacity is unavailable', 'Native file request failed', 'Native file response is invalid', 'Native resource response must be JSON', 'Native resource response is invalid', 'Native resource request failed', 'Invalid final response status', 'Invalid configured redirect location', 'timeout'])
const CODE_SHAPE = /^[A-Z][A-Z0-9_]{0,63}$/
const varyEncoding = v => (/accept-encoding/i.test(v || '') ? v : [v, 'Accept-Encoding'].filter(Boolean).join(', '))
function bodyEncoding(req, status, headers) {
  if (status < 200 || status === 204 || status === 206 || status === 304 || headers['content-encoding'] || !COMPRESSIBLE.test(headers['content-type'] || '') || /no-transform/i.test(headers['cache-control'] || '')) return null
  return acceptedEncodings(req.headers['accept-encoding'], DYNAMIC)[0] || ''
}
// One `bytes=` span of a file of `size` bytes (RFC 9110 §14): { start, end } inclusive, false when it cannot be
// satisfied (416), or null when the header is not one well-formed span (several, another unit, reversed): the whole
// file is sent then, which the RFC allows.
function byteRange(header, size) {
  const m = /^bytes=(\d*)-(\d*)$/.exec(String(header).trim())
  if (!m || (m[1] === '' && m[2] === '')) return null
  if (m[1] === '') return Number(m[2]) > 0 && size > 0 ? { start: Math.max(0, size - Number(m[2])), end: size - 1 } : false
  const start = Number(m[1])
  if (m[2] !== '' && Number(m[2]) < start) return null
  return start < size ? { start, end: m[2] === '' ? size - 1 : Math.min(Number(m[2]), size - 1) } : false
}
const encodeBody = (enc, buf) => enc === 'zstd' ? zlib.zstdCompressSync(buf) : enc === 'br' ? zlib.brotliCompressSync(buf, { params: { [zlib.constants.BROTLI_PARAM_QUALITY]: 5, [zlib.constants.BROTLI_PARAM_SIZE_HINT]: buf.length } }) : zlib.gzipSync(buf)
const encodeStream = enc => enc === 'zstd' ? zlib.createZstdCompress() : enc === 'br' ? zlib.createBrotliCompress({ params: { [zlib.constants.BROTLI_PARAM_QUALITY]: 5 } }) : zlib.createGzip()

function googleSignInPage(page) {
  let head = false, templates = 0
  const tags = /<!--[\s\S]*?-->|<(script|style|title|noscript)\b(?:"[^"]*"|'[^']*'|[^'">])*?>[\s\S]*?<\/\1\s*>|<([^'">]|"[^"]*"|'[^']*')*>/gi
  for (const match of page.matchAll(tags)) {
    const tag = match[0]
    if (tag.startsWith('<!--') || match[1]) continue
    const name = /^<\/?([^\s/>]+)/.exec(tag)?.[1].toLowerCase(), closing = tag.startsWith('</')
    if (name === 'head' && !templates) { if (closing) return false; head = true; continue }
    if (name === 'body' && !templates) return false
    if (name === 'template') { templates = Math.max(0, templates + (closing ? -1 : 1)); continue }
    if (!closing && ['script', 'style', 'title', 'noscript'].includes(name)) return false
    if (!head || templates || closing || name !== 'meta') continue
    const attrs = new Map()
    for (const item of tag.slice(5, -1).matchAll(/([^\s=/]+)(?:\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>]+)))?/g)) {
      const key = item[1].toLowerCase(); if (!attrs.has(key)) attrs.set(key, item[2] ?? item[3] ?? item[4] ?? '')
    }
    // Google's and Apple's sign-in popups both answer through window.opener.
    if (['brain-google-signin', 'brain-apple-signin'].includes(attrs.get('name')?.toLowerCase()) && attrs.get('content') === '1') return true
  }
  return false
}

// One-use provider cookies (Google nonce, Apple nonce, LinkedIn state): relayed to the backend and cleared at logout.
const PROVIDER_COOKIES = ['__Host-brain-google', '__Host-brain-apple', '__Host-brain-linkedin']
// The native backend owns state, nonce and provider configuration. Only this fixed protocol destination is external.
function providerStartRedirect(method, path, status, location) {
  if (!['GET', 'HEAD'].includes(method) || path !== '/_brain/auth/linkedin/start' || status !== 303 || typeof location !== 'string' || /[\\\x00-\x20\x7f]/.test(location)) return false
  try {
    const target = new URL(location)
    return target.origin === 'https://www.linkedin.com' && target.pathname === '/oauth/v2/authorization' && !target.username && !target.password && !target.hash
  } catch { return false }
}
// One configured loopback authority. No user-controlled target, proxy, redirect, auth cache or pending queue.
// ctx.backend.files(bucket): the same file calls as job and route handlers (docs/backend/files.md).
function bucketClient(target, key, bucket, readBudgetMs = () => 0) {
  const UUID = /^[a-f0-9]{8}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{12}$/i
  if (typeof bucket !== 'string' || !/^[a-z][a-z0-9_]{0,47}$/.test(bucket)) throw new HttpError(400, 'Invalid native bucket name')
  const base = '/_brain/files/' + bucket
  const idPath = id => { if (typeof id !== 'string' || !UUID.test(id)) throw new HttpError(404, 'No file has this id; file ids are UUIDs'); return base + '/' + id }
  // A read waits for a draining or restarting backend like ctx.backend reads do (nativeAuthBridge); a write never repeats.
  const send = async (method, path, body, timeout) => {
    const end = Date.now() + (method === 'GET' ? readBudgetMs() : 0)
    for (let wait = 250; ; wait = Math.min(wait * 2, 2000)) {
      let response, failure, notNow = false
      try { response = await fetch(new URL(path, target), { method, redirect: 'error', signal: AbortSignal.timeout(timeout), headers: { 'x-api-key': key }, body }) } catch (e) {
        failure = new HttpError(503, 'Native backend is unavailable', { temporary: true }); notNow = e?.name !== 'TimeoutError'
      }
      if (response?.ok) return response
      if (response) {
        let code
        try { code = (await response.json())?.error?.code } catch {}
        const retryAfter = Number(response.headers.get('retry-after'))
        notNow = [502, 503, 504].includes(response.status) && (!code || code === 'DRAINING' || code === 'OVERLOADED')
        failure = new HttpError(response.status >= 400 && response.status < 500 ? response.status : 503, 'Native file request failed', { temporary: response.status === 429 || response.status === 503 || notNow, ...(typeof code === 'string' && /^[A-Z][A-Z0-9_]{0,63}$/.test(code) ? { code } : {}), ...(retryAfter > 0 ? { retryAfter } : {}) })
      }
      const pause = Math.max(wait, (failure.retryAfter || 0) * 1000)
      if (!notNow || Date.now() + pause > end) throw failure
      await new Promise(done => setTimeout(done, pause))
    }
  }
  const json = async response => { try { return await response.json() } catch { throw new HttpError(502, 'Native file response is invalid') } }
  return Object.freeze({
    async list(options = {}) {
      if (!options || typeof options !== 'object' || Array.isArray(options) || Object.keys(options).some(k => !['limit', 'offset', 'record'].includes(k))) throw new HttpError(400, 'Invalid native file list options')
      const { limit, offset, record } = options
      if ((limit !== undefined && (!Number.isInteger(limit) || limit < 1 || limit > 100)) || (offset !== undefined && (!Number.isInteger(offset) || offset < 0 || offset > 10000)) || (record !== undefined && !UUID.test(String(record)))) throw new HttpError(400, 'Invalid native file list options')
      const q = new URLSearchParams(Object.entries({ limit, offset, record }).filter(([, v]) => v !== undefined))
      return json(await send('GET', base + (q.size ? '?' + q : ''), undefined, 10000))
    },
    async meta(id) { return json(await send('GET', idPath(id) + '/meta', undefined, 10000)) },
    async read(id, { maxBytes = 16 * 1024 * 1024 } = {}) {
      if (!Number.isInteger(maxBytes) || maxBytes < 1 || maxBytes > 64 * 1024 * 1024) throw new HttpError(400, 'Invalid native file read limit')
      const response = await send('GET', idPath(id), undefined, 30000)
      if (Number(response.headers.get('content-length')) > maxBytes) { await response.body?.cancel(); throw new HttpError(413, 'The file exceeds the read limit; raise maxBytes up to 64 MiB') }
      const bytes = Buffer.from(await response.arrayBuffer())
      if (bytes.length > maxBytes) throw new HttpError(413, 'The file exceeds the read limit; raise maxBytes up to 64 MiB')
      return { bytes, contentType: response.headers.get('content-type'), etag: response.headers.get('etag') }
    },
    async upload({ filename, contentType = 'application/octet-stream', data, record } = {}) {
      if (typeof filename !== 'string' || !filename || Buffer.byteLength(filename) > 255 || typeof contentType !== 'string' || (record !== undefined && !UUID.test(String(record)))) throw new HttpError(400, 'Invalid native file upload')
      if (typeof data !== 'string' && !(data instanceof Uint8Array)) throw new HttpError(400, 'File data must be a string, Buffer or Uint8Array')
      const form = new FormData()
      form.set('file', new Blob([data], { type: contentType }), filename)
      return json(await send('POST', base + (record === undefined ? '' : '?record=' + record), form, 60000))
    },
    async remove(id) { await send('DELETE', idPath(id), undefined, 10000); return null },
  })
}
// Failures of a native read that may be asked again (a gateway error, DRAINING, OVERLOADED), marked where built.
const askAgain = new WeakSet()
// `readBudget`: ms (or a function giving them) a read may spend waiting for a draining or restarting backend; the
// server passes its load timeout minus a second, so the page still answers within it.
export function nativeAuthBridge(raw, refused, serviceKey, report, ownSession, readBudget = 8000) {
  const budget = typeof readBudget === 'function' ? readBudget : () => readBudget
  const readBudgetMs = () => Math.max(0, Math.min(Number(budget()) || 0, 30000))
  // How to fix the configuration is for the operator: a visitor of a production app reads a generic message.
  const configuration = (status, detail, rid, shown = 'This service is not available right now') => {
    if (process.env.NODE_ENV !== 'production') return new HttpError(status, detail)
    report?.('error', 'native backend configuration', { rid, fix: detail })
    return new HttpError(status, shown)
  }
  if (!raw) return null
  // The visitor's cookies only travel for a request from the site itself; the caller's log gets the reason (B0742).
  const bridgeOrigin = (incoming, rid, message) => {
    try {
      const origin = new URL(incoming.headers.origin)
      if (origin.origin === incoming.headers.origin && origin.host === (incoming.headers['x-forwarded-host'] || incoming.headers.host)) return origin.origin
    } catch {}
    if (refused) refused('B0742', incoming, rid, 'native')
    throw new HttpError(403, 'B0742: ' + message)
  }
  if (typeof raw !== 'string' || raw.length > 2048) throw Error('BRAIN_BACKEND_URL must be an explicit loopback HTTP origin')
  let target
  try { target = new URL(raw) } catch { throw Error('BRAIN_BACKEND_URL must be an explicit loopback HTTP origin') }
  if (target.protocol !== 'http:' || !['127.0.0.1', '[::1]', 'localhost'].includes(target.hostname) || target.username || target.password || target.pathname !== '/' || target.search || target.hash) throw Error('BRAIN_BACKEND_URL must be an explicit loopback HTTP origin')
  if (target.hostname === 'localhost') target.hostname = '127.0.0.1'
  const names = new Set(['__Host-brain', '__Host-brain-access', '__Host-brain-refresh'])
  const agent = new HttpAgent({ keepAlive: true, maxSockets: 8, maxTotalSockets: 8, maxFreeSockets: 2, timeout: 30000 })
  const owned = new Set()
  let active = 0, stopped = false
  const cookies = (req, includeRefresh) => {
    const selected = new Map(), header = req.headers.cookie || ''
    if (header.length > 16384) throw new HttpError(431, 'Authentication cookies exceed the request limit')
    for (const part of header.split(';')) {
      const at = part.indexOf('='); if (at < 0) continue
      const name = part.slice(0, at).trim(), value = part.slice(at + 1).trim()
      if (!names.has(name) || name === '__Host-brain-refresh' && !includeRefresh) continue
      if (selected.has(name) || value.length > 8192 || !/^[a-zA-Z0-9._-]*$/.test(value)) throw new HttpError(400, 'Invalid authentication cookies')
      selected.set(name, value)
    }
    return [...selected].map(([name, value]) => name + '=' + value).join('; ')
  }
  const safeUser = user => {
    if (!user || typeof user.id !== 'string' || !/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(user.id) || typeof user.email !== 'string' || user.email.length > 254 || typeof user.role !== 'string' || !/^[a-z][a-z0-9_]{0,47}$/.test(user.role) || ['public','owner'].includes(user.role) || typeof user.verified !== 'boolean') throw new HttpError(503,'Native authentication response is unavailable')
    const clean = {id:user.id,email:user.email,role:user.role,verified:user.verified}
    if (typeof user.name === 'string' && user.name.trim() && user.name.length <= 400 && [...user.name].length <= 200 && !/[\u0000-\u001f\u007f-\u009f]/.test(user.name)) clean.name = user.name.trim()
    if (typeof user.picture === 'string' && Buffer.byteLength(user.picture) <= 2048 && !/[\u0000-\u001f\u007f-\u009f]/.test(user.picture)) {
      try {
        const url = new URL(user.picture)
        if (url.protocol === 'https:' && url.hostname && !url.username && !url.password) clean.picture = user.picture
      } catch {}
    }
    return Object.freeze(clean)
  }
  const unavailable = (why = 'the backend did not answer') => new HttpError(503, `Native authentication is unavailable: ${why}`)
  const call = (incoming, response, path, method, rid, origin) => {
    if (stopped || active >= 8) return Promise.reject(unavailable(stopped ? 'the server is shutting down' : '8 session checks are already in flight'))
    const cookie = cookies(incoming, method === 'POST')
    const client = forwardedFor(incoming)
    active++
    return new Promise((resolve, reject) => {
      let outgoing, timer, finished = false
      const finish = (error, value) => {
        if (finished) return
        finished = true; active--; clearTimeout(timer)
        incoming.removeListener('aborted', cancel); response.removeListener('close', disconnected)
        if (outgoing) owned.delete(outgoing)
        error ? reject(unavailable()) : resolve(value)
      }
      const cancel = () => outgoing?.destroy(unavailable())
      const disconnected = () => { if (!response.writableEnded) cancel() }
      try {
        outgoing = httpRequest(new URL(path, target), { method, agent, headers: { accept: 'application/json', ...(cookie ? { cookie } : {}), 'x-request-id': rid, ...(client ? { 'x-forwarded-for': client } : {}), ...(origin ? { origin } : {}), ...(method === 'POST' ? { 'content-length': '0' } : {}) } }, backend => {
          let bytes = 0; const chunks = []
          if (backend.headers['content-encoding'] || Number(backend.headers['content-length']) > 16384) { backend.destroy(); return finish(true) }
          backend.on('data', chunk => { bytes += chunk.length; if (bytes > 16384) { backend.destroy(); finish(true) } else chunks.push(chunk) })
          backend.on('error', () => finish(true))
          backend.on('end', () => {
            if (backend.statusCode === 401 && method === 'GET') return finish(false, { user: null })
            if (backend.statusCode !== 200) return finish(true)
            let payload
            try { payload = JSON.parse(Buffer.concat(chunks).toString('utf8')) } catch { return finish(true) }
            if (method === 'GET') {
              try { return finish(false, {user:safeUser(payload?.user)}) } catch { return finish(true) }
            }
            if (payload?.ok !== true) return finish(true)
            const cleared = []
            for (const value of backend.headers['set-cookie'] || []) {
              if (value.length > 8192 || /[\r\n]/.test(value)) return finish(true)
              const name = value.split('=', 1)[0]
              if (names.has(name) || PROVIDER_COOKIES.includes(name)) cleared.push(value)
            }
            finish(false, { 'set-cookie': cleared, 'clear-site-data': '"storage"' })
          })
        })
        owned.add(outgoing); incoming.once('aborted', cancel); response.once('close', disconnected)
        timer = setTimeout(() => outgoing.destroy(unavailable('the backend took more than 2 s')), 2000)
        outgoing.once('error', () => finish(true)); outgoing.end()
      } catch { outgoing?.destroy(); finish(true) }
    })
  }
  // The access cookie lives 5 minutes and the session far longer: a password JWT session would read as anonymous on the
  // next page. The backend answers concurrent refreshes with the same token inside its grace window, so tabs and
  // prefetches cannot revoke the family. A denied refresh is anonymous; an unavailable backend stays an error.
  const renew = (req, res, rid) => {
    const selected = cookies(req, true)
    let origin
    try { origin = new URL(process.env.PUBLIC_SITE_URL).origin } catch { origin = 'https://' + (req.headers['x-forwarded-host'] || req.headers.host) }
    return new Promise((resolve, reject) => {
      if (stopped || active >= 8) return reject(unavailable(stopped ? 'the server is shutting down' : '8 session checks are already in flight'))
      active++
      let settled = false
      const done = (error, value) => { if (settled) return; settled = true; active--; clearTimeout(timer); error ? reject(unavailable()) : resolve(value) }
      const outgoing = httpRequest(new URL('/_brain/auth/refresh', target), { method: 'POST', agent, headers: { accept: 'application/json', cookie: selected, origin, 'x-request-id': rid, 'content-length': '0' } }, backend => {
        let bytes = 0; const chunks = []
        backend.on('data', chunk => { bytes += chunk.length; if (bytes > 16384) { backend.destroy(); done(true) } else chunks.push(chunk) })
        backend.on('error', () => done(true))
        backend.on('end', () => {
          if (backend.statusCode === 401 || backend.statusCode === 403) return done(false)
          if (backend.statusCode !== 200) return done(true)
          try {
            const user = safeUser(JSON.parse(Buffer.concat(chunks).toString('utf8'))?.user)
            const set = (backend.headers['set-cookie'] || []).filter(value => /^__Host-brain-(?:access|refresh)=/.test(value) && value.length <= 8192 && !/[\r\n]/.test(value))
            if (!set.some(value => value.startsWith('__Host-brain-access='))) return done(true)
            req.$nativeUser = user
            req.$nativeLoginHeaders = { 'set-cookie': set, 'cache-control': 'no-store' }
            // later backend calls of this same request use the new tokens
            const fresh = new Map(set.map(value => { const pair = value.split(';', 1)[0], at = pair.indexOf('='); return [pair.slice(0, at), pair.slice(at + 1)] }))
            const kept = (req.headers.cookie || '').split(';').map(part => part.trim()).filter(part => part && !fresh.has(part.split('=', 1)[0]))
            req.headers.cookie = [...kept, ...[...fresh].map(([name, value]) => name + '=' + value)].join('; ')
            done(false)
          } catch { done(true) }
        })
      })
      const timer = setTimeout(() => outgoing.destroy(unavailable('the backend took more than 2 s')), 2000)
      owned.add(outgoing); outgoing.once('close', () => owned.delete(outgoing))
      outgoing.once('error', () => done(true)); outgoing.end()
    })
  }
  let proxyBytes = 0, streams = 0
  // A file transfer lasts as long as the client needs, so streams have their own sockets and budget and never hold API slots.
  const streamAgent = new HttpAgent({ keepAlive: false, maxSockets: 32, maxTotalSockets: 32 })
  const REQUEST_HEADERS = ['origin','authorization','x-api-key','content-type','content-length','x-brain-rpc','x-brain-captcha','x-brain-login-grant','accept','if-match','if-none-match','x-brain-webhook-id','x-brain-webhook-timestamp','x-brain-webhook-signature','user-agent','dnt','sec-gpc','cf-ipcountry']
  const FILE_HEADERS = ['range','if-range','if-modified-since','if-unmodified-since']
  const RESPONSE_HEADERS = ['content-type','content-disposition','allow','retry-after','www-authenticate','location','etag','last-modified','clear-site-data','cache-control']
  const FILE_RESPONSE_HEADERS = ['content-length','content-range','accept-ranges','vary','content-security-policy','x-content-type-options','cross-origin-resource-policy','referrer-policy']
  // The identity the backend resolves through trustProxy: the hops already received plus the peer this gateway observed.
  // Hops to the left of an untrusted peer are ignored by the backend, so a client header can never choose the identity.
  const forwardedFor = req => {
    if (req.$brainForwardedFor) return req.$brainForwardedFor
    const peer = req.socket?.remoteAddress
    if (!peer) return undefined
    const prior = req.headers['x-forwarded-for']
    return typeof prior === 'string' && prior.length <= 4096 ? prior + ', ' + peer : peer
  }
  const requestHeaders = (req, rid, extra = []) => {
    const headers = { 'x-request-id':rid }
    for (const key of [...REQUEST_HEADERS, ...extra]) {
      const value = req.headers[key]
      if (typeof value === 'string' && value.length <= 8192) headers[key] = value
    }
    const selected = []
    for (const part of String(req.headers.cookie || '').split(';')) {
      const name = part.trim().split('=')[0]
      if (['__Host-brain','__Host-brain-access','__Host-brain-refresh',...PROVIDER_COOKIES].includes(name)) selected.push(part.trim())
    }
    if (selected.length) headers.cookie = selected.join('; ')
    const client = forwardedFor(req)
    if (client) headers['x-forwarded-for'] = client
    return headers
  }
  const nativeCookies = values => (values || []).filter(value => /^(?:__Host-brain(?:-access|-refresh|-google|-apple|-linkedin)?)=/.test(value) && value.length <= 8192).slice(0,8)
  // /_brain/files: both directions stream with backpressure. The bucket enforces its own maxBytes and access checks.
  function fileStream(req, res, url, rid) {
    if (stopped || streams >= 32) throw bridgeBusy('Backend stream capacity is unavailable')
    if (!['GET','HEAD','POST','DELETE'].includes(req.method)) throw new HttpError(405, 'Method not allowed')
    const ceiling = 2 * 1024 * 1024 * 1024 + 1024 * 1024
    if (Number(req.headers['content-length'] || 0) > ceiling) throw new HttpError(413, 'Request body is too large')
    clearTimeout(req.$brainBodyGuard)
    streams++
    return new Promise((resolve, reject) => {
      let outgoing, deadline, done = false, sent = 0
      const finish = failure => {
        if (done) return
        done = true; streams--; clearTimeout(deadline); owned.delete(outgoing)
        req.unpipe(outgoing); req.removeListener('data', count); res.removeListener('close', closed)
        if (failure && !res.headersSent && !res.destroyed) return reject(failure)
        if (failure && !res.destroyed) res.destroy()
        outgoing?.destroy(); resolve()
      }
      const closed = () => finish()
      const count = chunk => { sent += chunk.length; if (sent > ceiling) finish(new HttpError(413, 'Request body is too large')) }
      const unavailable = transport => new HttpError(503, 'Backend request unavailable', { transport })
      try {
        outgoing = httpRequest(new URL(url.pathname + url.search, target), { agent:streamAgent, method:req.method, headers:requestHeaders(req, rid, FILE_HEADERS) }, backend => {
          const safe = { 'cache-control':'no-store' }
          for (const key of [...RESPONSE_HEADERS, ...FILE_RESPONSE_HEADERS]) {
            const value = backend.headers[key]
            if (typeof value === 'string' && value.length <= 8192) safe[key] = value
          }
          const cookies = nativeCookies(backend.headers['set-cookie'])
          if (cookies.length) safe['set-cookie'] = cookies
          if (safe.vary) safe.vary = varyOnce(safe.vary)
          if (backend.statusCode >= 500) res.$brainCause = 'answered by the native backend'
          res.writeHead(backend.statusCode || 502, safe)
          backend.once('error', e => finish(unavailable(e.code)))
          backend.once('aborted', () => finish(unavailable('BACKEND_ABORTED')))
          backend.once('end', () => { res.end(() => finish()) })
          backend.pipe(res)
        })
        owned.add(outgoing)
        // No progress in either direction (a stalled backend, or a client that stopped reading and paused the socket).
        outgoing.setTimeout(60000, () => finish(new HttpError(504, 'Backend request timed out')))
        // The backend's own upload deadline is 300 s; nothing else needs a total limit after the headers.
        if (req.method === 'POST') deadline = setTimeout(() => finish(new HttpError(504, 'Backend request timed out')), 310000)
        outgoing.once('error', e => finish(unavailable(e.code)))
        res.once('close', closed)
        req.on('data', count)
        req.pipe(outgoing)
      } catch { finish(unavailable()) }
    })
  }
  function proxy(req, res, url, rid, reply, responseLimit = 20 * 1024 * 1024, declared) {
    if (stopped || active >= 8) throw bridgeBusy('Backend request capacity is unavailable')
    if (!['GET','HEAD','POST','PUT','PATCH','DELETE','OPTIONS'].includes(req.method)) throw new HttpError(405, 'Method not allowed')
    const post = req.method === 'POST'
    const limit = url.pathname === '/_brain/uploads' && post ? 5 * 1024 * 1024 + 65536 : declared || post && /^\/api\/[^/]+\/batch$/.test(url.pathname) ? 1024 * 1024 : 65536
    if (Number(req.headers['content-length'] || 0) > limit) throw new HttpError(413, 'Request body is too large')
    active++
    return new Promise((resolve, reject) => {
      let outgoing, timer, done = false, bytes = 0, requestBytes = 0, failure, transport
      const chunks = []
      const headers = requestHeaders(req, rid, declared?.headers)
      const finish = (error, backend) => {
        if (done) return
        done = true; clearTimeout(timer); active--; proxyBytes -= bytes; owned.delete(outgoing)
        req.removeListener('aborted',cancel); req.removeListener('data',data); req.removeListener('end',end)
        res.removeListener('close',cancel)
        if (error) { if (!res.destroyed) reject(failure || new HttpError(503,'Backend request unavailable',{transport})); else resolve(); return }
        const safe = { 'cache-control':'no-store' }
        for (const key of RESPONSE_HEADERS) {
          const value = backend.headers[key]
          if (typeof value === 'string' && value.length <= 8192) safe[key] = value
        }
        // the response headers a route contract allows its handler to set (docs/backend/routes.md)
        if (declared) for (const [key, value] of Object.entries(backend.headers)) {
          if ((key === 'link' || key === 'content-language' || key.startsWith('x-') && key !== 'x-request-id') && typeof value === 'string' && value.length <= 8192) safe[key] = value
        }
        const cookies = nativeCookies(backend.headers['set-cookie'])
        if (cookies.length) safe['set-cookie'] = cookies
        if (backend.statusCode >= 500 && !res.$capture) res.$brainCause = 'answered by the native backend'
        try { reply(res,backend.statusCode || 502,req.method === 'HEAD' ? '' : Buffer.concat(chunks,bytes),safe,providerStartRedirect(req.method,url.pathname,backend.statusCode,safe.location)); resolve() } catch (e) { reject(e) }
      }
      const cancel = () => { outgoing?.destroy(); finish(true) }
      const data = chunk => {
        requestBytes += chunk.length
        if (requestBytes > limit) { failure = new HttpError(413,'Request body is too large'); outgoing.destroy(); finish(true); return }
        if (!outgoing.write(chunk)) { req.pause(); outgoing.once('drain',()=>req.resume()) }
      }
      const end = () => outgoing.end()
      try {
        outgoing = httpRequest(new URL(url.pathname + url.search,target),{agent,method:req.method,headers},backend => {
          backend.on('data',chunk => {
            bytes += chunk.length; proxyBytes += chunk.length
            if (bytes > responseLimit || proxyBytes > 64 * 1024 * 1024) { failure = new HttpError(502,'Backend response exceeds its byte budget'); outgoing.destroy(); finish(true); return }
            chunks.push(chunk)
          })
          backend.once('end',()=>finish(false,backend)); backend.once('error',e=>{transport=e.code;finish(true)}); backend.once('aborted',()=>{transport='BACKEND_ABORTED';finish(true)})
        })
        owned.add(outgoing)
        timer = setTimeout(()=>{failure = new HttpError(504,'Backend request timed out');outgoing.destroy();finish(true)},declared || url.pathname === '/_brain/uploads' || url.pathname.startsWith('/_brain/uploads/') ? 35000 : 10000)
        outgoing.once('error',e=>{transport=e.code;finish(true)}); req.once('aborted',cancel); res.once('close',cancel)
        req.on('data',data); req.once('end',end)
        if (req.readableEnded) end()
      } catch { outgoing?.destroy(); finish(true) }
    })
  }
  const readCache = new Map()
  let readCacheBytes = 0
  const rememberRead = (path, etag, body) => {
    readCacheBytes -= readCache.get(path)?.body.length || 0
    readCache.delete(path)
    if (body.length > 1024 * 1024) return
    readCache.set(path, { etag, body })
    readCacheBytes += body.length
    while (readCache.size > 256 || readCacheBytes > 16 * 1024 * 1024) {
      const oldest = readCache.keys().next().value
      readCacheBytes -= readCache.get(oldest).body.length
      readCache.delete(oldest)
    }
  }
  // Reaches the request helper of a resource client without adding a method anyone can see on it.
  const OPERATION_CALL = Symbol('operation')
  const resource = (incoming, response, rid, name, serviceKey) => {
    if (typeof name !== 'string' || !/^[a-z][a-z0-9_]{0,47}$/.test(name)) throw new HttpError(400, 'Invalid native resource name')
    const idPath = id => {
      if (typeof id !== 'string' || !/^[a-f0-9]{8}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{12}$/i.test(id)) throw new HttpError(404, 'No record has this id; record ids are UUIDs')
      return '/api/' + name + '/' + id
    }
    const request = async (method, path, body, notFound) => {
      if (response.writableEnded || response.destroyed) throw Object.assign(new HttpError(409, 'The server request has ended'), { clientGone: true })
      if (method !== 'GET' && incoming.method !== 'POST') throw new HttpError(405, 'Native writes require a server function called through POST')
      let bytes = Buffer.alloc(0)
      if (body !== undefined) {
        if (!body || typeof body !== 'object' || Array.isArray(body)) throw new HttpError(400, 'Native resource values must be an object')
        try { bytes = Buffer.from(JSON.stringify(body)) } catch { throw new HttpError(400, 'Native resource values must be JSON serializable') }
        if (bytes.length > 65536) throw new HttpError(413, 'Native resource values exceed their byte budget')
      }
      const outgoing = () => {
        const req = Readable.from(bytes.length ? [bytes] : [])
        req.method = method
        req.$brainForwardedFor = forwardedFor(incoming)
        // A service call acts as the key's owner, never as the visitor: it sends no session.
        req.headers = { ...(serviceKey ? { authorization: 'Bearer ' + serviceKey } : ownSession ? {} : { cookie: cookies(incoming, false) }), accept: 'application/json', 'content-type':'application/json', 'content-length':String(bytes.length) }
        if (method !== 'GET') {
          req.headers.origin = bridgeOrigin(incoming, rid, 'Native writes require a same-origin server request')
        }
        return req
      }
      let req = outgoing()
      let value
      // Revalidated reads (ETag/304): a read made without any credential remembers its body, so an unchanged answer is not
      // sent or parsed again. Each answer is parsed afresh, so `load` can change what it gets. Signed-in reads stay out.
      const remembers = method === 'GET' && !req.headers.authorization && !req.headers.cookie
      const known = remembers ? readCache.get(path) : undefined
      const conditional = r => (known ? (r.headers['if-none-match'] = known.etag, r) : r)
      conditional(req)
      const answered = (_res, status, data, headers) => {
          if (status === 304 && known) {
            readCache.delete(path); readCache.set(path, known)
            value = JSON.parse(known.body)
            return
          }
          if (status === 422) {
            // the backend's own field messages (never driver text): a server function that rethrows answers 422 {errors}
            let fields
            try { fields = JSON.parse(data.toString('utf8'))?.error?.fields } catch {}
            const errors = fields && typeof fields === 'object' && !Array.isArray(fields) ? Object.fromEntries(Object.entries(fields).filter(([k, v]) => typeof v === 'string' && k.length <= 128).slice(0, 64).map(([k, v]) => [k, v.slice(0, 512)])) : {}
            throw new HttpError(422, 'Invalid input', { errors })
          }
          if (status === 404 && notFound) { value = notFound(); return }
          if (status < 200 || status >= 300) {
            // the message stays generic; the backend's own code and fix (never driver text) let server code branch on it
            let answer
            try { answer = JSON.parse(data.toString('utf8'))?.error } catch {}
            const code = typeof answer?.code === 'string' && /^[A-Z][A-Z0-9_]{0,63}$/.test(answer.code) ? answer.code : undefined
            const fix = code && typeof answer.fix === 'string' ? answer.fix.slice(0, 512) : undefined
            const wait = Number(headers['retry-after'])
            // A gateway error, or a backend draining/at capacity, is "not now": a read may ask again (below). A code
            // of the backend's own (BATCH_TOO_SLOW…) is an answer on purpose and is never repeated.
            const notNow = [502, 503, 504].includes(status) && (!code || code === 'DRAINING' || code === 'OVERLOADED')
            const failure = new HttpError(status >= 400 && status < 500 ? status : 503, NATIVE_FAILED, { code, fix, temporary: status === 429 || status === 503 || notNow, ...(wait > 0 ? { retryAfter: wait } : {}) })
            if (notNow) askAgain.add(failure)
            throw failure
          }
          if (status === 204) { value = null; return }
          if (!String(headers['content-type'] || '').startsWith('application/json')) throw new HttpError(502, 'Native resource response must be JSON')
          try { value = JSON.parse(data.toString('utf8')) } catch { throw new HttpError(502, 'Native resource response is invalid') }
          if (remembers && typeof headers.etag === 'string') rememberRead(path, headers.etag, data)
      }
      try {
        // An autodeploy drains the backend (503 DRAINING, then a closed port) and starts the new one: a read waits for
        // it within the load budget instead of failing the page, honoring Retry-After. A write is never repeated.
        const end = Date.now() + readBudgetMs()
        for (let wait = 250; ; wait = Math.min(wait * 2, 2000)) {
          try {
            await proxy(req, response, new URL(path, target), rid, answered, 2 * 1024 * 1024)
            break
          } catch (e) {
            const transient = ['ECONNREFUSED', 'ECONNRESET'].includes(e?.transport) || askAgain.has(e)
            const pause = Math.max(wait, (e?.retryAfter || 0) * 1000)
            if (method !== 'GET' || !transient || stopped || response.destroyed || Date.now() + pause > end) throw e
            req.destroy()
            await new Promise(done => setTimeout(done, pause))
            if (stopped || response.destroyed) throw e
            req = conditional(outgoing())
          }
        }
      } catch (e) {
        // A read that failed: `load` may catch it and render anyway, so the page answer says what happened (ssr below).
        // "Not now" is the backend limiting or unavailable: the same request can succeed later, so a page must not stand in for it.
        if (e && (e.transport || e.status === 504 || e.code === 'BRIDGE_BUSY')) e.temporary = true
        if (method === 'GET' && (e?.temporary || !(e?.status >= 400 && e.status < 500))) {
          const seen = incoming.$brainFailedReads || (incoming.$brainFailedReads = { reads: 0, backendCode: undefined, transport: undefined, temporary: false, retryAfter: undefined })
          seen.reads++
          seen.backendCode ??= e?.code
          seen.transport ??= e?.transport
          if (e?.temporary) { seen.temporary = true; seen.retryAfter = Math.max(seen.retryAfter || 0, e.retryAfter || 0) || undefined }
        }
        throw e
      } finally { req.destroy() }
      if (response.destroyed || response.writableEnded) throw Object.assign(new HttpError(409, 'The server request has ended'), { clientGone: true })
      return value
    }
    const names = (value, label) => {
      const list = typeof value === 'string' ? value.split(',') : value
      const shape = label === 'include' ? /^[a-z][a-z0-9_]{0,47}(?:\.[a-z][a-z0-9_]{0,47}){0,2}$/ : /^[a-z][a-z0-9_]{0,47}$/
      if (!Array.isArray(list) || list.length > 16 || list.some(item => typeof item !== 'string' || !shape.test(item))) throw new HttpError(400, `Invalid native ${label} names`)
      return list.join(',')
    }
    return Object.freeze({
      [OPERATION_CALL]: (operation, input) => request('POST', '/_brain/operations/' + operation, input),
      // the HTTP list contract (docs/backend/filters.md, sorting.md, pagination.md): the backend validates declared
      // filters and sorts and answers 422 {errors}; this only rejects shapes the query string cannot express
      list(options = {}) {
        if (!options || typeof options !== 'object' || Array.isArray(options) || Object.keys(options).some(key => !LIST_OPTIONS.includes(key))) throw new HttpError(400, 'Invalid native list options')
        const {search, limit, offset, filter, sort, cursor, after, include, aggregate, count, fields} = options
        if ((search !== undefined && (typeof search !== 'string' || Buffer.byteLength(search) > 256)) || (limit !== undefined && (!Number.isInteger(limit) || limit < 1 || limit > 100)) || (offset !== undefined && (!Number.isInteger(offset) || offset < 0 || offset > 10000))) throw new HttpError(400, 'Invalid native list pagination or search')
        const q = new URLSearchParams()
        if (filter !== undefined) {
          if (!filter || typeof filter !== 'object' || Array.isArray(filter) || Object.keys(filter).length > 32) throw new HttpError(400, 'Invalid native list filter')
          const scalar = value => {
            if (typeof value === 'number' && Number.isFinite(value)) return String(value)
            if (typeof value === 'boolean') return String(value)
            if (typeof value === 'string' && Buffer.byteLength(value) <= 256) return value
            throw new HttpError(400, 'Invalid native filter value')
          }
          for (const [field, condition] of Object.entries(filter)) {
            if (!/^[a-z][a-z0-9_]{0,47}(?:\.[a-z][a-z0-9_]{0,47})?$/.test(field)) throw new HttpError(400, 'Invalid native list filter field')
            const ops = condition && typeof condition === 'object' && !Array.isArray(condition) ? Object.entries(condition) : [['eq', condition]]
            if (!ops.length) throw new HttpError(400, 'Invalid native filter value')
            for (const [op, value] of ops) {
              if (!FILTER_OPS.includes(op)) throw new HttpError(400, 'Invalid native filter operator')
              let raw
              if (op === 'in' || op === 'between' || op === 'has') {
                const items = Array.isArray(value) ? value : [value]
                if (!items.length || items.length > 50 || (op === 'between' && items.length !== 2)) throw new HttpError(400, 'Invalid native filter value')
                raw = items.map(scalar)
                if (raw.some(item => item.includes(','))) throw new HttpError(400, 'Invalid native filter value')
                raw = raw.join(',')
              } else raw = scalar(value)
              q.append(op === 'eq' ? field : `${field}[${op}]`, raw)
            }
          }
        }
        if (search !== undefined) q.set('search', search)
        if (limit !== undefined) q.set('limit', String(limit))
        if (offset !== undefined) q.set('offset', String(offset))
        if (sort !== undefined) {
          const keys = typeof sort === 'string' ? sort.split(',') : sort
          if (!Array.isArray(keys) || !keys.length || keys.length > 4 || keys.some(key => typeof key !== 'string' || !/^-?[a-z][a-z0-9_]{0,47}$/.test(key))) throw new HttpError(400, 'Invalid native list sort')
          q.set('sort', keys.join(','))
        }
        if (cursor !== undefined && after !== undefined) throw new HttpError(400, 'Use either cursor or after')
        for (const [key, value] of [['cursor', cursor], ['after', after]]) if (value !== undefined) {
          if (typeof value !== 'string' || !/^[A-Za-z0-9_-]{1,2048}$/.test(value)) throw new HttpError(400, 'Invalid native list cursor')
          q.set(key, value)
        }
        for (const [key, value] of [['include', include], ['aggregate', aggregate], ['count', count], ['fields', fields]]) if (value !== undefined) q.set(key, names(value, key))
        if (options.owner !== undefined) {
          if (options.owner !== 'me') throw new HttpError(400, "Invalid native list owner; use 'me'")
          q.set('owner', 'me')
        }
        const query = q.toString()
        return request('GET', '/api/' + name + (query ? '?' + query : ''))
      },
      get(id, options = {}) {
        if (!options || typeof options !== 'object' || Array.isArray(options) || Object.keys(options).some(key => key !== 'include')) throw new HttpError(400, 'Invalid native get options')
        const path = idPath(id)
        return request('GET', options.include === undefined ? path : path + '?' + new URLSearchParams({include: names(options.include, 'include')}))
      },
      create: values => request('POST', '/api/' + name, values),
      update: (id, values) => request('PUT', idPath(id), values),
      // only the sent fields change (docs/backend/patch.md); null clears an optional field
      async patch(id, values) {
        if (values && typeof values === 'object' && !Array.isArray(values) && !Object.keys(values).length) throw new HttpError(422, 'Send at least one field', { errors: { body: 'Send at least one field to change' } })
        return request('PATCH', idPath(id), values)
      },
      // a declared atomic action or state transition (docs/backend/atomic-updates.md); the key still needs `<resource>:write`
      action: (id, action, input = {}) => request('POST', idPath(id) + '/actions/' + encodeURIComponent(action), input),
      transition: (id, transition, input = {}) => request('POST', idPath(id) + '/transitions/' + encodeURIComponent(transition), input),
      remove: id => request('DELETE', idPath(id)),
      // an earlier slug of a readable record → its current slug; live, unknown or unreadable slugs → null
      slugRedirect(slug) {
        if (typeof slug !== 'string' || !/^[A-Za-z0-9._~-]{1,512}$/.test(slug)) return Promise.resolve(null)
        return request('GET', '/api/' + name + '/redirects/' + slug, undefined, () => null).then(value => typeof value?.slug === 'string' ? value.slug : null)
      },
    })
  }
  // The token of the captcha widget: passed by the server function, or the header the page sent with this request.
  const captchaHeaders = (incoming,options) => {
    const token = options?.captcha ?? incoming.headers['x-brain-captcha']
    if (token === undefined) return {}
    if (typeof token !== 'string' || !token || Buffer.byteLength(token) > 8192 || /[\x00-\x1f\x7f]/.test(token)) throw new HttpError(403,'Complete the captcha again',{code:'CAPTCHA_FAILED'})
    return {'x-brain-captcha':token}
  }
  const captchaRefusal = (status,data) => {
    let code
    try {code=JSON.parse(data.toString('utf8'))?.error?.code}catch{}
    if(status===403&&code==='CAPTCHA_FAILED')return new HttpError(403,'Complete the captcha again',{code})
    if(status===503&&code==='CAPTCHA_UNAVAILABLE')return new HttpError(503,'The captcha could not be verified. Nothing was changed; retry shortly',{code})
    return null
  }
  async function login(incoming,response,rid,credentials,options) {
    if (incoming.method !== 'POST' || response.headersSent || response.writableEnded || response.destroyed) throw new HttpError(403,'B0742: Sign in through a same-origin POST server function')
    if (incoming.$nativeLoginStarted || incoming.$nativeLogout) throw new HttpError(409,'Use one authentication change per server request')
    if (!credentials || typeof credentials !== 'object' || Array.isArray(credentials) || Object.keys(credentials).some(key=>!['email','password'].includes(key)) || typeof credentials.email !== 'string' || typeof credentials.password !== 'string' || Buffer.byteLength(credentials.email)>254 || Buffer.byteLength(credentials.password)>1024) throw new HttpError(422,'Enter a valid email and password')
    const origin=bridgeOrigin(incoming,rid,'Sign in through a same-origin POST server function')
    incoming.$nativeLoginStarted=true
    const bytes=Buffer.from(JSON.stringify({email:credentials.email,password:credentials.password}))
    const request=Readable.from([bytes]);request.method='POST';request.$brainForwardedFor=forwardedFor(incoming);const grant=incoming.$nativeLoginGrant?.email===credentials.email?{'x-brain-login-grant':incoming.$nativeLoginGrant.grant}:captchaHeaders(incoming,options);delete incoming.$nativeLoginGrant;request.headers={origin,accept:'application/json','content-type':'application/json','content-length':String(bytes.length),...grant}
    let user
    try {
      await proxy(request,response,new URL('/_brain/auth/login',target),rid,(_res,status,data,headers)=>{
        const refusal=captchaRefusal(status,data);if(refusal)throw refusal
        if(status===403)throw new HttpError(403,"B0740: the backend refused this site's origin; fix: add the site origin to backend.origins in brain.json. The backend log has the seen and allowed origins. Run brain explain B0740")
        if(status!==200)throw new HttpError(status===400||status===401||status===422?401:status===429?429:503,status===429?'Too many attempts. Wait before trying again.':status===400||status===401||status===422?'Wrong email or password':'Sign-in is temporarily unavailable')
        let payload
        try {payload=JSON.parse(data.toString('utf8'))}catch{throw unavailable()}
        user=safeUser(payload?.user)
        const selected=headers['set-cookie']||[]
        if(new Set(selected.map(value=>value.split('=',1)[0])).size!==selected.length)throw unavailable()
        if(!selected.length||selected.length>3||selected.some(value=>!/^(?:__Host-brain|__Host-brain-access|__Host-brain-refresh)=/.test(value)||!/(?:^|;)\s*HttpOnly(?:;|$)/i.test(value)||!/(?:^|;)\s*Secure(?:;|$)/i.test(value)||!/(?:^|;)\s*Path=\/(?:;|$)/i.test(value)||!/(?:^|;)\s*SameSite=Lax(?:;|$)/i.test(value)))throw unavailable()
        if(response.writableEnded||response.destroyed)throw unavailable()
        incoming.$nativeLoginHeaders={'set-cookie':selected,'cache-control':'no-store'}
      },16384)
    } finally {request.destroy();bytes.fill(0)}
    return {user}
  }
  // Creates the account and leaves the session alone: `await ctx.backend.signup(v); return ctx.backend.login(v)` signs in
  // too. A refused address is the email field's error: the same 422 for an invalid one and for one already taken, and no
  // more than any signup form reveals by succeeding.
  async function signup(incoming,response,rid,credentials,options) {
    if (incoming.method !== 'POST' || response.headersSent || response.writableEnded || response.destroyed) throw new HttpError(403,'B0742: Sign up through a same-origin POST server function')
    if (!credentials || typeof credentials !== 'object' || Array.isArray(credentials) || Object.keys(credentials).some(key=>!['email','password'].includes(key)) || typeof credentials.email !== 'string' || typeof credentials.password !== 'string' || Buffer.byteLength(credentials.email)>254 || Buffer.byteLength(credentials.password)>1024) throw new HttpError(422,'Enter a valid email and password')
    if (Buffer.byteLength(credentials.password) < 12 || Buffer.byteLength(credentials.password) > 1024) throw new HttpError(422,'Use a password of 12 to 1024 bytes',{errors:{password:'Use a password of 12 to 1024 bytes'}})
    const origin=bridgeOrigin(incoming,rid,'Sign up through a same-origin POST server function')
    const bytes=Buffer.from(JSON.stringify({email:credentials.email,password:credentials.password}))
    const request=Readable.from([bytes]);request.method='POST';request.$brainForwardedFor=forwardedFor(incoming);request.headers={origin,accept:'application/json','content-type':'application/json','content-length':String(bytes.length),...captchaHeaders(incoming,options)}
    let user
    try {
      await proxy(request,response,new URL('/_brain/auth/signup',target),rid,(_res,status,data)=>{
        const refusal=captchaRefusal(status,data);if(refusal)throw refusal
        if(status===403)throw new HttpError(403,"B0740: the backend refused this site's origin; fix: add the site origin to backend.origins in brain.json. Run brain explain B0740")
        if(status===422)throw new HttpError(422,'This email cannot be used',{errors:{email:'This email cannot be used'}})
        if(status===429)throw new HttpError(429,'Too many attempts. Wait before trying again.')
        if(status!==200&&status!==201)throw unavailable()
        try {
          const payload=JSON.parse(data.toString('utf8'));user=safeUser(payload?.user)
          // The login that follows in this same request needs no second challenge; the grant never reaches the page.
          if(typeof payload?.login_grant==='string')incoming.$nativeLoginGrant={email:credentials.email,grant:payload.login_grant}
        }catch{throw unavailable()}
      },16384)
    } finally {request.destroy();bytes.fill(0)}
    return {user}
  }
  // Google needs its nonce bound to the browser that asked for it, so both steps relay the HttpOnly nonce cookie. The page
  // only ever sees the public client id and nonce it hands to Google.
  const sessionCookie = /^(?:__Host-brain|__Host-brain-access|__Host-brain-refresh|__Host-brain-google)=/
  const strictCookie = value => /(?:^|;)\s*HttpOnly(?:;|$)/i.test(value) && /(?:^|;)\s*Secure(?:;|$)/i.test(value) && /(?:^|;)\s*Path=\/(?:;|$)/i.test(value) && /(?:^|;)\s*SameSite=Lax(?:;|$)/i.test(value)
  async function googleStep(incoming,response,rid,path,payload,cookie,read) {
    if (incoming.method !== 'POST' || response.headersSent || response.writableEnded || response.destroyed) throw new HttpError(403,'Sign in through a same-origin POST server function')
    if (incoming.$nativeLoginStarted || incoming.$nativeLogout) throw new HttpError(409,'Use one authentication change per server request')
    let origin
    try { origin=new URL(incoming.headers.origin);if(origin.origin!==incoming.headers.origin || origin.host!==(incoming.headers['x-forwarded-host']||incoming.headers.host))throw Error() }catch{throw new HttpError(403,'Sign in through a same-origin POST server function')}
    incoming.$nativeLoginStarted=true
    const bytes=Buffer.from(JSON.stringify(payload))
    const request=Readable.from([bytes]);request.method='POST';request.headers={origin:origin.origin,accept:'application/json','content-type':'application/json','content-length':String(bytes.length),...(cookie?{cookie}:{})}
    let result
    try {
      await proxy(request,response,new URL(path,target),rid,(_res,status,data,headers)=>{
        if(status!==200)throw new HttpError(status===429?429:status<500?401:503,status===429?'Too many attempts. Wait before trying again.':status<500?'Google sign-in was not accepted':'Sign-in is temporarily unavailable')
        let parsed
        try {parsed=JSON.parse(data.toString('utf8'))}catch{throw unavailable()}
        const selected=headers['set-cookie']||[]
        if(new Set(selected.map(value=>value.split('=',1)[0])).size!==selected.length||!selected.length||selected.length>4||selected.some(value=>!sessionCookie.test(value)||!strictCookie(value)))throw unavailable()
        result=read(parsed)
        if(response.writableEnded||response.destroyed)throw unavailable()
        incoming.$nativeLoginHeaders={'set-cookie':selected,'cache-control':'no-store'}
      },16384)
    } finally {request.destroy();bytes.fill(0)}
    return result
  }
  const googleChallenge = (incoming,response,rid) => googleStep(incoming,response,rid,'/_brain/auth/google/challenge',{},null,payload=>{
    if(typeof payload?.client_id!=='string'||typeof payload?.nonce!=='string'||!/^[a-f0-9]{64}$/.test(payload.nonce)||!Number.isInteger(payload?.expires_in))throw unavailable()
    return {client_id:payload.client_id,nonce:payload.nonce,expires_in:payload.expires_in}
  })
  const googleLogin = (incoming,response,rid,values) => {
    if(!values||typeof values!=='object'||Array.isArray(values)||Object.keys(values).some(key=>key!=='credential')||typeof values.credential!=='string'||!values.credential||Buffer.byteLength(values.credential)>16384)throw new HttpError(422,'Send the credential Google returned')
    const nonce=String(incoming.headers.cookie||'').split(';').map(part=>part.trim()).find(part=>part.startsWith('__Host-brain-google='))?.slice('__Host-brain-google='.length)
    if(!nonce||!/^[a-f0-9]{64}$/.test(nonce))throw new HttpError(401,'Google sign-in was not accepted')
    return googleStep(incoming,response,rid,'/_brain/auth/google',{credential:values.credential},'__Host-brain-google='+nonce,payload=>{
      const user=safeUser(payload?.user)
      return {user}
    })
  }
  // ctx.backend for one server request (load, actions, RPC, dev sidecar): the same object everywhere
  const backend = (incoming, response, rid) => Object.freeze({
    resource: name => resource(incoming, response, rid, name),
    // A server-only resource (`api: false`) from trusted server code, with BRAIN_SERVICE_API_KEY
    service: name => {
      if (!serviceKey) throw configuration(503, 'ctx.backend.service needs BRAIN_SERVICE_API_KEY: create an API key with a scope naming the resource')
      return resource(incoming, response, rid, name, serviceKey)
    },
    // A declared file bucket from trusted server code, with BRAIN_SERVICE_API_KEY and its files.<bucket> scope
    files: bucket => {
      if (!serviceKey) throw configuration(503, 'ctx.backend.files needs BRAIN_SERVICE_API_KEY: create an API key with a files.<bucket> scope')
      return bucketClient(target, serviceKey, bucket, readBudgetMs)
    },
    slugRedirect: (name, slug) => resource(incoming, response, rid, name).slugRedirect(slug),
    // A declared operation, as this visitor (docs/backend/transactions.md).
    operation: (name, input = {}) => {
      if (typeof name !== 'string' || !/^[a-z][a-z0-9_]{0,47}$/.test(name)) throw new HttpError(400, 'Invalid operation name')
      return resource(incoming, response, rid, 'operations')[OPERATION_CALL](name, input)
    },
    login: (credentials, options) => login(incoming, response, rid, credentials, options),
    signup: (credentials, options) => signup(incoming, response, rid, credentials, options),
    google: Object.freeze({
      challenge: () => googleChallenge(incoming, response, rid),
      login: values => googleLogin(incoming, response, rid, values),
    }),
  })
  return {
    proxy,
    resource,
    backend,
    login,
    async prepare(req, res, rid) {
      if (!cookies(req, true)) { req.$nativeUser = null; req.$nativeAuth ||= Promise.resolve(); return }
      if (!req.$nativeAuth) req.$nativeAuth = (async () => {
        req.$nativeUser = cookies(req, false) ? (await call(req, res, '/_brain/auth/me', 'GET', rid)).user : null
        if (req.$nativeUser === null && /(?:^|;\s*)__Host-brain-refresh=/.test(req.headers.cookie || '')) await renew(req, res, rid)
      })()
      await req.$nativeAuth
    },
    session(req, res, rid) {
      return {
        user: req.$nativeUser,
        set() { throw configuration(409, 'With BRAIN_BACKEND_URL the backend owns the session: sign in with ctx.backend.login(credentials), or set BRAIN_BACKEND_SESSION=app so this app keeps its own signed session, and keep a server-only value with ctx.session.setSecret(value)', rid, 'Sign-in is not available right now') },
        clear() {
          if (req.$nativeLoginStarted) throw new HttpError(409,'Use a separate server request to sign out after signing in')
          if (req.method !== 'POST' || res.headersSent) throw new HttpError(403, 'B0742: Sign out through a same-origin POST request')
          const origin = bridgeOrigin(req, rid, 'Sign out through a same-origin POST request')
          if (!req.$nativeLogout) {
            req.$nativeLogout = call(req, res, '/_brain/auth/logout', 'POST', rid, origin).then(headers => { req.$nativeLogoutDone = true; req.$nativeLogoutHeaders = headers }, error => { req.$nativeLogoutDone = true; req.$nativeLogoutError = true; throw error })
            req.$nativeLogout.catch(() => {})
          }
          return req.$nativeLogout
        },
        headers: () => req.$nativeLoginHeaders || {},
      }
    },
    fileStream,
    _socketCookie: req => cookies(req, false),
    _forwardedFor: forwardedFor,
    close() { stopped = true; for (const request of owned) request.destroy(); agent.destroy(); streamAgent.destroy() },
  }
}

export function createServer(o = {}) {
  const dist = resolve(o.dist || process.cwd())
  // ctx.env: what the supervisor passed, read-only; the service key stays with ctx.backend.service
  const { BRAIN_SERVICE_API_KEY: serviceKey, BRAIN_JOBS_API_KEY: _jobs, BRAIN_JOBS_SYSTEM_KEY: _jobsSystem, BRAIN_ROUTES_API_KEY: _routes, BRAIN_ROUTES_SYSTEM_KEY: _routesSystem, ...visibleEnv } = o.env || ENV
  const env = Object.freeze(visibleEnv)
  const backendSession = o.backendSession ?? env.BRAIN_BACKEND_SESSION
  if (backendSession && backendSession !== 'app') throw Error("BRAIN_BACKEND_SESSION must be app, or unset for the backend's own session")
  const native = nativeAuthBridge(o.backendURL ?? env.BRAIN_BACKEND_URL, (...why) => originRefused(...why), serviceKey, (...entry) => log(...entry), backendSession === 'app', () => TIMEOUT - 1000)
  // Using the native backend does not have to mean it owns the visitor's session: `app` keeps the signed cookie.
  const nativeSession = native && backendSession !== 'app'
  if (o.nativeSockets !== undefined && typeof o.nativeSockets !== 'boolean') throw Error('nativeSockets must be an explicit boolean')
  const SITEMAP_BYTES = +(o.sitemapBytes ?? 50 * 1024 * 1024) // protocol: 50 MB uncompressed per sitemap file
  const secret = o.secret ?? env.BRAIN_SECRET ?? ''
  const COMPRESS = o.compress !== false && env.BRAIN_COMPRESS !== 'off' // a proxy that already compresses can opt out
  const sink = o.log || ((level, msg, extra) => console[level === 'error' ? 'error' : 'log'](json({ t: new Date().toISOString(), level, msg, ...extra })))
  // Requests in flight by rid: a 5xx whose cause reached no log line gets one when the response closes (dispatch below).
  const liveRequests = new Map()
  const hooked = new WeakSet() // core modules whose error hook reports to this server's log (render below)
  let statusPageWindow = 0, statusPageLogs = 0
  const log = (level, msg, extra = {}) => {
    const safe = { ...extra }
    if (level !== 'info' && extra.rid) { const live = liveRequests.get(extra.rid); if (live) live.logged = true }
    // Error messages may contain credentials or request data. Retain only bounded V8 source frames.
    if (typeof safe.stack === 'string') {
      const kind = /^(TypeError|ReferenceError|RangeError|SyntaxError|AggregateError|Error):/.exec(safe.stack)?.[1] || 'Error'
      safe.errorType = kind
      // "not now" (a 429/503, a backend restarting or busy) is not a defect: its own code, so alerts can tell them apart
      safe.code = safe.temporary ? 'NODE_TEMPORARY_ERROR' : kind === 'Error' ? 'NODE_INTERNAL_ERROR' : 'NODE_' + kind.replace(/Error$/, '').toUpperCase() + '_ERROR'
      const truncated = safe.stack.length > 32768 || safe.stack.split('\n').length > 41
      safe.traceTruncated = truncated
      safe.stack = safe.stack.slice(0, 32768).split('\n').filter(line => /^\s+at (?:async )?(?:[A-Za-z0-9_.$<>]+(?: \[as [A-Za-z0-9_$]+\])? \()?(?:file:\/\/\/|\/|node:)[^\s()?#]*:\d+:\d+\)?$/.test(line) && line.length <= 1024).slice(0, 40).join('\n')
      safe.private = true; safe.valuesCaptured = false
      safe.fix = safe.thrown ? `A non-Error value (${safe.thrown}) was thrown, so there are no source frames; throw an Error (or HttpError) instead` : safe.temporary ? 'A dependency answered not now (rate limit, restart, overload or a closed connection); the same request can succeed later. If it persists, check the backend health and its Retry-After' : kind === 'TypeError' ? 'Check nullable values, argument types and the data contract at these source frames' : kind === 'ReferenceError' ? 'Check the missing binding or import at these source frames' : kind === 'RangeError' ? 'Check recursive calls, array sizes and configured resource budgets at these source frames' : kind === 'SyntaxError' ? 'Check module syntax or JSON input parsing at these source frames without logging input values' : 'Inspect these source frames and the request reference; reproduce with synthetic inputs'
    }
    // a compiler budget failure (brain-budget.js) keeps its own code, limit, source position and fix
    const b = safe.budgetFailure
    delete safe.budgetFailure
    if (b) Object.assign(safe, b)
    if (typeof safe.path === 'string') safe.path = safe.path.split(/[?#]/, 1)[0].slice(0, 2048)
    sink(level, msg, safe)
  }
  // An origin refusal (B0741, B0742): the answer stays minimal and the operator's log gets the reason. It is bounded per
  // minute because anyone can trigger it, and it never names an allowlist: this server only compares against its own host.
  let originWindow = 0, originLogs = 0
  const originRefused = (code, req, rid, kind) => {
    const minute = Math.floor(Date.now() / 60000)
    if (originWindow !== minute) { originWindow = minute; originLogs = 0 }
    if (originLogs++ >= 16) return
    const bound = v => typeof v === 'string' ? v.slice(0, 256) : null
    const seenOrigin = bound(req.headers.origin), host = bound(req.headers.host), forwardedHost = bound(req.headers['x-forwarded-host'])
    const fix = seenOrigin
      ? `Origin ${seenOrigin} does not match the host this server sees (${forwardedHost || host}). Call it from the site itself; behind a proxy forward the host (nginx: proxy_set_header Host $host;). There is no allowlist to widen. Run brain explain ${code}`
      : `The request has no Origin${code === 'B0741' ? ' and no Sec-Fetch-Site' : ''}, so its site cannot be proven. Send it from the page's own origin in a browser; a script sets Origin to the site origin. Run brain explain ${code}`
    log('error', 'origin refused', { code, kind, rid, seenOrigin, secFetchSite: bound(req.headers['sec-fetch-site']), host, forwardedHost, fix })
  }
  // log fields for a thrown value: its stack (sanitized by log) plus, for B09xx budget failures, the typed fields only
  const trace = e => {
    const out = { stack: String((e && e.stack) || e) }
    // a thrown plain object, string or undefined has no stack: say what kind of value it was (never the value itself)
    if (!(e instanceof Error)) out.thrown = e === null ? 'null' : Array.isArray(e) ? 'array' : typeof e
    if (e && typeof e === 'object' && (e.temporary === true || e.status === 429 || e.status === 503)) out.temporary = true
    if (e instanceof HttpError && (OWN_CAUSES.has(e.message) || e.message.startsWith('Native authentication is unavailable:'))) out.cause = e.message
    if (e && e.code === 'ERR_MODULE_NOT_FOUND') out.fix = 'this process runs an older build whose files are gone; restart it after brain build'
    if (e && CODE_SHAPE.test(e.transport || '')) out.transport = e.transport
    if (e && typeof e.code === 'string' && CODE_SHAPE.test(e.code)) out.backendCode = e.code
    if (e && e.kind === 'budget' && /^B09\d\d$/.test(e.code) && typeof e.budget === 'string' && /^[a-zA-Z]{1,16}$/.test(e.budget)) {
      const str = (v, n) => typeof v === 'string' ? v.slice(0, n) : null, pos = v => Number.isSafeInteger(v) && v > 0 ? v : null
      out.budgetFailure = { code: e.code, budget: e.budget, limit: Number.isFinite(e.limit) ? e.limit : null, file: str(e.file, 256), component: str(e.component, 80), line: pos(e.line), column: pos(e.column), fix: str(e.fix, 200) || 'Bound the work or move it to an owned worker.' }
    }
    return out
  }
  const mf = readManifest(dist)
  // brain.json `server` (§11.1), written into the manifest by the build; createServer options and env still win
  const SL = mf.server || {}
  const TIMEOUT = +(o.timeout ?? env.BRAIN_TIMEOUT_MS ?? SL.timeoutMs ?? 10000)
  const LIMIT = +(o.rateLimit ?? env.BRAIN_RPC_LIMIT ?? SL.rpcPerMinute ?? 60) // per IP + function, per minute
  // Off by default: nginx below 1.29 takes a 103 as the final response, and the real one reaches the client as body bytes.
  const EARLY_HINTS = o.earlyHints ?? (env.BRAIN_EARLY_HINTS ? env.BRAIN_EARLY_HINTS === '1' : SL.earlyHints === true)
  const BODY_KB = +(o.maxBodyKB ?? env.BRAIN_MAX_BODY_KB ?? SL.maxBodyKB ?? 1024)
  if (!Number.isInteger(BODY_KB) || BODY_KB < 1 || BODY_KB > 65536) throw Error('brain.json server.maxBodyKB must be an integer from 1 to 65536 (KB)')
  const MAX_BODY = BODY_KB * 1024 // RPC and form bodies; bounded reads never buffer more
  const TOO_BIG = `Body larger than ${BODY_KB} KB; raise brain.json server.maxBodyKB`
  // a streamed route holds its 200 head this long for load() (#232): a load that settles in time still answers its
  // real status (303 redirect, 404 notFound, deliberate 4xx, 500); a slower one streams and handles them inline
  const STREAM_HOLD = +(o.streamHoldMs ?? 200)
  const tooBig = (rid, path) => (log('warn', 'body too large', { rid, path, limitKB: BODY_KB, fix: 'Raise brain.json server.maxBodyKB (or BRAIN_MAX_BODY_KB), up to 65536' }), TOO_BIG)
  const hasManifestSockets = Object.hasOwn(mf,'nativeSockets')
  if (hasManifestSockets && typeof mf.nativeSockets !== 'boolean') throw Error('Manifest nativeSockets must be an explicit boolean')
  const nativeSockets = o.nativeSockets === undefined ? hasManifestSockets && mf.nativeSockets : o.nativeSockets
  if (nativeSockets && !native) throw Error('Native sockets require a configured native backend; fix: run `brain backend serve` and set BRAIN_BACKEND_URL to its loopback URL')
  // the build's single route table (#187): already in match order, each with its RegExp source (named groups = params)
  const routes = mf.routes.map(r => ({ ...r, rx: r.rx ? new RegExp(r.rx) : exact(r.path) }))
  // The address RPC and form limits key on, ctx.req.ip and the logs: a client header counts only through a trusted proxy.
  const proxies = trustedProxies(o.trustProxy ?? (env.BRAIN_TRUST_PROXY ? env.BRAIN_TRUST_PROXY.split(',').map(entry => entry.trim()) : mf.trustProxy) ?? [])
  let proxyWarned = false
  const ipOf = req => {
    const forwarded = req.headers['x-forwarded-for']
    if (forwarded && !proxies && !proxyWarned && /^(?:127\.|::1$|::ffff:127\.)/.test(req.socket.remoteAddress || '')) {
      proxyWarned = true
      log('warn', 'trust-proxy-not-configured', { fix: 'a local proxy is forwarding client addresses and they are ignored, so all clients share one rate-limit key; set trustProxy in server/platform.jsbrain or BRAIN_TRUST_PROXY=127.0.0.1,::1' })
    }
    return resolveClient(req.socket.remoteAddress, forwarded, proxies)
  }
  // platform.access (docs/backend/access.md): /api, /_brain and /s answer this site only. Refused sources are counted
  // per client address (at most 512, the oldest dropped) for `brain access sources`, and logged at most 16 a minute.
  const access = accessPolicy(o.access ?? mf.access ?? {}, mf.site || env.PUBLIC_SITE_URL)
  const refusedSources = new Map()
  const budgetKey = `edge ${++servers} `
  let accessWindow = 0, accessLogs = 0, inFlight = 0, limitedWindow = 0, limited = 0, limitedLogs = 0
  // the dynamic budget (accessBudget) and the in-flight cap: past either the answer is an immediate 429 or 503 with
  // Retry-After, no queue, and one log line per minute sums what was turned away
  const overBudget = (req, res, path, client) => {
    const wait = access ? accessBudget(path, client, access, budgetKey) : 0
    const busy = !wait && access.concurrency !== null && !ACCESS_EXEMPT.test(path) && inFlight >= access.concurrency
    if (!wait && !busy) {
      if (access.concurrency !== null) { inFlight++; res.once('close', () => inFlight--) }
      return false
    }
    const minute = Math.floor(Date.now() / 60000)
    if (limitedWindow !== minute) {
      if (limited) log('warn', 'requests limited', { count: limited, minute: new Date(limitedWindow * 60000).toISOString(), fix: 'raise platform.access.perMinute or concurrency if real visitors are among them' })
      limitedWindow = minute; limited = 0; limitedLogs = 0
    }
    limited++
    if (limitedLogs++ < 4) log('warn', 'request limited', { ip: client, path: String(path).slice(0, 256), reason: busy ? 'concurrency' : 'perMinute', userAgent: typeof req.headers['user-agent'] === 'string' ? req.headers['user-agent'].slice(0, 256) : null })
    send(res, busy ? 503 : 429, busy ? 'Busy, try again shortly' : 'Too many requests', { 'content-type': 'text/plain; charset=utf-8', 'cache-control': 'no-store', 'retry-after': String(busy ? 2 : wait) })
    return true
  }
  const accessRefused = (req, rid, path, client, reason) => {
    const bound = v => typeof v === 'string' ? v.slice(0, 256) : null
    const now = new Date().toISOString()
    let entry = refusedSources.get(client)
    if (entry) refusedSources.delete(client)
    else {
      if (refusedSources.size >= 512) refusedSources.delete(refusedSources.keys().next().value)
      entry = { ip: client, count: 0, first: now, reasons: {} }
    }
    entry.count++; entry.last = now; entry.reasons[reason] = (entry.reasons[reason] || 0) + 1
    Object.assign(entry, { path: bound(path), origin: bound(req.headers.origin), referer: bound(req.headers.referer), host: bound(req.headers['x-forwarded-host'] || req.headers.host), userAgent: bound(req.headers['user-agent']), country: bound(req.headers['cf-ipcountry']) })
    refusedSources.set(client, entry)
    const minute = Math.floor(Date.now() / 60000)
    if (accessWindow !== minute) { accessWindow = minute; accessLogs = 0 }
    if (accessLogs++ >= 16) return
    log('warn', 'access refused', { code: 'B0752', reason, rid, ip: client, path: entry.path, origin: entry.origin, referer: entry.referer, secFetchSite: bound(req.headers['sec-fetch-site']), host: entry.host, userAgent: entry.userAgent, fix: 'the native API answers this site only; a page of the site, an API key, or an address in platform.access.ips passes. Run brain explain B0752' })
  }
  // server/routes/* of the native backend, listed by the build: only these exact shapes cross the bridge
  const nativeRoutes = (Array.isArray(mf.nativeRoutes) ? mf.nativeRoutes : []).map(route => {
    if (typeof route?.path !== 'string' || !Array.isArray(route.headers)) throw Error('Manifest nativeRoutes must list path and headers')
    return { headers: route.headers, rx: new RegExp('^' + route.path.split('/').map(part => /^\{.+\}$/.test(part) ? '[^/]+' : part.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')).join('/') + '$') }
  })
  // the real path: Node imports file URLs through realpath, and the /_b/ hook must name the same modules (a symlinked
  // dist such as /var → /private/var on macOS would otherwise load the core twice)
  const distURL = pathToFileURL(real(dist) + sep).href
  // Base path (§23 #148): the app lives under mf.base ("/app/"); routes, files and functions are matched on the app
  // path (base stripped); redirects the app gives as app paths get it back.
  const BASE = mf.base || '/'
  const analyticsTag = native && (o.analytics ?? env.BRAIN_ANALYTICS) !== 'off' && o.analytics !== false ? `<script type="module" async src="${BASE}_brain/analytics.js"></script>` : ''
  const RD = mf.redirects || {}, RDX = (RD.patterns || []).map(p => ({ re: new RegExp(p.re), to: p.to, status: p.status || 301 }))
  // `/old/` and `/old` are the same source (brain-seo normalizes the trailing slash)
  const redirectOf = path => {
    const p = path.length > 1 ? path.replace(/\/+$/, '') : path
    const ex = RD.exact && (RD.exact[p] || RD.exact[p + '/'])
    if (ex) return [ex[0], ex[1] || 301]
    for (const x of RDX) {
      const m = x.re.exec(path) || x.re.exec(p)
      if (m) return [x.to.replace(/\$(\d+)/g, (_, i) => m[+i] ?? ''), x.status]
    }
    return null
  }
  const withBase = u => (typeof u === 'string' && BASE !== '/' && u.startsWith('/') && !u.startsWith('//') && !(u + '/').startsWith(BASE) ? BASE + u.slice(1) : u)
  // §33 crawler-safe URLs: the redirect rule, the trailing-slash policy (brain.json seo.trailingSlash, default "never")
  // and the canonical host (seo.canonicalHost → site) are folded into ONE hop; loopback hosts are never redirected
  // (a proxy that does not forward Host, local runs). The query is kept unless a rule's target has its own.
  const SLASH = RD.slash || 'never', HOST = typeof RD.host === 'string' && /^https?:\/\//.test(RD.host) ? new URL(RD.host) : null
  const loopback = h => !h || /^(?:localhost|127(?:\.\d+){3}|\[::1\])(?::\d+)?$/i.test(h)
  const canonicalOf = (req, url) => {
    let path = url.pathname, query = url.search, status = 301, moved = false
    const to = redirectOf(path)
    if (to) {
      if (/^https?:\/\//.test(to[0])) return [to[0], to[1], true]
      const q = to[0].indexOf('?')
      path = q < 0 ? to[0] : to[0].slice(0, q)
      if (q >= 0) query = to[0].slice(q)
      status = to[1]
      moved = true
    }
    if (path !== '/' && SLASH === 'never' && path.endsWith('/')) path = path.replace(/\/+$/, '') || '/'
    else if (path !== '/' && SLASH === 'always' && !path.endsWith('/') && !/\.[^/]*$/.test(path)) path += '/'
    const host = HOST && !loopback(req.headers.host) && req.headers.host.toLowerCase() !== HOST.host
    if (!moved && !host && path === url.pathname) return null
    return [(host ? HOST.origin : '') + withBase(path) + query, status, host]
  }
  hook(distURL, BASE)

  // ---------- helpers ----------
  const send = (res, status, body, headers = {}, configuredExternal = false) => {
    if (native && res.req.$nativeLogout && !res.req.$nativeLogoutDone) {
      res.req.$nativeLogout.then(() => send(res, status, body, headers, configuredExternal), () => send(res, 503, '', {}, false))
      return
    }
    if (native && res.req.$nativeLogoutError) { status = 503; body = ''; headers = {}; res.req.$nativeLogoutError = false; res.$brainCause = 'native sign-out failed' }
    if (native && res.req.$nativeLogoutHeaders) {
      const relay = res.req.$nativeLogoutHeaders, own = headers['set-cookie']
      headers = { ...headers, ...relay, 'set-cookie': [...(own ? Array.isArray(own) ? own : [own] : []), ...relay['set-cookie']] }
    }
    if (!Number.isInteger(status) || status < 200 || status > 599 || !STATUS_CODES[status]) throw new HttpError(500, 'Invalid final response status')
    if (headers.location != null) {
      if (configuredExternal && /^https?:\/\//.test(headers.location)) {
        const target = new URL(headers.location)
        if (target.username || target.password || /[\\\x00-\x20\x7f]/.test(headers.location)) throw new HttpError(500, 'Invalid configured redirect location')
      } else safeLocation(headers.location)
    }
    if (status >= 400) {
      headers = { ...headers, 'cache-control': 'private, no-store', vary: [headers.vary, 'Accept'].filter(Boolean).join(', ') }
      const kind = headers['content-type'] || ''
      if (!kind || kind.startsWith('text/plain')) {
        const rid = res.getHeader('x-request-id') || ''
        if (wantsHtml(res.req)) {
          body = defaultError(status, rid, BASE)
          headers['content-type'] = MIME['.html']
          headers['content-security-policy'] = mf.csp || CSP0
        } else {
          body = json({ schemaVersion: 1, status, rid, error: STATUS_CODES[status] || 'Request failed' })
          headers['content-type'] = MIME['.json']
        }
        delete headers['content-length']; delete headers['content-encoding']; delete headers.etag
      }
    }
    const enc = COMPRESS ? bodyEncoding(res.req, status, headers) : null
    if (enc != null) {
      headers = { ...headers, vary: varyEncoding(headers.vary) }
      const size = body ? Buffer.byteLength(body) : 0
      if (enc && size >= 1024 && res.req.method !== 'HEAD') {
        body = encodeBody(enc, Buffer.isBuffer(body) ? body : Buffer.from(body))
        headers['content-encoding'] = enc
        delete headers['content-length']
      }
    }
    if (headers.vary) headers = { ...headers, vary: varyOnce(headers.vary) }
    res.writeHead(status, { ...SECURITY, ...(server.draining ? { connection: 'close' } : {}), ...headers })
    res.end(res.req.method === 'HEAD' || status === 204 || status === 205 || status === 304 ? undefined : body)
  }
  // HTML: `no-cache` + ETag (strong for files, weak for SSR) → a repeat visit gets 304 with no body (§35); the ETag is
  // taken before the per-response nonce is injected, so it is stable
  const html = (res, status, page, headers = {}, etag) => {
    // A 304 updates the cached document's policy too; keep it identical to its 200 without minting a CSP nonce.
    headers = { 'cache-control': 'no-cache', ...(googleSignInPage(page) ? { 'cross-origin-opener-policy': 'same-origin-allow-popups' } : {}), ...(mf.build ? { 'x-brain-build': mf.build } : {}), ...headers }
    if (etag && status === 200) {
      headers = { etag, ...headers }
      const inm = res.req.headers['if-none-match']
      if (inm && inm.split(',').some(t => t.trim().replace(/^W\//, '') === etag.replace(/^W\//, ''))) return send(res, 304, '', headers)
    }
    // RFC 9111 section 4.3.5 lets HEAD update cached GET headers without replacing its body.
    // As with 304, omit a new body-bound policy so the cached script nonce remains authorized.
    if (res.req.method === 'HEAD') return send(res, status, '', { 'content-type': MIME['.html'], ...headers })
    const nonce = randomBytes(16).toString('base64')
    // behavior analytics (docs/backend/analytics.md): with a native backend every page loads its collector as an async
    // module served by that backend, outside the build and the route budgets; BRAIN_ANALYTICS=off skips the tag
    if (analyticsTag && !page.includes(analyticsTag)) page = page.includes('</head>') ? page.replace('</head>', analyticsTag + '</head>') : page + analyticsTag
    const body = page.replace(/<script(?![^>]*\bnonce=)/g, `<script nonce="${nonce}"`)
    send(res, status, body, { 'content-type': MIME['.html'], 'content-security-policy': withNonce(mf.csp || CSP0, nonce), ...headers })
  }
  const notFound = (req, res) => failPage(res, 404, res.getHeader('x-request-id'))
  // `gone()` from load (§33): 410 tells crawlers the URL was removed for good; views/410.brain, else the 404 page
  const missing = (req, res, x) => (x && x.gone === true ? failPage(res, 410, res.getHeader('x-request-id')) : notFound(req, res))
  const failPage = (res, status, rid, detail) => {
    if (!wantsHtml(res.req)) return send(res, status, json({ schemaVersion: 1, status, rid, error: detail || STATUS_CODES[status] || 'Request failed' }), { 'content-type': MIME['.json'] })
    const custom = (status === 404 && mf.notFound) || (mf.errors && mf.errors[status]) || (status === 410 && !file('410.html') ? mf.notFound : `${status}.html`)
    const path = typeof custom === 'string' && file(custom)
    return html(res, status, path ? readFileSync(path, 'utf8') : defaultError(status, rid, BASE))
  }
  // `/403`, `/404.html`, `/503`…: the status when the app has that page, else 0 and the path is an ordinary one
  const reservedStatus = pathname => {
    const m = /^\/(403|404|410|5\d\d)(?:\.(?:html|brain))?$/.exec(pathname)
    if (!m) return 0
    const n = +m[1]
    return file(`${n}.html`) || (mf.errors && mf.errors[n]) || (n === 403 && mf.forbidden) || (n === 404 && mf.notFound) ? n : 0
  }
  // a file inside dist/ (never outside: no `..`, no absolute paths)
  function file(rel) {
    const p = normalize(join(dist, rel))
    if (p !== dist && !p.startsWith(dist + sep)) return null
    try {
      for (let cursor = p; cursor !== dist; cursor = dirname(cursor)) if (lstatSync(cursor).isSymbolicLink()) return null
      return statSync(p).isFile() ? p : null
    } catch {
      return null
    }
  }
  const mods = new Map()
  const load = rel => {
    if (BASE !== '/' && rel.startsWith(BASE)) rel = rel.slice(BASE.length) // '/app/_b/x.js' → dist/_b/x.js
    if (!mods.has(rel)) mods.set(rel, import(new URL(rel.replace(/^\//, ''), distURL).href))
    return mods.get(rel)
  }
  // Every module a request can import (the DOM shim, the core, each route's view, layouts, guards, middleware, server
  // module, dictionary, loading and error views, the RPC modules), imported once, one after another. `node server.mjs`
  // runs it before `brain:ready`, so the first request after a (re)start finds them loaded instead of reading,
  // compiling and evaluating the app's module graph (and its packages) while the visitor waits. A module that throws
  // stays rejected in `mods`, exactly as the request that imports it would have found it; it is counted, never fatal.
  const warm = async () => {
    const want = new Set(['_server/dom.js', mf.core])
    const add = r => {
      if (!r || typeof r !== 'object') return
      for (const k of ['view', 'server', 'loading', 'i18n']) if (typeof r[k] === 'string') want.add(r[k])
      for (const k of ['layouts', 'guards', 'middleware']) for (const m of r[k] || []) if (typeof m === 'string') want.add(m)
      add(r.error)
    }
    for (const r of mf.routes) add(r)
    add(mf.notFound)
    add(mf.forbidden)
    for (const m of Object.values(mf.rpc)) if (typeof m === 'string') want.add(m)
    for (const m of ['_server/brain-hydrate.js', '_server/brain-form.js']) if (file(m)) want.add(m)
    let failed = 0
    for (const rel of want) {
      try {
        await load(rel)
      } catch {
        failed++
      }
    }
    return { modules: want.size, failed }
  }

  const context = (req, res, url, extra) => {
    const cookies = parseCookies(req.headers.cookie)
    const { user, session: ses } = nativeSession ? { user: req.$nativeUser ?? null, session: sessionSecrets(native.session(req, res, res.getHeader('x-request-id')), cookies, secret) } : sessionFrom(cookies, secret)
    const signal = lifetime(res)
    return {
      url, query: Object.fromEntries(url.searchParams), cookies, user, session: ses, env,
      ...(native ? { backend: native.backend(req, res, res.getHeader('x-request-id')) } : {}),
      // `prefetch`: the router fetched this page ahead of a click (§19); a `load` with side effects can skip them
      prefetch: req.headers['x-brain-prefetch'] === '1' || /\bprefetch\b/.test(req.headers['sec-purpose'] ?? ''),
      req: { method: req.method, headers: req.headers, ip: ipOf(req) }, signal, llm: llmClient(env, signal), ...extra,
    }
  }

  // ---------- SSR ----------
  let queue = Promise.resolve() // the DOM shim is global: renders run one at a time (load runs concurrently)
  // forms: { '<hash>/<fn>': { values, errors, error } } — the state of a form posted without JS, re-rendered (§38)
  // `fixed`: render this route with these props (the 403 and _error pages): no guards, no load
  // `<meta cache="N">` (route.cache, #235): the SSR HTML of an anonymous GET is kept N s per URL (and language) and
  // served again; between N and 2N s the copy is served while a fresh one renders in the background
  // (stale-while-revalidate). Never for requests with a session cookie or Authorization, forms or error pages.
  const FILLING = new Map()
  const SSRC = new Map(), SSRC_MAX = 500, SSRC_BYTES = 32 * 1024 * 1024, SSRC_ENTRY = 1024 * 1024
  let ssrBytes = 0
  async function cached(req, res, route, params, url, rid) {
    const n = +route.cache, ms = n * 1000, key = (route.locale || '') + url.pathname + cacheSearch(url)
    // `kept.answer`: what this render answered when it was not a 200 (an error page, a redirect, a not-found), so the
    // request that started it replays that answer instead of running load() and its backend reads a second time
    const fill = (kept = {}) => {
      let failed = false
      const set = {}
      return new Promise(ok => {
        const cap = { req, $capture: (out, h, st) => {
          const headers = Object.fromEntries(Object.entries(h).map(([k, v]) => [k.toLowerCase(), v]))
          const privateResponse = headers['set-cookie'] || headers['vary'] || /private|no-store|no-cache/i.test(headers['cache-control'] || '')
          if (st >= 500) failed = true
          if (st !== 200) kept.answer = { page: out, h, st, set }
          ok(st === 200 ? { out, h, cacheable: !privateResponse, etag: 'W/"' + tag(out) + '"', t: Date.now(), bytes: Buffer.byteLength(out) } : null)
        }, writeHead(st, h) { if (st >= 500) failed = true; kept.answer = { st, h: h || {}, set } }, end: body => { if (kept.answer && kept.answer.page === undefined) kept.answer.body = body; ok(null) }, setHeader(k, v) { set[k] = v }, getHeader: name => (name === 'x-request-id' ? rid : set[name]), once() {}, removeListener() {} }
        ssr(req, cap, route, params, url, rid, null, 200, null, true).catch(() => ok(null))
      }).then(c => {
        // A failed revalidation keeps the copy until 2N, which is what `stale-if-error` promises downstream.
        if ((!c || !c.cacheable) && (failed || req.$brainFailedReads) && SSRC.has(key)) return c
        if (!c || !c.cacheable) {
          ssrBytes -= SSRC.get(key)?.bytes || 0
          SSRC.delete(key)
        }
        if (c && c.cacheable && c.bytes <= SSRC_ENTRY) {
          ssrBytes -= SSRC.get(key)?.bytes || 0
          SSRC.delete(key)
          SSRC.set(key, c)
          ssrBytes += c.bytes
          while (SSRC.size > SSRC_MAX || ssrBytes > SSRC_BYTES) {
            const oldest = SSRC.keys().next().value
            ssrBytes -= SSRC.get(oldest).bytes
            SSRC.delete(oldest)
          }
        }
        return c
      })
    }
    const serve = c => html(res, 200, c.out, { ...c.h, 'cache-control': c.cacheable ? `public, s-maxage=${n}, stale-while-revalidate=${n}, stale-if-error=${n}` : 'no-store' }, c.etag)
    let c = SSRC.get(key)
    const age = c ? Date.now() - c.t : Infinity
    if (c && age < 2 * ms) {
      if (age >= ms && !c.busy) (c.busy = true), fill().finally(() => (c.busy = false))
      return serve(c)
    }
    // Cold: the first request renders and the others that arrive meanwhile wait for it, so a burst runs load() once.
    // Only a shareable result is shared (a private one holds its visitor's cookie); anything else answers as usual, one by one.
    const running = FILLING.get(key)
    if (running) {
      c = await running
      if (c?.cacheable) return serve(c)
      return ssr(req, res, route, params, url, rid, null, 200, null, true)
    }
    const kept = {}
    const filling = fill(kept).finally(() => FILLING.delete(key))
    FILLING.set(key, filling)
    c = await filling
    if (c) return serve(c)
    // the first render already asked the backend and logged why; another would only spend more of a limit it just hit
    if (req.$brainFailedReads?.temporary) return comeBackPage(req, res, route, params, url, rid)
    delete req.$brainFailedReads
    // a redirect, a not-found or an error page: the answer that render produced, not a second load() (and its reads)
    const a = kept.answer
    if (a && !res.headersSent) {
      for (const [k, v] of Object.entries(a.set)) res.setHeader(k, v)
      if (a.page !== undefined) return html(res, a.st, a.page, a.h)
      res.writeHead(a.st, a.h)
      return void res.end(a.body)
    }
    return ssr(req, res, route, params, url, rid, null, 200, null, true) // nothing to replay (the render threw): as usual
  }
  const comeBackPage = (req, res, route, params, url, rid) => {
    res.setHeader('retry-after', String(Math.min(Math.ceil(req.$brainFailedReads.retryAfter) || 5, 3600)))
    res.setHeader('cache-control', 'no-store')
    return errorPage(req, res, route, params, url, rid, 503)
  }
  const anonymous = req => (req.method === 'GET' || req.method === 'HEAD') && !req.headers.authorization && !req.headers.cookie

  const preloadHints = new WeakMap()
  async function ssr(req, res, route, params, url, rid, forms = null, status = 200, fixed = null, nocache = false) {
    // the route's preloads (the same Link values as _headers): on the response, where hosts turn them into early hints,
    // capped so a long list cannot trip a proxy's header buffer. A 103 (#232) only when opted in, and never to HTTP/1.0.
    if (route.links && route.links.length && !res.$capture && !res.$hinted) {
      res.$hinted = true
      let selected = preloadHints.get(route)
      if (!selected) {
        selected = []
        let bytes = 0
        // Preserve graph order within a priority; render-blocking styles and fonts precede script hints.
        const priority = value => /(?:^|;)\s*as=(?:style|font)(?:;|$)/.test(value) ? 0 : 1
        for (const value of [...route.links].sort((a, b) => priority(a) - priority(b))) {
          const size = Buffer.byteLength(value) + (selected.length ? 2 : 0)
          if (bytes + size > 2048) continue
          selected.push(value); bytes += size
        }
        preloadHints.set(route, selected)
        if (selected.length < route.links.length) log('warn', 'preload hints trimmed to 2048 bytes; fix: reduce the route preload graph', { route: route.path, kept: selected.length, dropped: route.links.length - selected.length })
      }
      if (selected.length && !res.headersSent) res.setHeader('link', selected.join(', '))
      if (selected.length && EARLY_HINTS && res.writeEarlyHints && req.httpVersion !== '1.0') {
        try {
          res.writeEarlyHints({ link: selected })
        } catch {}
      }
    }
    if (!nocache && route.cache > 0 && !forms && !fixed && status === 200 && anonymous(req)) return cached(req, res, route, params, url, rid)
    if (native) await native.prepare(req, res, rid)
    const ctx = context(req, res, url, { params })
    // a page rendered for a signed-in user is never stored (§35): after logout, Back must not show the previous
    // user's page from the HTTP cache or bfcache. Anonymous pages keep `no-cache` + ETag.
    const own = ctx.user != null || ctx.cookies[COOKIE] !== undefined || ctx.cookies[SECRET_COOKIE] !== undefined || native && /(?:^|;\s*)__Host-brain(?:-access|-refresh)?=/.test(req.headers.cookie || '')
    const keep = h => (own ? { ...h, 'cache-control': 'private, no-store' } : h)
    let props = fixed || {}, extra = {}
    if (!fixed && ((route.guards && route.guards.length) || (route.middleware && route.middleware.length))) {
      let r
      try {
        const guards = [], middleware = []
        for (const g of route.guards || []) guards.push(await load(g))
        for (const m of route.middleware || []) middleware.push(await load(m))
        r = await pageLocale.run(route.locale, () => runChain({ guards, middleware, ctx }))
      } catch (e) {
        log('error', 'guard failed', { rid, path: url.pathname, ...trace(e) })
        return errorPage(req, res, route, params, url, rid)
      }
      // a guarded page must not reach the index through its login redirect (§33): the redirect itself says noindex
      if (r.redirect) return send(res, r.status, '', { location: withBase(linkTo(route, redirect(r.redirect, r.status ?? 303).redirect)), 'cache-control': 'no-store', 'x-robots-tag': NOINDEX, ...ctx.session.headers() })
      if (r.deny !== undefined) return forbidden(req, res, url, rid, r.deny)
      extra = r.headers
    }
    // what load() thrown or returned instead of props answers: redirect, notFound() (F11), deliberate status, failure
    const loadOutcome = e => {
      // the visitor left (a closed tab, an aborted prefetch): nobody reads an answer, and it is no failure of the app
      if (e && e.clientGone && res.destroyed) return log('info', 'client left during load', { rid, path: url.pathname })
      if (e && e.redirect) return send(res, e.status || 303, '', { location: withBase(linkTo(route, redirect(e.redirect, e.status ?? 303).redirect)), ...ctx.session.headers() })
      if ((e instanceof HttpError && e.status === 404) || (e && e.notFound === true)) return missing(req, res, e) // `notFound()` 404 / `gone()` 410
      // the backend or another service said "not now": a temporary 503 tells crawlers to come back, a 500 tells them it broke
      // a status the page chose itself (`new HttpError(429)`) stays as it is, with the same hint to come back
      if (e && (e.status === 429 || e.status === 503) && (!(e instanceof HttpError) || e.temporary) && !res.headersSent) {
        log('warn', 'load temporarily unavailable', { rid, path: url.pathname, status: 503, ...trace(e) })
        res.setHeader('retry-after', String(Math.min(Math.ceil(e.retryAfter) || 5, 3600)))
        res.setHeader('cache-control', 'no-store')
        return errorPage(req, res, route, params, url, rid, 503)
      }
      const status = intendedStatus(e) || 500
      log('error', 'load failed', { rid, path: url.pathname, status, ...trace(e) })
      if ((status === 429 || status === 503) && !res.headersSent) res.setHeader('retry-after', String(Math.min(Math.ceil(e.retryAfter) || 5, 3600)))
      return status === 500 ? errorPage(req, res, route, params, url, rid) : failPage(res, status, rid)
    }
    if (route.stream && route.server && !fixed && !forms && status === 200 && !res.$capture) return streamed(req, res, route, params, url, rid, ctx, keep(extra || {}), loadOutcome)
    try {
      if (!fixed && route.server) {
        const m = await load(route.server)
        if (route.i18n) await load(route.i18n) // t() inside load(): the dictionary is there before it runs
        if (typeof m.load === 'function') props = (await withTimeout(pageLocale.run(route.locale, () => Promise.resolve(m.load(ctx))), TIMEOUT, ctx)) ?? {}
      }
    } catch (e) {
      return loadOutcome(e)
    }
    if (props && (props.redirect || props.notFound === true)) return loadOutcome(props) // `return redirect()` / `return notFound()`
    if (req.$brainFailedReads && !fixed) {
      if (status === 200 && req.$brainFailedReads.temporary) return loadOutcome(comeBack(req, rid, url))
      softFailure(req, rid, url, status)
      extra = { ...extra, 'cache-control': 'no-store' } // what it rendered instead is not worth keeping
    }
    let body, seo, failed
    try {
      const at = { path: url.pathname, params: params || {}, query: Object.fromEntries(url.searchParams) } // route() (§41.5)
      ;({ body, seo, failed } = await (queue = queue.then(() => render(route, props, forms, at), () => render(route, props, forms, at))))
    } catch (e) {
      log('error', 'render failed', { rid, path: url.pathname, ...trace(e) })
      return failPage(res, 500, rid)
    }
    // a component that failed shows its fallback inside the page (still 200); the error goes to the log with the rid
    for (const f of failed || []) log('error', 'component failed during SSR', { rid, path: url.pathname, comp: f.comp, where: f.w, ...trace(f.e) })
    const page = readFileSync(file(route.page), 'utf8')
      .replace('<!--brain:ssr-->', () => body)
      .replace('<!--brain:props-->', () => jsonInHtml(props))
    // a page behind a guard passes it only for its visitor: it stays out of the index even when it renders (§33)
    if (route.guards && route.guards.length) res.setHeader('x-robots-tag', NOINDEX)
    // with seo.trailingSlash "ignore" both forms answer: the canonical takes the form the sitemap lists (no slash)
    let out = head(page, seo, SLASH === 'always' || url.pathname === '/' ? url.pathname : url.pathname.replace(/\/+$/, '') || '/', url.search, route, BASE.slice(0, -1))
    // route params for route() in the browser (the client has no route table)
    if (params && Object.keys(params).length) out = out.replace('</head>', () => `<meta name="brain-params" content="${esc(JSON.stringify(params))}"></head>`)
    if (forms) out = out.replace('</body>', () => `<script type="application/json" id="brain-forms">${jsonInHtml(forms)}</script></body>`)
    if (res.$capture) return res.$capture(out, keep({ ...extra, ...ctx.session.headers() }), status)
    html(res, status, out, keep({ ...extra, ...ctx.session.headers() }), own ? undefined : 'W/"' + tag(out) + '"')
  }
  // Streaming SSR (#232, `route.stream`: the build sets it when the view's <meta seo> is static and it has a load()):
  // load() starts first and the head waits up to STREAM_HOLD for it: a load that redirects, is not found or throws a
  // status in that time gets the same answer as a non-streamed route (`outcome`). Past it, the page up to the region
  // (head, CSS, layout shell) is sent, the view when load() ends. The status and headers are out by then, so a late
  // redirect becomes location.replace and a late failure an inline notice with the request id (both logged); load()
  // cannot change the session on a streamed route.
  // `load` caught a read the backend refused for now (429, 503, unreachable) and rendered without its data: a 200 would put
  // that page in the index and tell a crawler to keep going, so it answers 503 with Retry-After. The log says what was behind it.
  const comeBack = (req, rid, url) => {
    softFailure(req, rid, url, 503)
    return Object.assign(new Error('A backend read failed for now'), { status: 503, retryAfter: req.$brainFailedReads.retryAfter })
  }
  // `load` caught any other failed backend read and rendered: the status it asked for stands, the log says what is behind it.
  const softFailure = (req, rid, url, status) => {
    const { reads, backendCode, transport } = req.$brainFailedReads
    log('warn', 'page rendered after a failed backend read', { rid, path: url.pathname, status, reads, backendCode, transport, fix: 'load caught the failure and returned other props; let it throw for a 503, or show that the data is missing' })
  }
  async function streamed(req, res, route, params, url, rid, ctx, extra, outcome) {
    const [top, bottom = ''] = readFileSync(file(route.page), 'utf8').split('<!--brain:ssr-->')
    const sm = await load(route.server)
    if (route.i18n) await load(route.i18n)
    const loadP = typeof sm.load === 'function' ? withTimeout(pageLocale.run(route.locale, () => Promise.resolve().then(() => sm.load(ctx))), TIMEOUT, ctx) : Promise.resolve(null)
    loadP.catch(() => {}) // handled below; never an unhandled rejection while the loading view renders
    let timer
    const early = await Promise.race([loadP.then(v => ({ v }), e => ({ e })), new Promise(r => (timer = setTimeout(r, STREAM_HOLD)))])
    clearTimeout(timer)
    if (early && (early.e !== undefined || (early.v && (early.v.redirect || early.v.notFound === true)))) return outcome(early.e !== undefined ? early.e : early.v)
    if (early && req.$brainFailedReads?.temporary) return outcome(comeBack(req, rid, url))
    const seoF = sm.seo || (await load(route.view)).default.seo
    if (route.i18n) await load(route.i18n)
    const nonce = randomBytes(16).toString('base64'), n = h => h.replace(/<script(?![^>]*\bnonce=)/g, `<script nonce="${nonce}"`)
    globalThis.__brainLocale = route.locale || null
    let seo0 = null
    try {
      seo0 = typeof seoF === 'function' ? seoF({}) : null
    } finally {
      globalThis.__brainLocale = null
    }
    const initialHead = head(top, seo0, url.pathname, url.search, route, BASE.slice(0, -1))
    const enc = COMPRESS && req.method !== 'HEAD' ? acceptedEncodings(req.headers['accept-encoding'], DYNAMIC)[0] : null
    res.writeHead(200, {
      ...SECURITY, 'content-type': MIME['.html'], ...(req.method === 'HEAD' ? {} : { 'content-security-policy': withNonce(mf.csp || CSP0, nonce) }), 'cache-control': 'no-cache',
      ...(googleSignInPage(initialHead) ? { 'cross-origin-opener-policy': 'same-origin-allow-popups' } : {}),
      ...(mf.build ? { 'x-brain-build': mf.build } : {}), ...extra, ...ctx.session.headers(),
      ...(COMPRESS ? { vary: varyEncoding(extra.vary) } : {}), ...(enc ? { 'content-encoding': enc } : {}),
    })
    // each chunk is flushed through the encoder, so the head still reaches the browser before load() ends
    const zs = enc ? encodeStream(enc) : null
    if (zs) zs.pipe(res)
    const write = s => (zs ? (zs.write(s), zs.flush()) : res.write(s))
    const at = { path: url.pathname, params: params || {}, query: Object.fromEntries(url.searchParams) }
    write(n(initialHead))
    // QA #253: the folder's _loading.brain goes in the region with the first chunk (SPEC: shows while load runs),
    // between two marker comments; the last chunk starts with a script that removes them and what is between
    let clear = ''
    if (route.loading && !early) { // the loading view (#253) only when load() is still running
      try {
        const lr = { ...route, view: route.loading, layouts: [], server: null }
        const { body } = await (queue = queue.then(() => render(lr, {}, null, at), () => render(lr, {}, null, at)))
        write(n(`<!--brain:loading-->${body}<!--/brain:loading-->`))
        clear = `<script>{const s=document.currentScript;let x=0;for(const c of[...s.parentNode.childNodes]){if(c.data=='brain:loading')x=1;if(x)c.remove();if(c.data=='/brain:loading')x=0}s.remove()}</script>`
      } catch (e) {
        log('error', 'loading view failed (streamed)', { rid, path: url.pathname, ...trace(e) })
      }
    }
    const end = (body, props = {}, b = bottom) => (zs || res).end(n(clear + body + b.replace('<!--brain:props-->', () => jsonInHtml(props))))
    // QA #289: the page closes with the _error.brain template (its client mounts the error view with { message, rid }),
    // never with the route's own mount script, which would replace the error with the view rendered from {}. Without an
    // _error.brain the notice stays: the view's mount script is left out.
    const fail = async () => {
      const body = await failBody(route, url, rid)
      if (route.error && route.error.page && file(route.error.page)) {
        const eb = readFileSync(file(route.error.page), 'utf8').split('<!--brain:ssr-->')[1]
        if (eb !== undefined) return end(body, { message: 'Something went wrong.', rid }, eb)
      }
      end(body, {}, bottom.replace(/<script type="module">(?:(?!<\/script>)[\s\S])*?brain-view[\s\S]*?<\/script>/g, ''))
    }
    let props
    try {
      props = (await loadP) ?? {}
      if (props.redirect || props.notFound === true) throw props
    } catch (e) {
      // too late for a real redirect: the page says noindex so the stub never enters the index
      if (e && e.redirect) return end(`<meta name="robots" content="noindex"><script>location.replace(${jsonInHtml(withBase(linkTo(route, redirect(e.redirect, e.status ?? 303).redirect)))})</script>`)
      // a streamed page already sent its 200 head: the region gets the not-found notice and the page is noindex
      if (e && e.notFound === true) return end(`<meta name="robots" content="noindex"><p role="alert" class="brain-fallback">Page not found.</p>`, {}, bottom.replace(/<script type="module">(?:(?!<\/script>)[\s\S])*?brain-view[\s\S]*?<\/script>/g, ''))
      if (e && e.clientGone && res.destroyed) return log('info', 'client left during load', { rid, path: url.pathname })
      log('error', 'load failed (streamed)', { rid, path: url.pathname, status: 200, ...trace(e) }) // headers already out
      return fail()
    }
    if (req.$brainFailedReads) softFailure(req, rid, url, 200)
    try {
      const { body, failed } = await (queue = queue.then(() => render(route, props, null, at), () => render(route, props, null, at)))
      for (const f of failed || []) log('error', 'component failed during SSR', { rid, path: url.pathname, comp: f.comp, where: f.w, ...trace(f.e) })
      // the 200 is already out: a page rendered without its data at least stays out of the index
      const stale = req.$brainFailedReads?.temporary ? '<meta name="robots" content="noindex">' : ''
      const pm = params && Object.keys(params).length ? `<meta name="brain-params" content="${esc(JSON.stringify(params))}">` : ''
      end(stale + pm + body, props)
    } catch (e) {
      log('error', 'render failed (streamed)', { rid, path: url.pathname, status: 200, ...trace(e) })
      await fail()
    }
  }
  // the head is already out: the folder's _error.brain (props { message, rid }) renders inside the region, else a notice
  async function failBody(route, url, rid) {
    const notice = `<p role="alert" class="brain-fallback">Something went wrong. Reference: ${esc(rid)}</p>`
    if (!route.error) return notice
    try {
      const props = { message: 'Something went wrong.', rid }, at = { path: url.pathname, params: {}, query: {} }
      return (await (queue = queue.then(() => render(route.error, props, null, at), () => render(route.error, props, null, at)))).body
    } catch {
      return notice
    }
  }
  // deny → views/403.brain (manifest `forbidden`, props.reason) or the static 403.html, always status 403
  function forbidden(req, res, url, rid, reason) {
    res.setHeader('x-robots-tag', NOINDEX) // a members-only page never enters the index (§33)
    if (!wantsHtml(req)) return failPage(res, 403, rid)
    if (mf.forbidden) return ssr(req, res, mf.forbidden, {}, url, rid, null, 403, { reason })
    const f = file('403.html')
    if (f) return html(res, 403, readFileSync(f, 'utf8'))
    return failPage(res, 403, rid)
  }
  // a failing load or guard → the folder's _error.brain (route `error`, props { message, rid }), else the generic page
  function errorPage(req, res, route, params, url, rid, status = 500) {
    if (wantsHtml(req) && route.error) return ssr(req, res, route.error, params, url, rid, null, status, { message: 'Something went wrong.', rid })
    return failPage(res, status, rid)
  }
  async function render(route, props, forms, at = null) {
    const { install } = await load('_server/dom.js')
    const doc = install()
    const root = doc.createElement('div')
    const core = await load(mf.core)
    const mount = core.mount || core.m
    // The core's console fallback (`BRAIN_ERROR: …`) is for the browser: on the server an fx/drv error goes to this
    // render's failures (logged with rid and path by ssr()), once. A component boundary (brain-guard) collects its own.
    if (core._err && !hooked.has(core)) {
      hooked.add(core)
      core._err((e, x, w) => {
        if (w === 'render' || w === 'B0908') return // brain-guard reports these itself, with the component's name
        for (let o = x; o; o = o.nodeType ? o.parentNode : o.p) if (o.$g) return
        const into = globalThis.__brainSSR
        if (into) into.push({ e, w, comp: null })
        else log('error', 'component failed after SSR', { where: w, ...trace(e) }) // an async fx of a finished render
      })
    }
    const V = (await load(route.view)).default
    const L = []
    for (const l of route.layouts || []) L.push((await load(l)).default)
    let tree = () => V(props)
    for (let i = L.length - 1; i > 0; i--) {
      const inner = tree, C = L[i]
      tree = () => C({ children: inner })
    }
    if (route.i18n) await load(route.i18n) // the route's dictionary chunk: add(locale, {…}) before anything renders
    globalThis.__brainForms = forms // form() reads it while the view is built (renders are serialized)
    globalThis.__brainIds = (globalThis.__brainIds || 0) + 1 // each page numbers its instance ids from the start, as the client will
    globalThis.__brainLocale = route.locale || null // t(), fmt and locale() of this page (§37)
    globalThis.__brainAlternates = route.alternates || null // <LangSwitch> links, in the HTML before any JS
    globalThis.__brainRoute = at // route(): app path, params, query of this request
    globalThis.__brainBase = BASE !== '/' ? BASE : null // computed href/src get the base (brain.js attr)
    const failed = (globalThis.__brainSSR = []) // components that fell back (brain-guard, §42): logged by ssr()
    let stop
    // #285: with `adopt` in the manifest (codegen navigates with fc/nx), the HTML is stamped so islands resume on it
    const unstamp = mf.adopt && core._rt ? (await load('_server/brain-hydrate.js')).stamp(core._rt()) : null
    try {
      stop = L.length ? mount(L[0], root, { children: tree }) : mount(V, root, props)
    } finally {
      globalThis.__brainForms = null
      if (unstamp) unstamp()
    }
    await new Promise(r => setTimeout(r, 0))
    globalThis.__brainSSR = null
    fillValues(root)
    const body = localizeLinks(route, root.innerHTML)
    stop()
    // SEO head: the codegen emits `export const seo` / `head` in the view's server module; the client chunk's V.seo
    // (older builds) is the fallback. Still inside the page's locale: `<meta seo title={t('…')}>` is translated.
    let seo = null
    try {
      const sm = route.server ? await load(route.server) : null
      const seoF = (sm && sm.seo) || V.seo, headF = (sm && sm.head) || V.head
      seo = typeof seoF === 'function' ? seoF(props) : null
      if (seo && typeof headF === 'function') seo = { ...seo, $head: headF(props) }
    } catch {} finally {
      globalThis.__brainLocale = globalThis.__brainAlternates = globalThis.__brainRoute = globalThis.__brainBase = null
    }
    return { body, seo, failed }
  }

  // ---------- RPC (transport; the logic is callRpc, shared with the dev server) ----------
  async function rpc(req, res, hash, fn, url, rid) {
    const fail = (status, error, { $h, ...extra } = {}) => send(res, status, json({ error, ...extra }), { 'content-type': MIME['.json'], 'cache-control': 'no-store', ...$h })
    if (req.method !== 'POST') return fail(405, 'Use POST', { $h: { allow: 'POST' } })
    if (req.headers['x-brain-rpc'] !== '1') return fail(403, 'Missing X-Brain-RPC header')
    if (!sameOrigin(req)) return originRefused('B0741', req, rid, 'rpc'), fail(403, 'Cross-origin request', { brainCode: 'B0741' })
    const modPath = fnId(hash + '/' + fn) && Object.hasOwn(mf.rpc, hash) && mf.rpc[hash]
    if (!modPath) return fail(404, 'Unknown function')
    let f
    try {
      f = serverFn(await load(modPath), fn)
    } catch (e) {
      log('error', 'rpc module failed to load', { rid, fn: hash + '/' + fn, ...trace(e) })
      return fail(500, 'Internal error', { rid })
    }
    if (typeof f !== 'function') return fail(404, 'Unknown function') // a guessed name never gets a rate-limit key
    const ra = overLimit(ipOf(req) + ' ' + hash + '/' + fn, LIMIT) // before reading the body
    if (ra) return fail(429, 'Too many requests', { $h: { 'retry-after': String(ra) } })
    let raw
    try {
      raw = await body(req, MAX_BODY)
    } catch (e) {
      if (e.aborted) return
      return fail(413, tooBig(rid, url.pathname))
    }
    if (native) await native.prepare(req, res, rid)
    const ctx = context(req, res, url, { params: {} })
    const r = await callRpc({ fn: f, args: raw, ctx, timeout: TIMEOUT })
    if (r.cause) log('error', r.status === 504 ? 'rpc timeout' : 'rpc failed', { rid, fn: hash + '/' + fn, ms: TIMEOUT, ...trace(r.cause) })
    const b = r.status >= 500 ? { ...r.body, rid } : r.body
    send(res, r.status, json(b), { 'content-type': MIME['.json'], 'cache-control': 'no-store', ...(r.headers || {}), ...ctx.session.headers() })
  }
  // strict (form posts, no custom header possible): a request with neither Origin nor Sec-Fetch-Site is refused
  function sameOrigin(req, strict) {
    const host = req.headers['x-forwarded-host'] || req.headers.host
    const origin = req.headers.origin
    if (origin) {
      try {
        return new URL(origin).host === host
      } catch {
        return false
      }
    }
    const site = req.headers['sec-fetch-site']
    return site ? site === 'same-origin' : !strict
  }

  // ---------- form posted without JS (§38; transport, the logic is callForm, shared with the dev server) ----------
  async function formPost(req, res, route, params, url, rid) {
    const plain = (status, text, h = {}) => send(res, status, text, { 'content-type': MIME['.txt'], 'cache-control': 'no-store', ...h })
    if (!sameOrigin(req, true)) return originRefused('B0741', req, rid, 'form'), plain(403, 'Cross-origin form post', { 'x-brain-code': 'B0741' })
    if (!/^application\/x-www-form-urlencoded/i.test(req.headers['content-type'] || '')) return plain(415, 'Forms are sent as application/x-www-form-urlencoded')
    let raw
    try {
      raw = await body(req, MAX_BODY)
    } catch (e) {
      if (e.aborted) return
      return plain(413, tooBig(rid, url.pathname))
    }
    const id = new URLSearchParams(raw).get('_brain') || '', fid = fnId(id), modPath = fid && Object.hasOwn(mf.rpc, fid.hash) && mf.rpc[fid.hash]
    if (!modPath) return plain(400, 'Unknown form') // only `<hash>/<name>`: no rate-limit key for anything else
    if (native) await native.prepare(req, res, rid)
    const ctx = context(req, res, url, { params })
    let module, rules
    try {
      module = await load(modPath)
      rules = await load('_server/brain-form.js')
      if (route.i18n) await load(route.i18n) // the page's dictionary: validation messages in its language (§37)
    } catch (e) {
      log('error', 'form module failed to load', { rid, fn: id, ...trace(e) })
      return failPage(res, 500, rid)
    }
    const r = await callForm({ module, raw, ctx, rules, key: ipOf(req) + ' ' + fid.hash + '/' + fid.fn, limit: LIMIT, timeout: TIMEOUT, locale: route.locale })
    if (r.cause) log('error', 'form failed', { rid, fn: id, ...trace(r.cause) })
    if (r.status === 303) return send(res, 303, '', { location: withBase(r.location), 'cache-control': 'no-store', ...ctx.session.headers() })
    if (r.status === 422) return ssr(req, res, route, params, url, rid, r.state, 422)
    plain(r.status, r.body.error, r.headers)
  }

  // ---------- dynamic sitemap (§32) ----------
  // A dynamic view without paths() can export `sitemap()` (manifest route `sitemap: true`): it returns the params of its
  // pages, plus an optional `lastmod`, at request time. /sitemap.xml is the build's file with those URLs added (a sitemap
  // index gets one more child, /sitemap-dynamic.xml). A file holds at most 50 000 URLs (sitemaps.org): when the build's
  // URLs plus the dynamic ones pass that, /sitemap.xml becomes an index of /sitemap-static.xml (the build's file) and
  // /sitemap-dynamic.xml; dynamic URLs past 50 000 are dropped with a warning. Absolute URLs take the site origin from
  // the manifest (an older manifest: from the build's sitemap); a build without a sitemap serves none. The list is kept 5 min (one sitemap() run
  // at a time when it expires) and the response says max-age=300; a sitemap() that throws is logged and skipped.
  const DYN = routes.filter(r => r.sitemap && r.server)
  let dynList = null
  const fill = (pattern, p) => {
    let ok = true
    const out = pattern.replace(/([:*])(\w+)/g, (_, k, n) => {
      const v = p && p[n]
      if (v == null || v === '') return (ok = false), ''
      return k === '*' ? String(v).split('/').map(encodeURIComponent).join('/') : encodeURIComponent(String(v))
    })
    return ok ? out : null
  }
  const day = v => (v instanceof Date ? v.toISOString().slice(0, 10) : typeof v === 'string' && /^\d{4}-\d{2}-\d{2}/.test(v) ? v : null)
  const MAX_URLS = 50000
  let dynRun = null
  const dynGood = new Map()
  async function collect(origin) {
    const urls = []
    let dropped = 0
    // Views answer at the same time, so one slow view costs one timeout; a view that fails keeps its last good list.
    const lists = await Promise.all(DYN.map(async r => {
      try {
        const sm = await load(r.server)
        const list = typeof sm.sitemap === 'function' ? await withTimeout(Promise.resolve().then(() => sm.sitemap({ env })), TIMEOUT) : []
        dynGood.set(r, Array.isArray(list) ? list : [])
      } catch (e) {
        log('error', 'sitemap() failed', { view: r.view, ...trace(e) })
      }
      return dynGood.get(r) || []
    }))
    DYN.forEach((r, n) => {
        for (const p of lists[n]) {
          const path = fill(r.path, p)
          if (!path) continue
          const loc = origin + withBase(path)
          if (loc.length > 2048) log('warn', 'sitemap(): URL longer than 2048 characters left out; fix: shorten the slug', { view: r.view, loc: loc.slice(0, 120) })
          else if (urls.length < MAX_URLS) urls.push({ loc, lastmod: day(p.lastmod) })
          else dropped++
        }
    })
    if (dropped) log('warn', 'sitemap(): more than 50000 URLs, the rest are not listed', { dropped })
    if (!urls.length) log('warn', 'sitemap(): no view returned a URL, so /sitemap.xml lists only the pages of the build', { views: DYN.map(r => r.view) })
    dynList = { at: Date.now(), urls }
    return urls
  }
  async function dynamicUrls(origin) {
    if (dynList && Date.now() - dynList.at < 300000) return dynList.urls
    // concurrent requests after expiry share one run of every sitemap(); a crawler never waits while a list exists
    if (!dynRun) dynRun = collect(origin).finally(() => (dynRun = null))
    return dynList ? dynList.urls : dynRun
  }
  // §33 S01: every sitemap file holds at most 50 000 URLs and SITEMAP_BYTES bytes of UTF-8 XML; the dynamic URLs fill
  // /sitemap-dynamic.xml, /sitemap-dynamic-2.xml, … in sitemap() order, so the split is deterministic
  const XML_SIGNATURE = '<!-- by BRAINLLM https://chrisifm.github.io/BRAINLLM/ -->'
  const XML_HEADER = `<?xml version="1.0" encoding="UTF-8"?>\n${XML_SIGNATURE}\n`
  const SET_HEAD = XML_HEADER + '<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">\n'
  const SET_FOOT = '</urlset>\n'
  const bytes = t => Buffer.byteLength(t)
  function pages(urls) {
    const out = [[]]
    let size = bytes(SET_HEAD + SET_FOOT)
    for (const u of urls) {
      const line = `  <url><loc>${esc(u.loc)}</loc>${u.lastmod ? `<lastmod>${esc(u.lastmod)}</lastmod>` : ''}</url>\n`
      const n = bytes(line)
      if (out.at(-1).length && (out.at(-1).length >= MAX_URLS || size + n > SITEMAP_BYTES)) out.push([]), (size = bytes(SET_HEAD + SET_FOOT))
      out.at(-1).push(line)
      size += n
    }
    return out.map(p => p.join(''))
  }
  async function serveSitemap(req, res, url) {
    const f = file('/sitemap.xml')
    if (!f) return false
    const original = readFileSync(f, 'utf8')
    const declaration = original.startsWith('<?xml') ? original.indexOf('?>') + 2 : 0
    const text = original.includes(XML_SIGNATURE) ? original : declaration > 1 ? original.slice(0, declaration) + '\n' + XML_SIGNATURE + original.slice(declaration) : XML_SIGNATURE + '\n' + original
    const origin = mf.site || [...text.matchAll(/<loc>(https?:\/\/[^<]+)<\/loc>/g)].map(m => URL.canParse(m[1]) && new URL(m[1]).origin).find(Boolean)
    if (!origin) return false
    const index = text.includes('<sitemapindex')
    const sets = pages(await dynamicUrls(origin))
    // a urlset that would pass either limit with the dynamic URLs is split: index → static file + dynamic file(s)
    const split = !index && (sets.length > 1 || (text.match(/<url>/g) || []).length + (sets[0].match(/<url>/g) || []).length > MAX_URLS || bytes(text) + bytes(sets[0]) > SITEMAP_BYTES)
    const xml = body => send(res, 200, req.method === 'HEAD' ? '' : body, { 'content-type': MIME['.xml'], 'cache-control': 'public, max-age=300' })
    if (url.pathname === '/sitemap-static.xml') return split ? (xml(text), true) : false
    const name = i => (i ? `/sitemap-dynamic-${i + 1}.xml` : '/sitemap-dynamic.xml')
    const child = p => `  <sitemap><loc>${esc(origin + withBase(p))}</loc></sitemap>\n`
    const kids = sets.map((_, i) => child(name(i))).join('')
    if (url.pathname !== '/sitemap.xml') {
      const i = sets.findIndex((_, i) => name(i) === url.pathname)
      return i >= 0 && (index || split) ? (xml(SET_HEAD + sets[i] + SET_FOOT), true) : false
    }
    xml(
      index
        ? text.replace('</sitemapindex>', `${kids}</sitemapindex>`)
        : split
          ? `${XML_HEADER}<sitemapindex xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">\n${child('/sitemap-static.xml')}${kids}</sitemapindex>\n`
          : text.replace('</urlset>', sets[0] + '</urlset>'),
    )
    return true
  }

  // ---------- static ----------
  // Content validators use bounded buffers and a bounded metadata cache; a full media file never enters the heap.
  const fileTags = new Map(), hashingFiles = new Map()
  async function fileTag(handle, stat) {
    const key = `${stat.dev}:${stat.ino}:${stat.size}:${stat.mtimeMs}:${stat.ctimeMs}`
    if (fileTags.has(key)) return fileTags.get(key)
    if (hashingFiles.has(key)) return hashingFiles.get(key)
    if (hashingFiles.size >= 8) throw new HttpError(503, 'Static file validators are busy; retry shortly')
    const task = (async () => {
      const hash = createHash('sha256'), buf = Buffer.alloc(65536), until = performance.now() + 30000
      let offset = 0
      while (offset < stat.size) {
        if (performance.now() > until) throw new HttpError(503, 'Static file validator deadline exceeded; retry shortly')
        const { bytesRead } = await handle.read(buf, 0, Math.min(buf.length, stat.size - offset), offset)
        if (!bytesRead) throw new Error('Static file changed while computing its validator')
        hash.update(buf.subarray(0, bytesRead)); offset += bytesRead
      }
      const after = await handle.stat()
      if (after.size !== stat.size || after.mtimeMs !== stat.mtimeMs || after.ctimeMs !== stat.ctimeMs) throw new HttpError(503, 'Static file changed; retry shortly')
      const etag = '"' + hash.digest('base64url') + '"'
      fileTags.set(key, etag)
      if (fileTags.size > 256) fileTags.delete(fileTags.keys().next().value)
      return etag
    })()
    hashingFiles.set(key, task)
    try { return await task } finally { hashingFiles.delete(key) }
  }
  async function streamFile(req, res, handle, status, headers, span) {
    if (req.method === 'HEAD') return send(res, status, '', headers)
    res.writeHead(status, { ...SECURITY, ...(server.draining ? { connection: 'close' } : {}), ...headers })
    // The descriptor stays pinned from stat and hashing through transmission, including atomic deployments.
    const length = Number(headers['content-length'])
    if (!length) return res.end()
    await pipeline(handle.createReadStream({ start: 0, end: length - 1, ...span, autoClose: false }), res)
  }
  // Never served (brain-pack private.rs holds the same list for the Rust handler and every generated host config).
  const PRIVATE_PREFIXES = ['_server', '_brain', 'server.mjs']
  const PRIVATE_FILES = ['_headers', '_redirects']
  // §33 content negotiation: `Accept` preferring text/markdown gets the page's Markdown twin (`/pricing` → pricing.md,
  // `/` → index.md) when the build published one (`seo.markdown`, default true; never for noindex pages).
  function publicStaticPath(pathname) {
    let path
    try { path = decodeURIComponent(pathname) } catch { return null }
    if (/[\\\x00]/.test(path)) return null
    const clean = normalize(path).replaceAll('\\', '/')
    const rel = clean.replace(/^\/+/, '').toLowerCase() // a case-insensitive file system opens _SERVER/ as _server/
    if (PRIVATE_PREFIXES.some(p => rel.startsWith(p)) || PRIVATE_FILES.includes(rel) || clean.split('/').some(part => part.startsWith('.') && part.toLowerCase() !== '.well-known')) return null
    return path
  }
  function serveTwin(req, res, url) {
    if (!prefersMarkdown(req.headers.accept)) return false
    const path = publicStaticPath(url.pathname)
    if (path === null) return false
    const md = file(mdPath(path))
    if (!md) return false
    send(res, 200, req.method === 'HEAD' ? '' : readFileSync(md), { 'content-type': MIME['.md'], 'cache-control': 'public, max-age=3600', vary: 'accept', ...twinCanonical(path) })
    return true
  }
  // A Markdown copy repeats its page: the header names the canonical the page itself declares. A page without one (no
  // site configured) or with a per-request pattern gets no header, so this never invents a URL.
  function twinCanonical(path) {
    const stem = path.replace(/\.md$/, '').replace(/\/+$/, '')
    const page = file(stem ? stem + '.html' : 'index.html') || file(stem + '/index.html') || file(stem ? stem + '.brain' : 'index.brain') || file(stem + '/index.brain')
    if (!page) return {}
    const tag = /<link\b[^>]*\brel=["']canonical["'][^>]*>/i.exec(readFileSync(page, 'utf8'))?.[0]
    const href = tag && /\bhref="(https?:\/\/[^"<>\s]+)"/.exec(tag)?.[1].replaceAll('&amp;', '&')
    return href && URL.canParse(href) && !/\/[:*]/.test(new URL(href).pathname) ? { link: `<${href}>; rel="canonical"` } : {}
  }
  // What exists whatever the routes say: generated sitemaps, built files and the icon browsers ask for by habit.
  async function serveFixed(req, res, url) {
    if (DYN.length && /^\/sitemap(-static|-dynamic(-\d+)?)?\.xml$/.test(url.pathname) && (await serveSitemap(req, res, url))) return true
    if (await serveStatic(req, res, url)) return true
    if (url.pathname !== '/favicon.ico' || !file('favicon.svg')) return false
    send(res, 301, '', { location: withBase('/favicon.svg'), 'cache-control': 'public, max-age=86400' })
    return true
  }

  async function serveStatic(req, res, url) {
    const p = publicStaticPath(url.pathname)
    if (p === null) return false
    const candidates = p.endsWith('/') ? [p + 'index.html', p + 'index.brain', ...(p.length > 1 ? [p.slice(0, -1) + '.html', p.slice(0, -1) + '.brain'] : [])] : [p, p + '.html', p + '/index.html', p + '.brain', p + '/index.brain']
    for (const c of candidates) {
      const f = file(c)
      if (!f) continue
      const ext = extname(f), type = FEEDS[basename(f)] || MIME[ext] || 'application/octet-stream'
      if (ext === '.html' || ext === '.brain') {
        // §33: the page's Markdown twin (`/pricing` → pricing.md) when the client prefers text/markdown
        if (serveTwin(req, res, url)) return true
        const md = file(mdPath(p))
        const page = readFileSync(f, 'utf8')
        // static HTML changes only with a deploy: its ETag is the build signature (content hash without a manifest)
        return html(res, 200, page, md ? { vary: 'accept', 'x-brain-static': '1' } : { 'x-brain-static': '1' }, '"' + (mf.build || tag(page)) + '"'), true
      }
      const headers = {
        ...(ext === '.md' ? twinCanonical(p) : {}),
        'content-type': type,
        // the worker script must be revalidated on every check, or a CDN could keep an old worker for an hour (§35.7)
        'cache-control': p.startsWith('/_b/') ? 'public, max-age=31536000, immutable' : p === '/sw.js' || /\.(m3u8?|mpd)$/i.test(p) ? 'no-cache' : 'public, max-age=3600',
        vary: 'accept-encoding',
      }
      // the smallest precompressed sibling the client accepts (the build keeps only those smaller than the file)
      let best = null
      for (const enc of req.headers.range ? [] : acceptedEncodings(req.headers['accept-encoding'])) {
        const encoded = file(c + (enc === 'br' ? '.br' : enc === 'zstd' ? '.zst' : '.gz'))
        const size = encoded ? statSync(encoded).size : Infinity
        if (encoded && (!best || size < best.size)) best = { enc, encoded, size }
      }
      const selected = best ? best.encoded : f
      const handle = await open(selected, FS.O_RDONLY | (FS.O_NOFOLLOW || 0) | (FS.O_NONBLOCK || 0))
      try {
        const stat = await handle.stat()
        if (!stat.isFile()) throw new Error('Static output must be a regular file')
        headers.etag = await fileTag(handle, stat)
        if (res.destroyed) return true
        if (!best) headers['accept-ranges'] = 'bytes'
        const inm = req.headers['if-none-match']
        if (inm && inm.split(',').some(t => t.trim() === '*' || t.trim().replace(/^W\//, '') === headers.etag)) return send(res, 304, '', headers), true
        const span = req.headers.range && (!req.headers['if-range'] || req.headers['if-range'] === headers.etag) ? byteRange(req.headers.range, stat.size) : null
        if (span === false) return send(res, 416, '', { ...headers, 'content-range': `bytes */${stat.size}` }), true
        if (span) {
          headers['content-range'] = `bytes ${span.start}-${span.end}/${stat.size}`
          headers['content-length'] = String(span.end - span.start + 1)
        } else {
          headers['content-length'] = String(stat.size)
          if (best) headers['content-encoding'] = best.enc
        }
        await streamFile(req, res, handle, span ? 206 : 200, headers, span)
      } finally {
        await handle.close()
      }
      return true
    }
    return false
  }

  // /health is reserved for the backend (B0735), so an app cannot declare it and deploy gates ask this port for it. An app
  // page at that path still wins: this only runs where the answer would have been a 404. Nothing here names the backend.
  const isProbe = path => path === '/health' || path === '/health/live' || path === '/health/ready'
  // /_brain/health[/live|/ready] is the same probe under the framework prefix; the backend only serves /health, so it is
  // answered here (mapped to that route) instead of being forwarded with the rest of /_brain and coming back a 404.
  const brainProbe = path => path.startsWith('/_brain/health') && isProbe(path.slice(7)) ? path.slice(7) : null
  const answerProbe = async (req, res, url, rid) => {
    const headers = { 'content-type': MIME['.json'], 'cache-control': 'no-store' }
    if (req.method !== 'GET' && req.method !== 'HEAD') return notFound(req, res)
    if (!native) return send(res, 200, json({ ok: true }), headers)
    try { await native.proxy(req, res, new URL(url.pathname, url), rid, send) }
    catch (e) {
      if (!res.headersSent) {
        log('warn', 'health probe could not reach the native backend', { rid, path: url.pathname, status: 503, ...trace(e) })
        send(res, 503, json({ ok: false }), headers)
      }
    }
  }

  const urlVariant = req => {
    if (req.method !== 'GET' && req.method !== 'HEAD') return null
    const q = req.url.indexOf('?'), raw = q < 0 ? req.url : req.url.slice(0, q)
    if (!raw.includes('//') && !raw.endsWith('/index.html')) return null
    let path = raw.replace(/\/{2,}/g, '/')
    const app = '/' + (path.startsWith(BASE) ? path.slice(BASE.length) : path.slice(1))
    if (/^\/(?:api|_brain|_r|s)(?:\/|$)/.test(app) && !raw.startsWith('//')) return null
    if (path.endsWith('/index.html')) path = path.slice(0, -'index.html'.length)
    if (path !== BASE && path !== '/' && (path.endsWith('/') && SLASH !== 'always')) path = path.replace(/\/+$/, '')
    const location = path + (q < 0 ? '' : req.url.slice(q))
    try { safeLocation(location) } catch { return null }
    return location
  }
  // `<meta seo query="page">`: `page` is a number from 1 and page 1 is the URL without it. A view decides past its last
  // page with notFound(). → null (nothing to do) · 404 · the URL to redirect to.
  const pagedRoutes = new Map()
  const pageParameter = (route, url) => {
    if (!url.searchParams.has('page') || !route.page) return null
    if (!pagedRoutes.has(route.page)) {
      const keys = /<meta name="brain-query" content="([^"]*)">/.exec(readFileSync(file(route.page), 'utf8'))?.[1]
      pagedRoutes.set(route.page, !!keys && keys.split(/[\s,]+/).includes('page'))
    }
    if (!pagedRoutes.get(route.page)) return null
    const values = url.searchParams.getAll('page')
    if (values.length > 1 || !/^[1-9]\d{0,8}$/.test(values[0])) return 404
    if (values[0] !== '1') return null
    const rest = new URLSearchParams(url.search)
    rest.delete('page')
    return url.pathname + (rest.size ? '?' + rest : '')
  }

  // ---------- dispatch ----------
  const server = http(async (req, res) => {
    // Only a file upload may outlast the 30-second body budget; the dispatcher lifts this guard for that path alone.
    if (native) {
      req.$brainBodyGuard = setTimeout(() => {
        if (req.complete) return
        if (!res.headersSent) res.writeHead(408, { connection:'close' })
        res.end(); req.socket?.destroy()
      }, 30000)
      req.$brainBodyGuard.unref?.()
      res.once('close', () => clearTimeout(req.$brainBodyGuard))
    }
    const rid = req.headers['x-request-id'] && /^[\w-]{1,64}$/.test(req.headers['x-request-id']) ? req.headers['x-request-id'] : randomUUID()
    res.setHeader('x-request-id', rid)
    const live = { logged: false }
    liveRequests.set(rid, live)
    res.once('close', () => {
      if (liveRequests.get(rid) === live) liveRequests.delete(rid)
      // The net under every path: a 5xx that was answered always leaves a line with its request id.
      if (res.statusCode >= 500 && !live.logged && !res.$brainMuted) log(res.$brainCause === STATUS_PAGE ? 'info' : 'error', 'server error answered', { rid, path: req.$brainPath || String(req.url || '').split('?')[0], status: res.statusCode, cause: res.$brainCause || 'no cause recorded on this path', ...(res.$brainCause === STATUS_PAGE ? {} : { fix: 'Search this rid in the native backend log; its failed requests carry the same id' }) })
    })
    const t0 = performance.now()
    res.on('finish', () => {
      const ms = performance.now() - t0
      if (ms > 1000) log('warn', 'slow request', { rid, path: req.$brainPath || String(req.url || '').split('?')[0], status: res.statusCode, ms: Math.round(ms) })
    })
    try {
      // One URL per page: `//route`, `/a//b` and `/route/index.html` answer one 301 on this origin. The path is rewritten
      // as text, before URL parsing: `//host/x` would be read as another host. Backend paths are left to the backend.
      const variant = urlVariant(req)
      if (variant) return send(res, 301, '', { location: variant, 'cache-control': 'public, max-age=3600' })
      if (req.url.startsWith('//')) return notFound(req, res)
      const host = req.headers.host || 'localhost'
      let full
      try { full = new URL(req.url, 'http://' + host) } catch { return send(res, 400, 'Bad Request') }
      // /_health stays at the root: the supervisor calls it there whatever the base
      // refused API sources, for an operator on this machine only (a loopback socket and no forwarded hop)
      if (full.pathname === '/_access/sources' && req.method === 'GET') {
        if (!LOOPBACK(plainIp(req.socket.remoteAddress ?? '') || '') || req.headers['x-forwarded-for']) return notFound(req, res)
        return send(res, 200, json({ sources: [...refusedSources.values()].sort((a, b) => b.count - a.count) }), { 'content-type': MIME['.json'], 'cache-control': 'no-store' })
      }
      if (full.pathname === '/_health' || full.pathname === BASE + '_health') return send(res, 200, json({ ok: true, build: mf.build || null }), { 'content-type': MIME['.json'], 'cache-control': 'no-store' })
      let url = full
      if (BASE !== '/') {
        if (full.pathname === BASE.slice(0, -1)) return send(res, 301, '', { location: BASE + full.search })
        if (!full.pathname.startsWith(BASE)) {
          const probe = isProbe(full.pathname) ? full.pathname : brainProbe(full.pathname)
          return probe ? await answerProbe(req, res, new URL(probe, full), rid) : notFound(req, res)
        }
        url = new URL('/' + full.pathname.slice(BASE.length) + full.search, full) // the app path
      }
      req.$brainPath = url.pathname
      const probe = brainProbe(url.pathname)
      if (probe) return await answerProbe(req, res, new URL(probe, url), rid)
      if (native && /^\/_brain\/files(?:\/|$)/.test(url.pathname)) return await native.fileStream(req,res,url,rid)
      if (native && /^\/(?:api(?:\/|$)|_brain(?:\/|$)|s\/)/.test(url.pathname)) {
        const client = ipOf(req), refused = accessRefusal(req, url.pathname, client, access)
        if (refused) {
          accessRefused(req, rid, url.pathname, client, refused)
          return send(res, 403, json({ error: 'This API answers its own site only', code: 'B0752' }), { 'content-type': MIME['.json'], 'cache-control': 'no-store', 'x-brain-code': 'B0752' })
        }
        if (overBudget(req, res, url.pathname, client)) return
        return await native.proxy(req,res,url,rid,send)
      }
      const declaredRoute = native && nativeRoutes.find(route => route.rx.test(url.pathname))
      if (declaredRoute) {
        if (overBudget(req, res, url.pathname, ipOf(req))) return
        return await native.proxy(req,res,url,rid,send,undefined,declaredRoute)
      }
      // redirects.json (manifest `redirects`, brain-seo server_json): exact sources first, then patterns, on the app path;
      // same-site targets get the base, the query is kept when the target has none; slash policy and host in the same hop
      const to = (req.method === 'GET' || req.method === 'HEAD') && canonicalOf(req, url)
      if (to) return send(res, to[1], '', { location: to[0], 'cache-control': to[1] === 301 || to[1] === 308 ? 'public, max-age=3600' : 'no-store' }, to[2])
      const r = /^\/_r\/([\w-]+)\/([\w$]+)$/.exec(url.pathname)
      if (r) return await rpc(req, res, r[1], r[2], url, rid)
      if (url.pathname.startsWith('/_r/')) return send(res, 404, json({ error: 'Unknown function' }), { 'content-type': MIME['.json'] })
      if (url.pathname.startsWith('/_server/')) return notFound(req, res) // server code is never served
      const post = req.method === 'POST'
      if (!post && req.method !== 'GET' && req.method !== 'HEAD') return send(res, 405, 'Method not allowed', { allow: 'GET, HEAD, POST' })
      // a status page opened directly answers its own status: `/403` is never a 200 that a crawler or a probe can trust
      const reserved = post ? 0 : reservedStatus(publicStaticPath(url.pathname) ?? url.pathname)
      if (reserved === 403) return forbidden(req, res, url, rid, '')
      if (reserved) {
        // Anyone can open it, so the note about it is bounded and is not an error.
        const minute = Math.floor(Date.now() / 60000)
        if (statusPageWindow !== minute) { statusPageWindow = minute; statusPageLogs = 0 }
        res.$brainCause = STATUS_PAGE
        res.$brainMuted = statusPageLogs++ >= 16
        return failPage(res, reserved, rid)
      }
      for (const route of routes) {
        const m = route.rx.exec(url.pathname)
        if (!m) continue
        const protectedRoute = route.guards?.length || route.middleware?.length
        // a form page without load is prerendered: GET serves that file (build ETag); only its POST needs the server
        if (!post && !protectedRoute && route.forms && (await serveStatic(req, res, url))) return
        // a parameter never hides a real file: `/[code]` must not swallow /favicon.svg, /sitemap.xml or a prerendered /login
        if (!post && !protectedRoute && m.groups && (await serveFixed(req, res, url))) return
        if (!post && !protectedRoute && serveTwin(req, res, url)) return
        const paged = post ? null : pageParameter(route, url)
        if (paged === 404) return notFound(req, res)
        if (paged) return send(res, 301, '', { location: withBase(paged), 'cache-control': 'public, max-age=3600' })
        if (overBudget(req, res, url.pathname, ipOf(req))) return
        return await (post ? formPost : ssr)(req, res, route, m.groups ? { ...m.groups } : {}, url, rid)
      }
      if (post) return send(res, 405, 'Method not allowed', { allow: 'GET, HEAD' })
      if (await serveFixed(req, res, url)) return
      if (isProbe(url.pathname)) return await answerProbe(req, res, url, rid)
      notFound(req, res)
    } catch (e) {
      const busy = e instanceof HttpError && e.code === 'BRIDGE_BUSY'
      if (busy) log('warn', 'backend bridge busy', { rid, path: req.$brainPath || String(req.url || '').split('?')[0], fix: e.fix })
      else if (e instanceof HttpError && OWN_CAUSES.has(e.message)) log('warn', 'backend unavailable', { rid, path: req.$brainPath || String(req.url || '').split('?')[0], status: e.status, cause: e.message, ...(CODE_SHAPE.test(e.transport || '') ? { transport: e.transport } : {}) })
      else log('error', 'unhandled', { rid, path: req.url, ...trace(e) })
      try {
        if (busy && !res.headersSent) { res.setHeader('retry-after', String(e.retryAfter)); failPage(res, 503, rid, { code: e.code, message: 'Service Unavailable', fix: e.fix }) }
        else if (!res.headersSent) failPage(res, intendedStatus(e) || 500, rid)
        else res.end()
      } catch (again) {
        // the error page itself failed: still an answer, never an unhandled rejection that takes the process down
        log('error', 'error page failed', { rid, path: req.url, ...trace(again) })
        if (!res.headersSent) res.writeHead(500, SECURITY)
        res.end()
      }
    }
  })
  if (native) server.once('close', () => native.close())
  if (native && nativeSockets) server.$brainSocketStop = installNativeSockets(server,{target:o.backendURL ?? env.BRAIN_BACKEND_URL,base:BASE,cookie:native._socketCookie,client:native._forwardedFor,log})
  server.headersTimeout = 15000
  // With a native backend the file upload deadline is enforced per request (above); 30 s stays for every other body.
  server.requestTimeout = native ? 320000 : 30000
  // `warm: true` (`node server.mjs`): import the app's modules now; requests that arrive meanwhile share the same imports
  if (o.warm) server.$brainWarm = warm()
  return server
}

// ---------- session (signed cookie; shared with the dev server) ----------
const STATUS_PAGE = 'status page opened directly'
const COOKIE = '__Host-session', SECRET_COOKIE = '__Host-session-secret', SECRET_MAX = 3500, DEFAULT_SESSION_SECONDS = 7 * 24 * 3600
// Keys that still verify and decrypt during a rotation; new cookies always use the current key.
const previousKeys = () => (process.env.BRAIN_SECRET_PREVIOUS || '').split(',').map(k => k.trim()).filter(Boolean)
const sessionVersion = () => process.env.BRAIN_SESSION_VERSION || ''
const sessionSeconds = () => { const n = Number(process.env.BRAIN_SESSION_SECONDS); return Number.isInteger(n) && n > 0 ? n : DEFAULT_SESSION_SECONDS }
// Cookies of this process signed out before their expiry (mac -> exp). One process only: a shared store does not exist
// here, so a deployment with several processes signs everybody out with BRAIN_SESSION_VERSION instead.
const REVOKED = new Map(), REVOKED_MAX = 10000
const revoke = (mac, exp) => {
  const now = Date.now()
  for (const [k, e] of REVOKED) if (e <= now) REVOKED.delete(k)
  while (REVOKED.size >= REVOKED_MAX) REVOKED.delete(REVOKED.keys().next().value)
  REVOKED.set(mac, exp)
}
const secretKey = secret => Buffer.from(hkdfSync('sha256', secret, '', 'brain session secret v1', 32))
const seal = (secret, text) => {
  const iv = randomBytes(12), cipher = createCipheriv('aes-256-gcm', secretKey(secret), iv)
  cipher.setAAD(Buffer.from(SECRET_COOKIE))
  const body = Buffer.concat([cipher.update(text, 'utf8'), cipher.final()])
  return Buffer.concat([iv, cipher.getAuthTag(), body]).toString('base64url')
}
const unseal = (secret, value) => {
  try {
    const raw = Buffer.from(value, 'base64url')
    if (raw.length < 29) return null
    const decipher = createDecipheriv('aes-256-gcm', secretKey(secret), raw.subarray(0, 12))
    decipher.setAAD(Buffer.from(SECRET_COOKIE))
    decipher.setAuthTag(raw.subarray(12, 28))
    return Buffer.concat([decipher.update(raw.subarray(28)), decipher.final()]).toString('utf8')
  } catch { return null }
}
// `ctx.session.setSecret(value, { maxAge })`, `.secret()` and `.clearSecret()` on any session handle (signed cookie or
// native bridge): server-only values such as a provider token, AES-256-GCM encrypted in their own cookie. They never
// fill `ctx.user`, and `session.clear()` removes them too.
const missingSecret = what => new Error(`B0782: BRAIN_SECRET is not set: ${what} need it; fix: set 32+ random bytes in the host environment or in .brain_env (openssl rand -hex 32)`)
export function sessionSecrets(session, cookies, secret) {
  const out = []
  const write = (value, maxAge) => { out.length = 0; out.push(`${SECRET_COOKIE}=${value}; Path=/; HttpOnly; Secure; SameSite=Lax; Max-Age=${Math.max(0, maxAge)}`) }
  let current
  const read = () => {
    if (current !== undefined) return current
    current = null
    const v = cookies && cookies[SECRET_COOKIE]
    if (!v || !secret) return current
    for (const key of [secret, ...previousKeys()]) {
      const text = unseal(key, v)
      if (text === null) continue
      try {
        const s = JSON.parse(text)
        if (s && s.exp > Date.now()) current = s.v
      } catch {}
      break
    }
    return current
  }
  const { clear, headers } = session
  const clearSecret = () => { write('', 0); current = null }
  return Object.assign(session, {
    secret: read,
    setSecret(value, { maxAge = sessionSeconds() } = {}) {
      if (!secret) throw missingSecret('session secrets')
      const text = JSON.stringify({ v: value, exp: Date.now() + maxAge * 1000 })
      if (value === undefined || text === undefined || JSON.parse(text).v === undefined) throw new Error('session.setSecret needs a JSON value')
      const sealed = seal(secret, text)
      if (sealed.length > SECRET_MAX) throw new Error(`session secret too large (${sealed.length} bytes, limit ${SECRET_MAX}): keep it server-side and store a reference`)
      write(sealed, maxAge)
      current = value
    },
    clearSecret,
    clear(...args) { const result = clear.apply(session, args); if (out.length || (cookies && cookies[SECRET_COOKIE] !== undefined)) clearSecret(); return result },
    headers() {
      const base = headers.call(session)
      if (!out.length) return base
      const own = base['set-cookie']
      return { ...base, 'set-cookie': [...(Array.isArray(own) ? own : own ? [own] : []), ...out] }
    },
  })
}
// The user of a request: the signed `__Host-session` cookie (HMAC-SHA256 with BRAIN_SECRET; BRAIN_SECRET_PREVIOUS still
// verifies during a rotation, BRAIN_SESSION_VERSION signs everybody out), plus the session handle to set/clear it. Same
// in `node server.mjs` and in `brain run` (the dev sidecar imports it).
export function sessionFrom(cookies, secret) {
  const signWith = (key, data) => b64u(createHmac('sha256', key).update(data).digest())
  const sign = data => signWith(secret, data)
  let cur = null, curMac = null
  const v = cookies && cookies[COOKIE]
  if (v && secret) {
    const i = v.lastIndexOf('.')
    if (i > 0) {
      const data = v.slice(0, i), mac = Buffer.from(v.slice(i + 1))
      for (const key of [secret, ...previousKeys()]) {
        const good = Buffer.from(signWith(key, data))
        if (mac.length === good.length && timingSafeEqual(mac, good)) {
          try {
            const s = JSON.parse(Buffer.from(data, 'base64url').toString())
            if (s && s.exp > Date.now() && (s.sv ?? '') === sessionVersion() && !REVOKED.has(v.slice(i + 1))) { cur = s; curMac = v.slice(i + 1) }
          } catch {}
          break
        }
      }
    }
  }
  const set = []
  let cleared = false
  const cookie = (value, maxAge) => `${COOKIE}=${value}; Path=/; HttpOnly; Secure; SameSite=Lax; Max-Age=${maxAge}`
  const user = cur ? cur.u : null
  return {
    user,
    session: sessionSecrets({
      user,
      set(u, { maxAge = sessionSeconds() } = {}) {
        if (!secret) throw missingSecret('sessions')
        const version = sessionVersion()
        const data = b64u(JSON.stringify({ u, exp: Date.now() + maxAge * 1000, ...(version ? { sv: version } : {}) }))
        set.push(cookie(`${data}.${sign(data)}`, maxAge))
      },
      clear() {
        if (curMac) revoke(curMac, cur.exp)
        set.push(cookie('', 0))
        cleared = true
      },
      // logout wipes this origin's storage on the device: localStorage, IndexedDB, the Cache API and service workers
      // (§35: the next user sees nothing). Not "cache": Chrome clears the whole HTTP cache before it finishes the
      // response, so the logout call took ~18 s (F16). Nothing private is in that cache: RPC is no-store and SSR
      // HTML rendered with a session cookie is `private, no-store` (ssr() → keep).
      headers: () => ({ ...(set.length ? { 'set-cookie': set } : {}), ...(cleared ? { 'clear-site-data': '"storage"' } : {}) }),
    }, cookies, secret),
  }
}

// ---------- guards and middleware (§22; shared with the dev server) ----------
// `_guard.jsbrain`: (ctx) => true | false | redirect('/login') | deny('reason'), returned or thrown.
// `_middleware.jsbrain`: async (ctx, next) => …; may set ctx.headers (response headers) or stop with redirect/deny.
// The same shapes come out of brain-router's redirect()/deny() in the browser.
export const deny = (reason = 'Forbidden') => ({ deny: String(reason) })
const samePath = u => { try { safeLocation(u); return true } catch { return false } }
function verdict(v) {
  if (v === false) return { deny: 'Forbidden' }
  if (typeof v === 'string') v = { redirect: v }
  if (!v || typeof v !== 'object') return null
  if (v.redirect !== undefined) {
    if (!samePath(v.redirect)) throw new HttpError(500, 'redirect to another origin is not allowed: ' + v.redirect) // §31 open redirect
    return redirect(v.redirect, v.status ?? 303)
  }
  return v.deny !== undefined ? { deny: String(v.deny) } : null
}
// Runs the route's guards (outer folder first), then its middleware as an onion (`next()` runs the rest).
// → { ok: true, headers } | { redirect, status } | { deny: reason }. Any other error propagates (→ 500 / _error).
export async function runChain({ guards = [], middleware = [], ctx }) {
  const headers = (ctx.headers = ctx.headers || {})
  for (const g of guards) {
    const f = (g && g.default) || g
    let r
    try {
      r = verdict(await f(ctx))
    } catch (e) {
      if (!(r = verdict(e))) throw e
    }
    if (r) return r
  }
  let i = 0, stop = null
  const next = async () => {
    const m = middleware[i++]
    if (!m || stop) return
    try {
      const r = verdict(await ((m && m.default) || m)(ctx, next))
      if (r) stop = stop || r
    } catch (e) {
      const r = verdict(e)
      if (!r) throw e
      stop = stop || r
    }
  }
  await next()
  return stop || { ok: true, headers }
}

// ---------- shared request logic (server.mjs handlers and the dev server's sidecar call the same code, §11.1) ----------
// Fixed 60 s window per key (ip + the validated `hash/fn`). Callers count only a function that exists, so the keys are
// bounded by clients × real functions; HITS_MAX is the hard cap anyway (the oldest window goes first, like REVOKED) and a
// key past HITS_KEY_MAX characters is stored as its digest. The map stays ordered by window start: a new window is
// re-inserted at the end, so the expired entries are always the first ones.
const HITS = new Map() // key → [windowStart, count]
let servers = 0 // each createServer keeps its own dynamic budget (accessBudget) in HITS
export const HITS_MAX = 50000, HITS_KEY_MAX = 256
export const rateLimitKeys = () => HITS.size
const WINDOW = 60000
const dropExpired = now => {
  for (const [k, [t]] of HITS) {
    if (now - t <= WINDOW) break
    HITS.delete(k)
  }
}
let sweeping = null
// Returns 0, or the seconds to wait (Retry-After).
export function overLimit(key, max = 60) {
  if (!sweeping) (sweeping = setInterval(() => dropExpired(Date.now()), WINDOW)).unref()
  key = String(key)
  if (key.length > HITS_KEY_MAX) key = createHash('sha256').update(key).digest('base64url')
  const now = Date.now()
  let h = HITS.get(key)
  if (!h || now - h[0] > WINDOW) {
    if (h) HITS.delete(key)
    dropExpired(now)
    while (HITS.size >= HITS_MAX) HITS.delete(HITS.keys().next().value)
    HITS.set(key, (h = [now, 0]))
  }
  return ++h[1] > max ? Math.ceil((h[0] + WINDOW - now) / 1000) : 0
}
// A server function's id, `<hash>/<name>`: the RPC path and a form's `_brain` field. Anything else is refused before
// it reaches the rate limit (a suffix such as `hash/login/1` would otherwise run `login` under a fresh budget).
const FN_ID = /^([\w-]{1,64})\/([A-Za-z_$][\w$]{0,127})$/
export const fnId = id => {
  const m = typeof id === 'string' ? FN_ID.exec(id) : null
  return m ? { hash: m[1], fn: m[2] } : null
}
// The only functions RPC and form posts may run: the ones a view declares with `server:`, which the generated server
// module lists in `$server`. Its other exports (`load`, `paths`, `sitemap`, `seo`, `head`, `$forms`) run behind the
// route's guards or at render time and are never callable by name; a module without the list exposes nothing.
export const serverFn = (module, name) => {
  const list = module && module.$server
  const f = Array.isArray(list) && typeof name === 'string' && list.includes(name) ? module[name] : undefined
  return typeof f === 'function' ? f : undefined
}
// t(), fmt and locale() read globalThis.__brainLocale. A render sets it while nothing else runs (renders are serialized);
// load(), guards and form handlers overlap across awaits, so each carries its page's language in the async context.
const pageLocale = new AsyncLocalStorage()
let renderLocale = globalThis.__brainLocale
Object.defineProperty(globalThis, '__brainLocale', { configurable: true, get: () => pageLocale.getStore() ?? renderLocale, set: v => (renderLocale = v) })
// ctx.signal: aborted when the client disconnects before the answer ends or the function passes its deadline, so the
// work it started (fetch, a query) stops with it.
const aborts = new WeakMap()
const newSignal = () => {
  const c = new AbortController()
  aborts.set(c.signal, c)
  return c.signal
}
const lifetime = res => {
  const signal = newSignal()
  res.once?.('close', () => res.writableFinished || aborts.get(signal).abort()) // a captured render has no socket
  return signal
}
const withTimeout = (p, ms, ctx) => {
  let t
  return Promise.race([p, new Promise((_, rej) => (t = setTimeout(() => {
    aborts.get(ctx?.signal)?.abort()
    rej(new HttpError(504, 'timeout'))
  }, ms)))]).finally(() => clearTimeout(t))
}
// RPC body → arguments: a JSON array, with __proto__/constructor/prototype keys dropped. Throws HttpError(400).
export function parseArgs(raw) {
  let a
  try {
    a = raw ? JSON.parse(raw, (k, v) => (k === '__proto__' || k === 'constructor' || k === 'prototype' ? undefined : v)) : []
  } catch {
    throw new HttpError(400, 'Body is not JSON')
  }
  if (!Array.isArray(a)) throw new HttpError(400, 'Body must be a JSON array of arguments')
  return a
}
// Runs one `server:` function → { status, body, headers?, cause? }. `args`: raw body string or an array. With `key`, the
// rate limit applies here too. `cause` (500/504) is the real error, for the caller's log; the body stays generic.
export async function callRpc({ fn, args, ctx, key, limit = 60, timeout = 10000 }) {
  if (typeof fn !== 'function') return { status: 404, body: { error: 'Unknown function' } } // never counted: no key per guess
  if (key) {
    const ra = overLimit(key, limit)
    if (ra) return { status: 429, body: { error: 'Too many requests' }, headers: { 'retry-after': String(ra) } }
  }
  let a
  try {
    a = typeof args === 'string' || args == null ? parseArgs(args || '') : args
  } catch (e) {
    return { status: e.status || 400, body: { error: e.message } }
  }
  try {
    ctx.signal ||= newSignal() // the dev sidecar builds its own ctx
    const v = await withTimeout(Promise.resolve(fn(ctx, ...a)), timeout, ctx)
    return { status: 200, body: v ?? null }
  } catch (e) {
    if (e && e.errors && typeof e.errors === 'object') return { status: 422, body: { error: e.message || 'Invalid input', errors: e.errors } }
    if (e instanceof HttpError && e.status === 504) return { status: 504, body: { error: 'Timeout' }, cause: e }
    const pub = publicError(e)
    if (pub) return { status: pub.status, body: { error: pub.message } }
    const deliberate = intendedStatus(e)
    if (deliberate && deliberate < 500) return { status: deliberate, body: { error: e instanceof HttpError && e.message !== NATIVE_FAILED ? e.message : STATUS_CODES[deliberate] } }
    if (deliberate) return {status:deliberate,body:{error:STATUS_CODES[deliberate]},cause:e}
    return { status: 500, body: { error: 'Internal error' }, cause: e }
  }
}
// A form posted without JS (§38) → { status: 303, location } | { status: 422, state } | { status, body } (400/429).
// `module` exports the function and `$forms`; `rules` = { rule, check } (brain-schema / brain-form). `state` is what the
// page re-renders with: { '<hash>/<fn>': { values, errors, error } }.
// `locale` (§37): the page's language, so the schema messages (`form.*`) and the generic error come from its dictionary
// (globalThis.__brainDict, filled by the page's chunk) like form() in the browser. It is set only around the synchronous
// validation, so concurrent requests in other languages never see it.
// English text for the server's log: a form or a server function never shows it to a visitor
const NATIVE_FAILED = 'Native resource request failed'
export async function callForm({ module, raw, ctx, rules, key, limit = 60, timeout = 10000, locale }) {
  const q = new URLSearchParams(raw || ''), id = q.get('_brain') || '', fn = fnId(id)?.fn
  // exactly `<hash>/<name>` naming a `server:` function, checked before the rate limit counts anything
  const run = fn && serverFn(module, fn)
  if (!run) return { status: 400, body: { error: 'Unknown form' } }
  if (key) {
    const ra = overLimit(key, limit)
    if (ra) return { status: 429, body: { error: 'Too many requests' }, headers: { 'retry-after': String(ra) } }
  }
  const schema = module.$forms && module.$forms[fn]
  const G = globalThis, had = '__brainLocale' in G, prev = G.__brainLocale
  let values, errors, generic
  if (locale) G.__brainLocale = locale
  try {
    ;({ values, errors } = schema ? coerce(rules, schema, q) : { values: Object.fromEntries([...q].filter(([k]) => k !== '_brain')), errors: {} })
    const D = G.__brainDict, m = locale && D && D[locale] && D[locale]['form.error']
    generic = typeof m === 'string' ? m : 'Something went wrong. Try again.'
  } finally {
    if (locale) had ? (G.__brainLocale = prev) : delete G.__brainLocale
  }
  const again = (errs, error, cause) => ({ status: 422, state: { [id]: { values, errors: errs, error } }, cause })
  if (Object.keys(errors).length) return again(errors, '')
  let r
  try {
    ctx.signal ||= newSignal()
    r = await withTimeout(pageLocale.run(locale, () => Promise.resolve(run(ctx, values))), timeout, ctx)
  } catch (e) {
    if (e && e.errors && typeof e.errors === 'object') return again(e.errors, '')
    if (e instanceof HttpError && e.status >= 400 && e.status < 500) return again({}, e.message === NATIVE_FAILED ? generic : e.message)
    const pub = publicError(e)
    if (pub) return again({}, pub.message)
    const deliberate = intendedStatus(e)
    if (deliberate && deliberate < 500) return {status:deliberate,body:{error:STATUS_CODES[deliberate]}}
    if (deliberate) return {status:deliberate,body:{error:STATUS_CODES[deliberate]},cause:e}
    return again({}, generic, e)
  }
  if (r && r.errors) return again(r.errors, '')
  const to = r && r.redirect != null ? safeLocation(r.redirect) : ctx.url.pathname + ctx.url.search
  return { status: 303, location: to }
}

// ---------- pure helpers ----------
// form-encoded → values with the schema's types + messages (same rules and texts as form() in the browser)
function coerce(F, schema, q, base = '', values = {}, errors = {}) {
  for (const k of Object.keys(schema)) {
    const path = base ? base + '.' + k : k, r = F.rule(schema[k])
    if (r.t === 'obj') {
      values[k] = {}
      coerce(F, r.s, q, path, values[k], errors)
      continue
    }
    const one = v => {
      if (r.t === 'num') {
        if (v == null || v.trim() === '') return null
        // Validate the original decimal token before conversion; Number('0x10') hides invalid form input.
        return F.check({ ...r, list: false, req: false, min: null, max: null }, v) ? v : Number(v)
      }
      if (r.t === 'bool') return v == null || v === '' || v === 'false' || v === '0' ? false : v === 'on' || v === 'true' || v === '1' ? true : v
      return v == null ? '' : v
    }
    const v = r.list ? q.getAll(path).map(one) : r.t === 'bool' ? one(q.get(path)) : one(q.get(path) ?? '')
    values[k] = v
    const msg = F.check(r, r.t === 'num' && !r.list && v === null ? '' : r.t === 'num' && !r.list && Number.isNaN(v) ? q.get(path) : v)
    if (msg) errors[path] = msg
  }
  return { values, errors }
}
function readManifest(dist) {
  const f = join(dist, '_server', 'manifest.json')
  const m = existsSync(f) ? JSON.parse(readFileSync(f, 'utf8')) : { routes: [], rpc: {} }
  return { core: '/_b/brain.js', notFound: '404.html', ...m, routes: m.routes || [], rpc: m.rpc || {} }
}
// a route without `rx` (older manifest) matches only its exact path
const exact = p => new RegExp('^' + p.replace(/[.*+?^${}()|[\]\\]/g, '\\$&') + '\\/?$')
function parseCookies(h = '') {
  const out = {}
  for (const part of h.split(';')) {
    const i = part.indexOf('=')
    if (i > 0) {
      const k = part.slice(0, i).trim()
      if (k && !(k in out)) {
        // Another site's cookie on this domain may hold anything: it must not cost the page.
        const value = part.slice(i + 1).trim()
        try { out[k] = decodeURIComponent(value) } catch { out[k] = value }
      }
    }
  }
  return out
}
const plainIp = text => { const ip = String(text).trim().replace(/^"|"$/g, '').replace(/^\[([^\]]+)\](?::\d+)?$/, '$1').replace(/^(\d+\.\d+\.\d+\.\d+):\d+$/, '$1').replace(/^::ffff:(\d+\.\d+\.\d+\.\d+)$/i, '$1'); return isIP(ip) ? ip : null }
/** platform.trustProxy: one BlockList of proxies. Same limits as the backend's parse_proxy (brain-backend/src/platform.rs). */
export function trustedProxies(list = []) {
  const nets = new BlockList()
  for (const entry of list) {
    const [address, prefix, extra] = String(entry).split('/')
    const family = isIP(address)
    const bits = prefix === undefined ? (family === 4 ? 32 : 128) : Number(prefix)
    const wide = family === 4 ? bits < 8 : bits < 16 && !(bits === 7 && /^fc00::$/i.test(address))
    if (!family || extra !== undefined || !Number.isInteger(bits) || bits > (family === 4 ? 32 : 128)) throw Error(`trustProxy entry \`${entry}\` must be an IP address or CIDR range`)
    if (wide) throw Error(`trustProxy entry \`${entry}\` is too wide; use at least /8 for IPv4 and /16 for IPv6, and list your proxy ranges`)
    nets.addSubnet(address, bits, family === 4 ? 'ipv4' : 'ipv6')
  }
  return list.length ? nets : null
}
/**
 * The client is the right-most forwarded hop that is not a trusted proxy, and only when the TCP peer is trusted: hops
 * left of it are client-supplied and never consulted. Same walk as resolve_client (brain-backend/src/edge_http.rs).
 */
/**
 * platform.access (docs/backend/access.md): who may reach the dynamic answers and how much each client may take.
 * `mode` "open" (default) lets any client in, scripts and headless browsers included; "own-host" refuses /api, /_brain
 * and /s to anything without evidence of this site. Nothing is limited unless the app sets `perMinute` (dynamic
 * requests per client and minute) or `concurrency` (dynamic requests in flight for the whole process). Normalizes `{ mode, perMinute, concurrency, origins, hosts, ips,
 * local }`; `site` is the site origin (PUBLIC_SITE_URL), whose host is always one of the hosts.
 * Same limits as the backend's parse_access (brain-backend/src/platform.rs).
 */
export function accessPolicy(raw = {}, site) {
  if (raw === null || typeof raw !== 'object' || Array.isArray(raw)) throw Error('platform.access must be an object')
  for (const key of Object.keys(raw)) if (!['mode', 'perMinute', 'concurrency', 'origins', 'hosts', 'ips', 'local'].includes(key)) throw Error(`platform.access.${key} is not a setting; use mode, perMinute, concurrency, origins, hosts, ips or local`)
  const list = (key, max) => {
    const value = raw[key] ?? []
    if (!Array.isArray(value) || value.length > max || value.some(v => typeof v !== 'string' || !v || v.length > 256)) throw Error(`platform.access.${key} must be an array of at most ${max} strings`)
    return value
  }
  const origins = new Set()
  for (const origin of list('origins', 32)) {
    let parsed
    try { parsed = new URL(origin) } catch { throw Error(`platform.access.origins entry \`${origin}\` must be an origin such as https://admin.example.com`) }
    if (parsed.origin !== origin || !/^https?:$/.test(parsed.protocol)) throw Error(`platform.access.origins entry \`${origin}\` must be exactly scheme://host[:port], without a path or a trailing slash`)
    origins.add(origin)
  }
  const hosts = new Set()
  for (const host of list('hosts', 32)) {
    if (!/^(?=.{1,253}$)[a-z0-9](?:[a-z0-9-]*[a-z0-9])?(?:\.[a-z0-9](?:[a-z0-9-]*[a-z0-9])?)*$/.test(host)) throw Error(`platform.access.hosts entry \`${host}\` must be a lowercase host name without scheme, port or wildcard`)
    hosts.add(host)
  }
  try { if (site) hosts.add(new URL(site).hostname) } catch {}
  const ips = list('ips', 64)
  const nets = ips.length ? trustedProxies(ips) : null
  if (raw.local !== undefined && typeof raw.local !== 'boolean') throw Error('platform.access.local must be true or false')
  const mode = raw.mode ?? 'open'
  if (mode !== 'open' && mode !== 'own-host') throw Error('platform.access.mode must be "open" or "own-host"')
  const count = (key, max) => {
    const value = raw[key] ?? null
    if (value !== null && (!Number.isInteger(value) || value < 1 || value > max)) throw Error(`platform.access.${key} must be a whole number from 1 to ${max}`)
    return value
  }
  return Object.freeze({ mode, perMinute: count('perMinute', 100000), concurrency: count('concurrency', 10000), origins, hosts, nets, local: raw.local !== false })
}
// Paths a third party reaches on purpose: links, the analytics script, signed inbound webhooks and Apple's
// server-to-server notifications. Each carries its own check.
const ACCESS_EXEMPT = /^\/(?:s\/|_brain\/analytics\.js$|_brain\/webhooks\/in\/[a-z][a-z0-9_]{0,47}$|_brain\/auth\/apple\/notifications$)/
const LOOPBACK = ip => /^(?:127\.|::1$)/.test(ip)
/**
 * Why a request to /api, /_brain or /s may not reach the backend, or null when it may. It passes with browser evidence
 * of this site (an Origin of this host or of access.origins, Sec-Fetch-Site same-origin, a same-host Referer), a
 * non-ambient credential the backend verifies (Authorization, X-Api-Key), a client address in access.ips, or a
 * loopback client while access.local is on. Headers can be forged: this keeps other sites' pages, foreign hosts and
 * plain scripts out, and the per-client rate limits bound what a forger reads.
 */
export function accessRefusal(req, path, client, policy) {
  if (!policy || policy.mode !== 'own-host' || ACCESS_EXEMPT.test(path) || req.method === 'OPTIONS') return null
  const h = req.headers
  const host = String(h['x-forwarded-host'] || h.host || '').toLowerCase()
  const hostname = host.replace(/:\d+$/, '').replace(/^\[|\]$/g, '')
  const local = policy.local && LOOPBACK(client)
  if (policy.hosts.size && !policy.hosts.has(hostname) && !(local && ['localhost', '127.0.0.1', '::1'].includes(hostname))) return 'host'
  if (local) return null
  if (h.authorization || h['x-api-key']) return null
  if (policy.nets && client !== '?' && policy.nets.check(client, isIP(client) === 4 ? 'ipv4' : 'ipv6')) return null
  const own = value => {
    try {
      const url = new URL(value)
      return url.host === host || policy.origins.has(url.origin)
    } catch { return false }
  }
  if (h.origin) return own(h.origin) ? null : 'origin'
  const site = h['sec-fetch-site']
  const navigation = (req.method === 'GET' || req.method === 'HEAD') && h['sec-fetch-mode'] === 'navigate'
  if (site === 'same-origin') return null
  // a person who opened the URL, or a sign-in provider sending the browser back to its callback
  if (navigation && (site === 'none' || path.startsWith('/_brain/auth/'))) return null
  if (h.referer && own(h.referer)) return null
  return site ? 'cross-site' : 'no-browser-evidence'
}
/**
 * Whether a dynamic request (a page render, a form post, /api, /_brain, /s) stays inside its budget: 0, or the seconds
 * to wait. Checked before any body is read or the backend is asked, so a refused client costs one map lookup. A
 * loopback client while access.local is on, an address in access.ips and the exempt callbacks are not counted.
 */
export function accessBudget(path, client, policy, key = 'edge ') {
  if (!policy || !policy.perMinute || ACCESS_EXEMPT.test(path)) return 0
  if (policy.local && LOOPBACK(client)) return 0
  if (policy.nets && client !== '?' && policy.nets.check(client, isIP(client) === 4 ? 'ipv4' : 'ipv6')) return 0
  return overLimit(key + client, policy.perMinute)
}
export function resolveClient(peer, forwarded, nets) {
  const socketPeer = plainIp(peer ?? '')
  const trusted = ip => nets.check(ip, isIP(ip) === 4 ? 'ipv4' : 'ipv6')
  if (!socketPeer) return '?'
  if (!nets || !trusted(socketPeer)) return socketPeer
  let client = socketPeer
  for (const text of String(forwarded ?? '').split(',').reverse()) {
    const ip = plainIp(text)
    if (!ip) return client
    client = ip
    if (!trusted(ip)) return ip
  }
  return client
}
export function readBody(req, max) { return body(req, max) }
function body(req, max) {
  return new Promise((ok, fail) => {
    let n = 0
    const parts = []
    // past the limit the rest is drained without keeping it (so the 413 reaches the client); far past it, cut
    req.on('data', c => {
      n += c.length
      if (n > max + Math.min(15 * max, 16 << 20)) req.destroy() // at most 16 MB of drain for a large limit
      else if (n <= max) parts.push(c)
    })
    req.on('end', () => (n > max ? fail(new Error('too large')) : ok(Buffer.concat(parts).toString('utf8'))))
    // a disconnect or the cut above never reaches 'end': the reader settles anyway and nobody is left to answer
    const aborted = () => fail(Object.assign(new Error('body aborted'), { aborted: true }))
    req.on('error', aborted)
    req.on('close', aborted)
  })
}
// bind:value / checked are properties; copy them into attributes so the SSR HTML shows them
function fillValues(n) {
  for (const c of n.childNodes || []) {
    if (c.nodeType !== 1) continue
    const t = (c.tagName || '').toLowerCase()
    if (t === 'textarea' && c.value != null) c.textContent = String(c.value)
    else if (t === 'input' && c.type !== 'password') {
      if (c.value != null && c.value !== '') c.setAttribute('value', String(c.value))
      if (c.checked === true) c.setAttribute('checked', '')
    } else if (t === 'option' && c.selected === true) c.setAttribute('selected', '')
    // <select value={x}>: the option with that value starts selected (an option's value is its attribute or text)
    else if (t === 'select' && c.value != null && c.value !== '') {
      const pick = o => {
        for (const x of o.childNodes || []) {
          if (x.nodeType !== 1) continue
          if (x.tagName === 'OPTION' && (x.getAttribute('value') ?? x.textContent) === String(c.value)) x.setAttribute('selected', '')
          else pick(x)
        }
      }
      pick(c)
    }
    fillValues(c)
  }
}
// `/` → index.md, `/docs/setup/` → docs/setup.md (brain-seo files::md_path)
const mdPath = p => (p.replace(/^\/+|\/+$/g, '') || 'index') + '.md'
// true when `Accept` ranks text/markdown above text/html (an equal weight goes to whichever is listed first);
// wildcards alone never select Markdown, so browsers keep getting HTML
export function prefersMarkdown(accept) {
  if (!accept) return false
  let md = -1, html = -1, mdAt = 0, htmlAt = 0, any = -1
  accept.split(',').forEach((part, i) => {
    const [type, ...params] = part.trim().toLowerCase().split(';')
    const qp = params.map(x => x.trim()).find(x => x.startsWith('q='))
    const q = qp ? Number(qp.slice(2)) : 1
    if (!(q >= 0)) return
    if (type === 'text/markdown' || type === 'text/x-markdown') (md = Math.max(md, q)), (mdAt = mdAt || i + 1)
    else if (type === 'text/html' || type === 'application/xhtml+xml') (html = Math.max(html, q)), (htmlAt = htmlAt || i + 1)
    else if (type === 'text/*' || type === '*/*') any = Math.max(any, q)
  })
  if (html < 0) (html = any), (htmlAt = Infinity) // an explicit type beats a wildcard of the same weight
  return md > 0 && (md > html || (md === html && mdAt < htmlAt))
}
// Root-relative image metadata follows the app base; bare references follow the canonical document URL.
export function metadataImage(root, canonical, value) {
  const encode = s => encodeURI(s.toWellFormed()).replaceAll('%25', '%')
  value = value.trim()
  if (!value) return ''
  if (/^[A-Za-z][A-Za-z0-9+.-]*:/.test(value) || !root) return encode(value)
  try {
    const home = new URL(root.endsWith('/') ? root : root + '/')
    const prefix = home.pathname.replace(/\/$/, '')
    if (/^\/(?!\/)/.test(value)) value = new URL(value === prefix || value.startsWith(prefix + '/') ? value : prefix + value, home.origin).href
    return new URL(value, canonical || home).href
  } catch { return encode(value) }
}
// SEO head from View.seo(props): title, description, canonical, og:*/twitter:*, JSON-LD and the view's <head> block
// F5: the build writes an SSR route's canonical/og:url from its pattern (/posts/:slug); the request has the real path.
// A view's own <head> tags replace the generated ones with the same name/property (one tag per key, SPEC «SEO»).
function head(page, seo, path, search = '', route, base = '') {
  let h = page
  // `<meta seo query="page">`: these parameters make a different page; any other (a dialog, a tracking tag) does not
  let identity = '', query = ''
  h = h.replace(/<meta name="brain-query" content="([^"]*)">/, (_, keys) => {
    const given = new URLSearchParams(search), kept = new URLSearchParams()
    for (const key of keys.split(/[\s,]+/)) for (const value of key ? given.getAll(key) : []) kept.append(key, value)
    if (kept.size) identity = esc((query = '?' + kept))
    return ''
  })
  if (path) {
    // the attribute is already escaped HTML: only the request path that replaces a pattern is escaped
    const real = v => v.replace(/^(https?:\/\/[^/]+)?(\/[^"]*)$/, (m, o = '', p) => (/\/[:*]/.test(p) ? o + esc(base + path) : m)) + identity
    h = h.replace(/(<link[^>]*rel=["']canonical["'][^>]*href=")([^"]*)(")/i, (m, a, v, b) => a + real(v) + b)
    h = h.replace(/(<meta[^>]*property=["']og:url["'][^>]*content=")([^"]*)(")/i, (m, a, v, b) => a + real(v) + b)
  }
  if (!seo) return graph(h, path ? base + path : path, query)
  // Match prerendering: blank optional fields are absent; zero and false remain distinct values.
  seo = Object.fromEntries(Object.entries(seo).filter(([, v]) => v != null && !(typeof v === 'string' && !v.trim())))
  const meta = (attr, key, val, escaped) => {
    const tag = `<meta ${attr}="${esc(key)}" content="${escaped ? val.replace(/"/g, '&quot;') : esc(val)}">`
    const rx = new RegExp(`<meta[^>]*\\b${attr}=["']${key.replace(/[.*+?^${}()|[\]\\:]/g, '\\$&')}["'][^>]*>`, 'i')
    h = rx.test(h) ? h.replace(rx, () => tag) : h.replace('</head>', () => tag + '</head>')
  }
  for (const [k, v] of Object.entries(seo)) {
    // `schema` becomes JSON-LD below and `lastmod` feeds it: neither is a meta tag (prerender seo_head does the same)
    if (v == null || k === '$head' || k === 'schema' || k === 'lastmod') continue
    if (k === 'title') h = /<title>[\s\S]*?<\/title>/i.test(h) ? h.replace(/<title>[\s\S]*?<\/title>/i, () => `<title>${esc(v)}</title>`) : h.replace('</head>', () => `<title>${esc(v)}</title></head>`)
    else if (k === 'canonical') {
      // a root-relative canonical takes the origin the build wrote into the template (PUBLIC_SITE_URL), as it does on
      // a prerendered page: a relative canonical is ignored by search engines
      const relative = base && (v + '/').startsWith(base + '/') ? v.slice(base.length) || '/' : v
      const translated = linkTo(route, relative)
      const local = /^\/(?!\/)/.test(translated) ? base + translated : translated
      const built = /^\/(?!\/)/.test(local) && /<link[^>]*rel=["']canonical["'][^>]*href="(https?:\/\/[^/"]+)/i.exec(h)?.[1]
      const url = built ? built + local : local
      const tag = `<link rel="canonical" href="${esc(url)}">`
      h = /<link[^>]*rel=["']canonical["'][^>]*>/i.test(h) ? h.replace(/<link[^>]*rel=["']canonical["'][^>]*>/i, () => tag) : h.replace('</head>', () => tag + '</head>')
      if (seo['og:url'] == null) meta('property', 'og:url', url) // og:url follows the canonical unless the view sets its own
    } else if (k === 'jsonld') h = h.replace('</head>', () => `<script type="application/ld+json">${jsonInHtml(v)}</script></head>`)
    else if (/^(og|twitter|article):/.test(k)) meta(k.startsWith('twitter:') ? 'name' : 'property', k, v)
    else if (typeof v === 'string') meta('name', k, v)
  }
  // What the build writes for a prerendered page and can only be known here: the social card of this request's values
  // (brain-seo social::head, prerender seo_head). The view's own value always wins.
  const shared = /<title>([\s\S]*?)<\/title>/i.exec(h)?.[1]
  const origin = /<link[^>]*rel=["']canonical["'][^>]*href="(https?:\/\/[^/"]+)/i.exec(h)?.[1]
  const canonical = /<link[^>]*rel=["']canonical["'][^>]*href="([^"]+)"/i.exec(h)?.[1].replaceAll('&amp;', '&')
  for (const key of ['og:image', 'og:image:url', 'og:image:secure_url', 'twitter:image', 'twitter:image:src']) if (typeof seo[key] === 'string') {
    seo[key] = metadataImage(origin ? origin + base : null, canonical, seo[key])
    meta(key.startsWith('twitter:') ? 'name' : 'property', key, seo[key])
  }
  if (seo.title != null && seo['og:title'] == null) meta('property', 'og:title', shared, true)
  if (seo.description != null && seo['og:description'] == null) meta('property', 'og:description', seo.description)
  if (seo['og:image'] != null) {
    if (seo['og:image:alt'] == null) meta('property', 'og:image:alt', seo['og:title'] ?? shared, seo['og:title'] == null)
    if (seo['twitter:card'] == null) meta('name', 'twitter:card', 'summary_large_image')
  }
  // `<meta seo schema="Article">` without `jsonld` (§32): one node with this request's values. It replaces the node
  // the build wrote into the template from the static attributes (same @type), so the page carries exactly one.
  if (typeof seo.schema === 'string' && seo.jsonld == null && /^[A-Za-z]+$/.test(seo.schema)) {
    const T = seo.schema
    const canon = /<link[^>]*rel=["']canonical["'][^>]*href="([^"]*)"/i.exec(h)
    const node = { '@context': 'https://schema.org', '@type': T }
    if (seo.title != null) node[/^(Article|BlogPosting|NewsArticle|TechArticle|Report)$/.test(T) ? 'headline' : 'name'] = String(seo.title).slice(0, 110)
    if (seo.description != null) node.description = String(seo.description)
    if (canon) node.url = canon[1].replace(/&amp;/g, '&')
    if (seo['og:image'] != null) node.image = seo['og:image']
    if (seo.lastmod != null) node.dateModified = String(seo.lastmod)
    const own = new RegExp(`<script[^>]*type=["']application/ld\\+json["'][^>]*>\\s*\\{"@context":"https://schema\\.org","@type":"${T}"[\\s\\S]*?</script>`, 'i')
    const tag = `<script type="application/ld+json">${jsonInHtml(node)}</script>`
    h = own.test(h) ? h.replace(own, () => tag) : h.replace('</head>', () => tag + '</head>')
  }
  if (seo.$head) {
    for (const [, attr, key] of seo.$head.matchAll(/<meta[^>]*\b(property|name)=["']([^"']+)["'][^>]*>/gi)) {
      const attributes = key.startsWith('twitter:') ? '(?:property|name)' : attr
      h = h.replace(new RegExp(`<meta[^>]*\\b${attributes}=["']${key.replace(/[.*+?^${}()|[\]\\:]/g, '\\$&')}["'][^>]*>`, 'gi'), '')
    }
    if (/<link[^>]*rel=["']canonical["']/i.test(seo.$head)) h = h.replace(/<link[^>]*rel=["']canonical["'][^>]*>/gi, '')
    h = h.replace('</head>', () => seo.$head + '</head>')
  }
  return graph(h, path ? base + path : path, query, seo)
}
// The WebSite + WebPage graph the build wrote into the template (brain-seo ServerHead::apply): its pattern URLs become
// this request's, like the canonical, and the WebPage takes this request's title and description.
function graph(h, path, query, seo) {
  return h.replace(/(<script type="application\/ld\+json">)(\{"@context":"https:\/\/schema\.org","@graph"[\s\S]*?)(<\/script>)/, (m, a, body, b) => {
    let g
    try { g = JSON.parse(body) } catch { return m }
    const url = v => (typeof v === 'string' && path ? v.replace(/^(https?:\/\/[^/#]+)(\/[^#]*)(#.*)?$/, (u, o, p, hash = '') => (/\/[:*]/.test(p) ? o + path + query + hash : u)) : v)
    const title = /<title>([\s\S]*?)<\/title>/i.exec(h)?.[1]
    for (const node of g['@graph'] ?? []) {
      for (const k of ['@id', 'url']) if (k in node) node[k] = url(node[k])
      if (node.isPartOf?.['@id']) node.isPartOf['@id'] = url(node.isPartOf['@id'])
      if ([node['@type']].flat().includes('WebPage')) {
        if (title && node.name == null) node.name = title.replace(/&lt;/g, '<').replace(/&gt;/g, '>').replace(/&quot;/g, '"').replace(/&#39;/g, "'").replace(/&amp;/g, '&')
        if (seo?.description != null && node.description == null) node.description = String(seo.description)
        if (typeof seo?.['og:image'] === 'string' && seo.jsonld == null) {
          const image = node.primaryImageOfPage
          if (!image || (image['@type'] === 'ImageObject' && !image['@id'])) node.primaryImageOfPage = { '@type': 'ImageObject', ...image, url: seo['og:image'] }
        }
      }
    }
    return a + jsonInHtml(g) + b
  })
}
// Only compiler-owned chunks receive the own-extension module format; ordinary Node imports keep their loader.
function resolveChunk(s, c, next, base, app) {
  if (app !== '/' && s.startsWith(app + '_b/')) s = s.slice(app.length - 1)
  const mapped = s.startsWith('/_b/')
  const url = mapped ? new URL('.' + s, base) : s.startsWith('file:') ? new URL(s) : s.startsWith('.') && c.parentURL?.startsWith('file:') ? new URL(s, c.parentURL) : null
  const owned = new URL('_b/', base).href
  if (url?.href.startsWith(owned) && url.pathname.endsWith('.jsbrain')) return { url: url.href, format: 'module', shortCircuit: true }
  if (mapped) return { url: url.href, shortCircuit: true }
  return next(s, c)
}
let hooked = false
function hook(base, app = '/') {
  if (hooked) return
  hooked = true
  // registerHooks (Node 22.15+, in-thread) and the compatibility loader share exactly the same resolver.
  if (Module.registerHooks) return void Module.registerHooks({ resolve: (s, c, next) => resolveChunk(s, c, next, base, app) })
  Module.register('data:text/javascript,' + encodeURIComponent(`${resolveChunk.toString()};export async function resolve(s,c,n){return resolveChunk(s,c,n,${json(base)},${json(app)})}`))
}

const LISTEN_WAIT_MS = 10000
// the supervisor gives an instance 30 s to print brain:ready: warming may hold it this long at most
const WARM_WAIT_MS = 15000
// `node server.mjs` (directly or under `brain start`); paths compared after symlinks (/var → /private/var on macOS)
function real(p) {
  try {
    return realpathSync(resolve(p))
  } catch {
    return resolve(p)
  }
}
if (process.argv[1] && real(process.argv[1]) === real(fileURLToPath(import.meta.url))) {
  process.env.NODE_ENV = 'production'
  // A closed log pipe is the operator's matter: requests do not depend on it.
  for (const stream of [process.stdout, process.stderr]) stream.on('error', error => { if (error.code !== 'EPIPE') throw error })
  const s = createServer({ dist: fileURLToPath(new URL('.', import.meta.url)), warm: true })
  const fd = ENV.BRAIN_LISTEN_FD
  const where = fd ? { fd: Number(fd) } : { port: +(ENV.PORT || 3000), host: ENV.HOST || '127.0.0.1' }
  s.once('listening', async () => {
    // ready once the app's modules are imported (see `warm`), so a reload only switches traffic to a warm instance;
    // a slow machine gets WARM_WAIT_MS at most, the rest keeps importing while requests are served
    let timer
    const warmed = await Promise.race([s.$brainWarm, new Promise(r => (timer = setTimeout(r, WARM_WAIT_MS)))])
    clearTimeout(timer)
    process.stdout.write(`brain:ready ${process.pid}\n`) // the supervisor waits for this line (§34)
    console.log(json({ t: new Date().toISOString(), level: 'info', msg: 'listening', ...(fd ? { fd: Number(fd) } : where), ...(warmed ? { warmModules: warmed.modules, warmFailed: warmed.failed } : { warm: 'continuing' }) }))
  })
  // A restart may find the previous instance still draining: wait for it, briefly. Two live servers never share a port.
  const waitUntil = Date.now() + LISTEN_WAIT_MS
  s.on('error', error => {
    if (error.code === 'EADDRINUSE' && !fd && !s.listening && Date.now() < waitUntil) return void setTimeout(() => s.listen(where), 100)
    if (s.listening) throw error
    const port = where.port
    const fix = error.code === 'EADDRINUSE' ? `find the owner with \`lsof -nP -iTCP:${port} -sTCP:LISTEN\` (Linux: \`ss -ltnp 'sport = :${port}'\`), stop it, or set another PORT` : 'use another HOST or PORT'
    const cause = error.code === 'EADDRINUSE' ? `another live process still holds the port after ${LISTEN_WAIT_MS / 1000} s` : error.code
    console.error(json({ t: new Date().toISOString(), level: 'error', msg: `cannot listen on ${where.host}:${port}: ${cause}`, fix }))
    process.exit(1)
  })
  s.listen(where)
  // SIGTERM/SIGINT: stop accepting, answer what is in flight or arrives on open connections (each then closes), for up
  // to BRAIN_DRAIN_MS (10 s), then cut whatever is left; exit 0 either way
  const drain = () => {
    if (s.draining) return
    s.draining = true
    s.$brainSocketStop?.()
    const ms = +(ENV.BRAIN_DRAIN_MS || 10000)
    const force = setTimeout(() => {
      s.closeAllConnections()
      process.exit(0)
    }, ms)
    force.unref()
    // Stop accepting but drop nothing (QA #120): a keep-alive connection between two requests, or one just accepted
    // whose request is not parsed yet, still gets its answer, with `Connection: close`. (http.Server#close would
    // destroy idle connections: under load that is an ECONNRESET per reload.) Idle sockets end by keepAliveTimeout.
    s.prependListener('request', (q, r) => {
      r.shouldKeepAlive = false
    })
    net.Server.prototype.close.call(s, () => process.exit(0))
  }
  process.on('SIGTERM', drain)
  process.on('SIGINT', drain)
}
