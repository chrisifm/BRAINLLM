// BRAINLLM production server (INTERNALS §11.1, §22, §31). Node ≥ 22, zero dependencies (node:http).
// `brain build` copies this file to dist/server.mjs when the app has `load` or `server:` functions; `brain start`
// (INTERNALS §34) runs it with the listening socket inherited as BRAIN_LISTEN_FD (else PORT/HOST) and with the
// environment it resolved from `.brain_env` (§36): this file reads no env file itself. It prints `brain:ready <pid>`
// once it accepts and drains on SIGTERM.
//
// It serves, in this order:
//   GET  /_health                → 200 {"ok":true,"build":"<sig>"}
//   POST /_r/<hash>/<fn>         → RPC to a `server:` function (same origin + X-Brain-RPC, 64 KB, rate limit, 10 s)
//   GET  routes in the manifest  → SSR: load(ctx) → render the view on the DOM shim → page with props and SEO head
//   GET  files in dist/          → static (precompressed .br/.gz when accepted; /_b/* immutable), clean URLs
//   anything else                → 404 page
// Every response carries the security headers; every HTML response gets a fresh CSP nonce on each <script>.
//
// dist/_server/manifest.json (emitted by brain build):
//   { v, build, core, routes: [{ path, view, layouts, server, page }], rpc: { <hash>: <module> }, notFound }
// `build` is the build signature (§35.7): sent as X-Brain-Build on every HTML response and in /_health.
// A page template (`page`) is the route's final HTML with `<!--brain:ssr-->` inside #brain-view and
// `<!--brain:props-->` inside <script type="application/json" id="brain-props">.
import { createServer as http, STATUS_CODES, request as httpRequest, Agent as HttpAgent } from 'node:http'
import net from 'node:net'
import { installNativeSockets } from './native-sockets.mjs'
import { Readable } from 'node:stream'
import { createHash, createHmac, randomBytes, randomUUID, timingSafeEqual } from 'node:crypto'
import { existsSync, readFileSync, realpathSync, statSync, lstatSync } from 'node:fs'
import * as Module from 'node:module'
import { basename, dirname, extname, join, normalize, resolve, sep } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'

const MIME = {
  '.html': 'text/html; charset=utf-8', '.js': 'text/javascript; charset=utf-8', '.mjs': 'text/javascript; charset=utf-8',
  '.css': 'text/css; charset=utf-8', '.json': 'application/json', '.svg': 'image/svg+xml', '.png': 'image/png',
  '.jpg': 'image/jpeg', '.jpeg': 'image/jpeg', '.webp': 'image/webp', '.avif': 'image/avif', '.gif': 'image/gif',
  '.ico': 'image/x-icon', '.woff2': 'font/woff2', '.woff': 'font/woff', '.txt': 'text/plain; charset=utf-8',
  '.xml': 'application/xml', '.webmanifest': 'application/manifest+json', '.map': 'application/json',
  '.mp4': 'video/mp4', '.webm': 'video/webm', '.pdf': 'application/pdf', '.wasm': 'application/wasm',
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
  'x-powered-by': 'BRAINLLM (https://chrisifm.github.io/BRAINLLM/)',
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
export class HttpError extends Error {
  constructor(status, message, extra) {
    super(message)
    this.status = status
    Object.assign(this, extra)
  }
}
// Plain deliberate 4xx signals are portable from server-only application modules; exception messages stay private.
function intendedStatus(error) {
  if (!error || typeof error !== 'object') return null
  const descriptor = Object.getOwnPropertyDescriptor(error, 'status')
  const status = descriptor && descriptor.value
  const plain = Object.getPrototypeOf(error) === Object.prototype || Object.getPrototypeOf(error) === null
  return Number.isInteger(status) && STATUS_CODES[status] && (error instanceof HttpError && status >= 400 && status <= 599 || plain && status >= 400 && status < 500) ? status : null
}
const REDIRECT_CODES = new Set([301, 302, 303, 307, 308])
function safeLocation(value) {
  if (typeof value !== 'string' || value.length > 8192 || !value.startsWith('/') || value.startsWith('//') || /[\\\x00-\x20\x7f]/.test(value) || /^\/%2f/i.test(value) || /%(?:0[0-9a-f]|1[0-9a-f]|5c|7f)/i.test(value) || /%20/i.test(value.split(/[?#]/,1)[0])) throw new HttpError(500, 'Invalid same-site redirect location')
  return value
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

// Coding weights are transport preferences, not substring matches (br;q=0 explicitly forbids Brotli).
export function acceptedEncodings(header = '') {
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
  return ['br','gzip'].map(name => [name,values.get(name) ?? values.get('*') ?? 0]).filter(([,q]) => q > 0).sort((a,b) => b[1] - a[1]).map(([name]) => name)
}

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
    if (attrs.get('name')?.toLowerCase() === 'brain-google-signin' && attrs.get('content') === '1') return true
  }
  return false
}

// One configured loopback authority. No user-controlled target, proxy, redirect, auth cache or pending queue.
export function nativeAuthBridge(raw) {
  if (!raw) return null
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
    return Object.freeze({id:user.id,email:user.email,role:user.role,verified:user.verified})
  }
  const unavailable = () => new HttpError(503, 'Native authentication is temporarily unavailable')
  const call = (incoming, response, path, method, rid, origin) => {
    if (stopped || active >= 8) return Promise.reject(unavailable())
    const cookie = cookies(incoming, method === 'POST')
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
        outgoing = httpRequest(new URL(path, target), { method, agent, headers: { accept: 'application/json', ...(cookie ? { cookie } : {}), 'x-request-id': rid, ...(origin ? { origin } : {}), ...(method === 'POST' ? { 'content-length': '0' } : {}) } }, backend => {
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
              if (names.has(name) || name === '__Host-brain-google') cleared.push(value)
            }
            finish(false, { 'set-cookie': cleared, 'clear-site-data': '"storage"' })
          })
        })
        owned.add(outgoing); incoming.once('aborted', cancel); response.once('close', disconnected)
        timer = setTimeout(() => outgoing.destroy(unavailable()), 2000)
        outgoing.once('error', () => finish(true)); outgoing.end()
      } catch { outgoing?.destroy(); finish(true) }
    })
  }
  let proxyBytes = 0
  function proxy(req, res, url, rid, reply, responseLimit = 20 * 1024 * 1024) {
    if (stopped || active >= 8) throw new HttpError(503, 'Backend request capacity is unavailable')
    if (!['GET','HEAD','POST','PUT','PATCH','DELETE','OPTIONS'].includes(req.method)) throw new HttpError(405, 'Method not allowed')
    const limit = url.pathname === '/_brain/uploads' && req.method === 'POST' ? 5 * 1024 * 1024 + 65536 : 65536
    if (Number(req.headers['content-length'] || 0) > limit) throw new HttpError(413, 'Request body is too large')
    active++
    return new Promise((resolve, reject) => {
      let outgoing, timer, done = false, bytes = 0, requestBytes = 0, failure
      const chunks = []
      const headers = { 'x-request-id':rid }
      for (const key of ['origin','authorization','content-type','content-length','x-brain-rpc','accept','x-brain-webhook-id','x-brain-webhook-timestamp','x-brain-webhook-signature','user-agent','dnt','sec-gpc','cf-ipcountry']) {
        const value = req.headers[key]
        if (typeof value === 'string' && value.length <= 8192) headers[key] = value
      }
      const selected = []
      for (const part of String(req.headers.cookie || '').split(';')) {
        const name = part.trim().split('=')[0]
        if (['__Host-brain','__Host-brain-access','__Host-brain-refresh','__Host-brain-google'].includes(name)) selected.push(part.trim())
      }
      if (selected.length) headers.cookie = selected.join('; ')
      const finish = (error, backend) => {
        if (done) return
        done = true; clearTimeout(timer); active--; proxyBytes -= bytes; owned.delete(outgoing)
        req.removeListener('aborted',cancel); req.removeListener('data',data); req.removeListener('end',end)
        res.removeListener('close',cancel)
        if (error) { if (!res.destroyed) reject(failure || new HttpError(503,'Backend request unavailable')); else resolve(); return }
        const safe = { 'cache-control':'no-store' }
        for (const key of ['content-type','content-disposition','allow','retry-after','www-authenticate','location','etag','last-modified','clear-site-data']) {
          const value = backend.headers[key]
          if (typeof value === 'string' && value.length <= 8192) safe[key] = value
        }
        const cookies = (backend.headers['set-cookie'] || []).filter(value => /^(?:__Host-brain(?:-access|-refresh|-google)?)=/.test(value) && value.length <= 8192)
        if (cookies.length) safe['set-cookie'] = cookies.slice(0,8)
        try { reply(res,backend.statusCode || 502,req.method === 'HEAD' ? '' : Buffer.concat(chunks,bytes),safe); resolve() } catch (e) { reject(e) }
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
          backend.once('end',()=>finish(false,backend)); backend.once('error',()=>finish(true)); backend.once('aborted',()=>finish(true))
        })
        owned.add(outgoing)
        timer = setTimeout(()=>{failure = new HttpError(504,'Backend request timed out');outgoing.destroy();finish(true)},url.pathname === '/_brain/uploads' || url.pathname.startsWith('/_brain/uploads/') ? 35000 : 10000)
        outgoing.once('error',()=>finish(true)); req.once('aborted',cancel); res.once('close',cancel)
        req.on('data',data); req.once('end',end)
        if (req.readableEnded) end()
      } catch { outgoing?.destroy(); finish(true) }
    })
  }
  const resource = (incoming, response, rid, name) => {
    if (typeof name !== 'string' || !/^[a-z][a-z0-9_]{0,47}$/.test(name)) throw new HttpError(400, 'Invalid native resource name')
    const idPath = id => {
      if (typeof id !== 'string' || !/^[a-f0-9]{8}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{12}$/i.test(id)) throw new HttpError(400, 'Invalid native record identifier')
      return '/api/' + name + '/' + id
    }
    const request = async (method, path, body) => {
      if (response.writableEnded || response.destroyed) throw new HttpError(409, 'The server request has ended')
      if (method !== 'GET' && incoming.method !== 'POST') throw new HttpError(405, 'Native writes require a server function called through POST')
      let bytes = Buffer.alloc(0)
      if (body !== undefined) {
        if (!body || typeof body !== 'object' || Array.isArray(body)) throw new HttpError(400, 'Native resource values must be an object')
        try { bytes = Buffer.from(JSON.stringify(body)) } catch { throw new HttpError(400, 'Native resource values must be JSON serializable') }
        if (bytes.length > 65536) throw new HttpError(413, 'Native resource values exceed their byte budget')
      }
      const req = Readable.from(bytes.length ? [bytes] : [])
      req.method = method
      req.headers = { cookie: cookies(incoming, false), accept: 'application/json', 'content-type':'application/json', 'content-length':String(bytes.length) }
      if (method !== 'GET') {
        let origin
        try { origin = new URL(incoming.headers.origin); if (origin.origin !== incoming.headers.origin || origin.host !== (incoming.headers['x-forwarded-host'] || incoming.headers.host)) throw Error() } catch { throw new HttpError(403, 'Native writes require a same-origin server request') }
        req.headers.origin = origin.origin
      }
      let value
      try {
        await proxy(req, response, new URL(path, target), rid, (_res, status, data, headers) => {
          if (status < 200 || status >= 300) throw new HttpError(status >= 400 && status < 500 ? status : 503, 'Native resource request failed')
          if (status === 204) { value = null; return }
          if (!String(headers['content-type'] || '').startsWith('application/json')) throw new HttpError(502, 'Native resource response must be JSON')
          try { value = JSON.parse(data.toString('utf8')) } catch { throw new HttpError(502, 'Native resource response is invalid') }
        }, 2 * 1024 * 1024)
      } finally { req.destroy() }
      if (response.destroyed || response.writableEnded) throw new HttpError(409, 'The server request has ended')
      return value
    }
    return Object.freeze({
      list(options = {}) {
        if (!options || typeof options !== 'object' || Array.isArray(options) || Object.keys(options).some(key => !['search','limit','offset'].includes(key))) throw new HttpError(400, 'Invalid native list options')
        const {search = '', limit = 20, offset = 0} = options
        if (typeof search !== 'string' || Buffer.byteLength(search) > 256 || !Number.isInteger(limit) || limit < 1 || limit > 100 || !Number.isInteger(offset) || offset < 0 || offset > 10000) throw new HttpError(400, 'Invalid native list pagination or search')
        return request('GET', '/api/' + name + '?' + new URLSearchParams({search,limit:String(limit),offset:String(offset)}))
      },
      get: id => request('GET', idPath(id)),
      create: values => request('POST', '/api/' + name, values),
      update: (id, values) => request('PUT', idPath(id), values),
      remove: id => request('DELETE', idPath(id)),
    })
  }
  async function login(incoming,response,rid,credentials) {
    if (incoming.method !== 'POST' || response.headersSent || response.writableEnded || response.destroyed) throw new HttpError(403,'Sign in through a same-origin POST server function')
    if (incoming.$nativeLoginStarted || incoming.$nativeLogout) throw new HttpError(409,'Use one authentication change per server request')
    if (!credentials || typeof credentials !== 'object' || Array.isArray(credentials) || Object.keys(credentials).some(key=>!['email','password'].includes(key)) || typeof credentials.email !== 'string' || typeof credentials.password !== 'string' || Buffer.byteLength(credentials.email)>254 || Buffer.byteLength(credentials.password)>1024) throw new HttpError(422,'Enter a valid email and password')
    let origin
    try { origin=new URL(incoming.headers.origin);if(origin.origin!==incoming.headers.origin || origin.host!==(incoming.headers['x-forwarded-host']||incoming.headers.host))throw Error() }catch{throw new HttpError(403,'Sign in through a same-origin POST server function')}
    incoming.$nativeLoginStarted=true
    const bytes=Buffer.from(JSON.stringify({email:credentials.email,password:credentials.password}))
    const request=Readable.from([bytes]);request.method='POST';request.headers={origin:origin.origin,accept:'application/json','content-type':'application/json','content-length':String(bytes.length)}
    let user
    try {
      await proxy(request,response,new URL('/_brain/auth/login',target),rid,(_res,status,data,headers)=>{
        if(status!==200)throw new HttpError(status===400||status===401||status===422?401:status===429?429:503,status===429?'Too many attempts. Wait before trying again.':status<500?'Wrong email or password':'Sign-in is temporarily unavailable')
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
  return {
    proxy,
    resource,
    login,
    async prepare(req, res, rid) {
      if (!cookies(req, false)) { req.$nativeUser = null; req.$nativeAuth ||= Promise.resolve(); return }
      if (!req.$nativeAuth) req.$nativeAuth = call(req, res, '/_brain/auth/me', 'GET', rid).then(result => { req.$nativeUser = result.user })
      await req.$nativeAuth
    },
    session(req, res, rid) {
      return {
        user: req.$nativeUser,
        set() { throw new HttpError(409, 'Use the native authentication API to establish a session') },
        clear() {
          if (req.$nativeLoginStarted) throw new HttpError(409,'Use a separate server request to sign out after signing in')
          if (req.method !== 'POST' || res.headersSent) throw new HttpError(403, 'Sign out through a same-origin POST request')
          let origin
          try { origin = new URL(req.headers.origin); if (origin.origin !== req.headers.origin || origin.host !== (req.headers['x-forwarded-host'] || req.headers.host)) throw Error() } catch { throw new HttpError(403, 'Sign out through a same-origin POST request') }
          if (!req.$nativeLogout) {
            req.$nativeLogout = call(req, res, '/_brain/auth/logout', 'POST', rid, origin.origin).then(headers => { req.$nativeLogoutDone = true; req.$nativeLogoutHeaders = headers }, error => { req.$nativeLogoutDone = true; req.$nativeLogoutError = true; throw error })
            req.$nativeLogout.catch(() => {})
          }
          return req.$nativeLogout
        },
        headers: () => req.$nativeLoginHeaders || {},
      }
    },
    _socketCookie: req => cookies(req, false),
    close() { stopped = true; for (const request of owned) request.destroy(); agent.destroy() },
  }
}

export function createServer(o = {}) {
  const dist = resolve(o.dist || process.cwd())
  const env = Object.freeze({ ...(o.env || ENV) }) // ctx.env: what the supervisor passed, read-only
  const native = nativeAuthBridge(o.backendURL ?? env.BRAIN_BACKEND_URL)
  if (o.nativeSockets !== undefined && typeof o.nativeSockets !== 'boolean') throw Error('nativeSockets must be an explicit boolean')
  const TIMEOUT = +(o.timeout ?? env.BRAIN_TIMEOUT_MS ?? 10000)
  const LIMIT = +(o.rateLimit ?? env.BRAIN_RPC_LIMIT ?? 60) // per IP + function, per minute
  const MAX_BODY = 64 * 1024
  const secret = o.secret ?? env.BRAIN_SECRET ?? ''
  const sink = o.log || ((level, msg, extra) => console[level === 'error' ? 'error' : 'log'](json({ t: new Date().toISOString(), level, msg, ...extra })))
  const log = (level, msg, extra = {}) => {
    const safe = { ...extra }
    // Error messages may contain credentials or request data. Retain only bounded V8 source frames.
    if (typeof safe.stack === 'string') {
      const kind = /^(TypeError|ReferenceError|RangeError|SyntaxError|AggregateError|Error):/.exec(safe.stack)?.[1] || 'Error'
      safe.errorType = kind
      safe.code = kind === 'Error' ? 'NODE_INTERNAL_ERROR' : 'NODE_' + kind.replace(/Error$/, '').toUpperCase() + '_ERROR'
      const truncated = safe.stack.length > 32768 || safe.stack.split('\n').length > 41
      safe.traceTruncated = truncated
      safe.stack = safe.stack.slice(0, 32768).split('\n').filter(line => /^\s+at (?:async )?(?:[A-Za-z0-9_.$<>]+(?: \[as [A-Za-z0-9_$]+\])? \()?(?:file:\/\/\/|\/|node:)[^\s()?#]*:\d+:\d+\)?$/.test(line) && line.length <= 1024).slice(0, 40).join('\n')
      safe.private = true; safe.valuesCaptured = false
      safe.fix = kind === 'TypeError' ? 'Check nullable values, argument types and the data contract at these source frames' : kind === 'ReferenceError' ? 'Check the missing binding or import at these source frames' : kind === 'RangeError' ? 'Check recursive calls, array sizes and configured resource budgets at these source frames' : kind === 'SyntaxError' ? 'Check module syntax or JSON input parsing at these source frames without logging input values' : 'Inspect these source frames and the request reference; reproduce with synthetic inputs'
    }
    if (typeof safe.path === 'string') safe.path = safe.path.split(/[?#]/, 1)[0].slice(0, 2048)
    sink(level, msg, safe)
  }
  const mf = readManifest(dist)
  const hasManifestSockets = Object.hasOwn(mf,'nativeSockets')
  if (hasManifestSockets && typeof mf.nativeSockets !== 'boolean') throw Error('Manifest nativeSockets must be an explicit boolean')
  const nativeSockets = o.nativeSockets === undefined ? hasManifestSockets && mf.nativeSockets : o.nativeSockets
  if (nativeSockets && !native) throw Error('Native sockets require a configured native backend')
  // the build's single route table (#187): already in match order, each with its RegExp source (named groups = params)
  const routes = mf.routes.map(r => ({ ...r, rx: r.rx ? new RegExp(r.rx) : exact(r.path) }))
  // the real path: Node imports file URLs through realpath, and the /_b/ hook must name the same modules (a symlinked
  // dist such as /var → /private/var on macOS would otherwise load the core twice)
  const distURL = pathToFileURL(real(dist) + sep).href
  // Base path (§23 #148): the app lives under mf.base ("/app/"); routes, files and functions are matched on the app
  // path (base stripped); redirects the app gives as app paths get it back.
  const BASE = mf.base || '/'
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
  hook(distURL, BASE)

  // ---------- helpers ----------
  const send = (res, status, body, headers = {}, configuredExternal = false) => {
    if (native && res.req.$nativeLogout && !res.req.$nativeLogoutDone) {
      res.req.$nativeLogout.then(() => send(res, status, body, headers, configuredExternal), () => send(res, 503, '', {}, false))
      return
    }
    if (native && res.req.$nativeLogoutError) { status = 503; body = ''; headers = {}; res.req.$nativeLogoutError = false }
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
    res.writeHead(status, { ...SECURITY, ...(server.draining ? { connection: 'close' } : {}), ...headers })
    res.end(res.req.method === 'HEAD' || status === 204 || status === 205 || status === 304 ? undefined : body)
  }
  // HTML: `no-cache` + ETag (strong for files, weak for SSR) → a repeat visit gets 304 with no body (§35); the ETag is
  // taken before the per-response nonce is injected, so it is stable
  const html = (res, status, page, headers = {}, etag) => {
    if (etag && status === 200) {
      headers = { etag, ...headers }
      const inm = res.req.headers['if-none-match']
      if (inm && inm.split(',').some(t => t.trim().replace(/^W\//, '') === etag.replace(/^W\//, ''))) return send(res, 304, '', headers)
    }
    const nonce = randomBytes(16).toString('base64')
    const body = page.replace(/<script(?![^>]*\bnonce=)/g, `<script nonce="${nonce}"`)
    send(res, status, body, { 'content-type': MIME['.html'], 'content-security-policy': withNonce(mf.csp || CSP0, nonce), 'cache-control': 'no-cache', ...(googleSignInPage(page) ? { 'cross-origin-opener-policy': 'same-origin-allow-popups' } : {}), ...(mf.build ? { 'x-brain-build': mf.build } : {}), ...headers })
  }
  const notFound = (req, res) => failPage(res, 404, res.getHeader('x-request-id'))
  const failPage = (res, status, rid) => {
    if (!wantsHtml(res.req)) return send(res, status, json({ schemaVersion: 1, status, rid, error: STATUS_CODES[status] || 'Request failed' }), { 'content-type': MIME['.json'] })
    const custom = (status === 404 && mf.notFound) || (mf.errors && mf.errors[status]) || `${status}.html`
    const path = typeof custom === 'string' && file(custom)
    return html(res, status, path ? readFileSync(path, 'utf8') : defaultError(status, rid, BASE))
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

  const context = (req, res, url, extra) => {
    const cookies = parseCookies(req.headers.cookie)
    const { user, session: ses } = native ? { user: req.$nativeUser ?? null, session: native.session(req, res, res.getHeader('x-request-id')) } : sessionFrom(cookies, secret)
    return {
      url, query: Object.fromEntries(url.searchParams), cookies, user, session: ses, env,
      ...(native ? { backend: Object.freeze({resource: name => native.resource(req, res, res.getHeader('x-request-id'), name),login:credentials=>native.login(req,res,res.getHeader('x-request-id'),credentials)}) } : {}),
      // `prefetch`: the router fetched this page ahead of a click (§19); a `load` with side effects can skip them
      prefetch: req.headers['x-brain-prefetch'] === '1',
      req: { method: req.method, headers: req.headers, ip: ipOf(req) }, ...extra,
    }
  }

  // ---------- SSR ----------
  let queue = Promise.resolve() // the DOM shim is global: renders run one at a time (load runs concurrently)
  // forms: { '<hash>/<fn>': { values, errors, error } } — the state of a form posted without JS, re-rendered (§38)
  // `fixed`: render this route with these props (the 403 and _error pages): no guards, no load
  // `<meta cache="N">` (route.cache, #235): the SSR HTML of an anonymous GET is kept N s per URL (and language) and
  // served again; between N and 2N s the copy is served while a fresh one renders in the background
  // (stale-while-revalidate). Never for requests with a session cookie or Authorization, forms or error pages.
  const SSRC = new Map(), SSRC_MAX = 500, SSRC_BYTES = 32 * 1024 * 1024, SSRC_ENTRY = 1024 * 1024
  let ssrBytes = 0
  async function cached(req, res, route, params, url, rid) {
    const n = +route.cache, ms = n * 1000, key = (route.locale || '') + url.pathname + url.search
    const fill = () =>
      new Promise(ok => {
        const cap = { req, $capture: (out, h, st) => {
          const headers = Object.fromEntries(Object.entries(h).map(([k, v]) => [k.toLowerCase(), v]))
          const privateResponse = headers['set-cookie'] || headers['vary'] || /private|no-store|no-cache/i.test(headers['cache-control'] || '')
          ok(st === 200 ? { out, h, cacheable: !privateResponse, etag: 'W/"' + tag(out) + '"', t: Date.now(), bytes: Buffer.byteLength(out) } : null)
        }, writeHead() {}, end: () => ok(null), setHeader() {}, getHeader() {} }
        ssr(req, cap, route, params, url, rid, null, 200, null, true).catch(() => ok(null))
      }).then(c => {
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
    const serve = c => html(res, 200, c.out, { ...c.h, 'cache-control': c.cacheable ? `public, s-maxage=${n}, stale-while-revalidate=${n}` : 'no-store' }, c.etag)
    let c = SSRC.get(key)
    const age = c ? Date.now() - c.t : Infinity
    if (c && age < 2 * ms) {
      if (age >= ms && !c.busy) (c.busy = true), fill().finally(() => (c.busy = false))
      return serve(c)
    }
    c = await fill()
    return c ? serve(c) : ssr(req, res, route, params, url, rid, null, 200, null, true) // a redirect or an error: as usual
  }
  const anonymous = req => req.method === 'GET' && !req.headers.authorization && !req.headers.cookie

  async function ssr(req, res, route, params, url, rid, forms = null, status = 200, fixed = null, nocache = false) {
    // 103 Early Hints (#232): the route's preloads (the same Link values as _headers) before any work, once
    if (route.links && route.links.length && res.writeEarlyHints && !res.$capture && !res.$hinted) {
      res.$hinted = true
      try {
        res.writeEarlyHints({ link: route.links })
      } catch {}
    }
    if (!nocache && route.cache > 0 && !forms && !fixed && status === 200 && anonymous(req)) return cached(req, res, route, params, url, rid)
    if (native) await native.prepare(req, res, rid)
    const ctx = context(req, res, url, { params })
    // a page rendered for a signed-in user is never stored (§35): after logout, Back must not show the previous
    // user's page from the HTTP cache or bfcache. Anonymous pages keep `no-cache` + ETag.
    const own = ctx.user != null || ctx.cookies[COOKIE] !== undefined || native && /(?:^|;\s*)__Host-brain(?:-access|-refresh)?=/.test(req.headers.cookie || '')
    const keep = h => (own ? { ...h, 'cache-control': 'private, no-store' } : h)
    let props = fixed || {}, extra = {}
    if (!fixed && ((route.guards && route.guards.length) || (route.middleware && route.middleware.length))) {
      let r
      try {
        const guards = [], middleware = []
        for (const g of route.guards || []) guards.push(await load(g))
        for (const m of route.middleware || []) middleware.push(await load(m))
        r = await runChain({ guards, middleware, ctx })
      } catch (e) {
        log('error', 'guard failed', { rid, path: url.pathname, stack: String((e && e.stack) || e) })
        return errorPage(req, res, route, params, url, rid)
      }
      if (r.redirect) return send(res, r.status, '', { location: withBase(redirect(r.redirect, r.status ?? 303).redirect), 'cache-control': 'no-store', ...ctx.session.headers() })
      if (r.deny !== undefined) return forbidden(req, res, url, rid, r.deny)
      extra = r.headers
    }
    if (route.stream && route.server && !fixed && !forms && status === 200 && !res.$capture) return streamed(req, res, route, params, url, rid, ctx, keep(extra || {}))
    try {
      if (!fixed && route.server) {
        const m = await load(route.server)
        if (typeof m.load === 'function') props = (await withTimeout(Promise.resolve(m.load(ctx)), TIMEOUT)) ?? {}
      }
    } catch (e) {
      if (e && e.redirect) return send(res, e.status || 303, '', { location: withBase(redirect(e.redirect, e.status ?? 303).redirect), ...ctx.session.headers() })
      if ((e instanceof HttpError && e.status === 404) || (e && e.notFound === true)) return notFound(req, res) // `throw notFound()` (F11)
      const status = intendedStatus(e) || 500
      log('error', 'load failed', { rid, path: url.pathname, status, stack: String((e && e.stack) || e) })
      return status === 500 ? errorPage(req, res, route, params, url, rid) : failPage(res, status, rid)
    }
    if (props && props.redirect) return send(res, props.status || 303, '', { location: withBase(redirect(props.redirect, props.status ?? 303).redirect), ...ctx.session.headers() })
    if (props && props.notFound === true) return notFound(req, res) // `return notFound()` (F11)
    let body, seo, failed
    try {
      const at = { path: url.pathname, params: params || {}, query: Object.fromEntries(url.searchParams) } // route() (§41.5)
      ;({ body, seo, failed } = await (queue = queue.then(() => render(route, props, forms, at), () => render(route, props, forms, at))))
    } catch (e) {
      log('error', 'render failed', { rid, path: url.pathname, stack: String((e && e.stack) || e) })
      return failPage(res, 500, rid)
    }
    // a component that failed shows its fallback inside the page (still 200); the error goes to the log with the rid
    for (const f of failed || []) log('error', 'component failed during SSR', { rid, path: url.pathname, comp: f.comp, where: f.w, stack: String((f.e && f.e.stack) || f.e) })
    const page = readFileSync(file(route.page), 'utf8')
      .replace('<!--brain:ssr-->', () => body)
      .replace('<!--brain:props-->', () => jsonInHtml(props))
    let out = head(page, seo, url.pathname)
    // route params for route() in the browser (the client has no route table)
    if (params && Object.keys(params).length) out = out.replace('</head>', () => `<meta name="brain-params" content="${esc(JSON.stringify(params))}"></head>`)
    if (forms) out = out.replace('</body>', () => `<script type="application/json" id="brain-forms">${jsonInHtml(forms)}</script></body>`)
    if (res.$capture) return res.$capture(out, keep({ ...extra, ...ctx.session.headers() }), status)
    html(res, status, out, keep({ ...extra, ...ctx.session.headers() }), own ? undefined : 'W/"' + tag(out) + '"')
  }
  // Streaming SSR (#232, `route.stream`: the build sets it when the view's <meta seo> is static and it has a load()):
  // the page up to the region (head, CSS, layout shell) is sent at once, the view when load() ends. The status and
  // headers are already out by then, so a redirect from load() becomes location.replace and a failure an inline notice
  // with the request id (both logged); load() cannot change the session on a streamed route.
  async function streamed(req, res, route, params, url, rid, ctx, extra) {
    const [top, bottom = ''] = readFileSync(file(route.page), 'utf8').split('<!--brain:ssr-->')
    const sm = await load(route.server)
    const seoF = sm.seo || (await load(route.view)).default.seo
    if (route.i18n) await load(route.i18n)
    const nonce = randomBytes(16).toString('base64'), n = h => h.replace(/<script(?![^>]*\bnonce=)/g, `<script nonce="${nonce}"`)
    res.writeHead(200, {
      ...SECURITY, 'content-type': MIME['.html'], 'content-security-policy': withNonce(mf.csp || CSP0, nonce), 'cache-control': 'no-cache',
      ...(mf.build ? { 'x-brain-build': mf.build } : {}), ...extra, ...ctx.session.headers(),
    })
    globalThis.__brainLocale = route.locale || null
    let seo0 = null
    try {
      seo0 = typeof seoF === 'function' ? seoF({}) : null
    } finally {
      globalThis.__brainLocale = null
    }
    // load() starts now: the loading view (#253) renders while it runs
    const loadP = typeof sm.load === 'function' ? withTimeout(Promise.resolve().then(() => sm.load(ctx)), TIMEOUT) : Promise.resolve(null)
    loadP.catch(() => {}) // handled below; never an unhandled rejection while the loading view renders
    const at = { path: url.pathname, params: params || {}, query: Object.fromEntries(url.searchParams) }
    res.write(n(head(top, seo0, url.pathname)))
    // QA #253: the folder's _loading.brain goes in the region with the first chunk (SPEC: shows while load runs),
    // between two marker comments; the last chunk starts with a script that removes them and what is between
    let clear = ''
    if (route.loading) {
      try {
        const lr = { ...route, view: route.loading, layouts: [], server: null }
        const { body } = await (queue = queue.then(() => render(lr, {}, null, at), () => render(lr, {}, null, at)))
        res.write(n(`<!--brain:loading-->${body}<!--/brain:loading-->`))
        clear = `<script>{const s=document.currentScript;let x=0;for(const c of[...s.parentNode.childNodes]){if(c.data=='brain:loading')x=1;if(x)c.remove();if(c.data=='/brain:loading')x=0}s.remove()}</script>`
      } catch (e) {
        log('error', 'loading view failed (streamed)', { rid, path: url.pathname, stack: String((e && e.stack) || e) })
      }
    }
    const end = (body, props = {}, b = bottom) => res.end(n(clear + body + b.replace('<!--brain:props-->', () => jsonInHtml(props))))
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
      if (e && e.redirect) return end(`<script>location.replace(${jsonInHtml(withBase(redirect(e.redirect, e.status ?? 303).redirect))})</script>`)
      // a streamed page already sent its 200 head: the region gets the not-found notice and the page is noindex
      if (e && e.notFound === true) return end(`<meta name="robots" content="noindex"><p role="alert" class="brain-fallback">Page not found.</p>`, {}, bottom.replace(/<script type="module">(?:(?!<\/script>)[\s\S])*?brain-view[\s\S]*?<\/script>/g, ''))
      log('error', 'load failed (streamed)', { rid, path: url.pathname, status: 200, stack: String((e && e.stack) || e) }) // headers already out
      return fail()
    }
    try {
      const { body, failed } = await (queue = queue.then(() => render(route, props, null, at), () => render(route, props, null, at)))
      for (const f of failed || []) log('error', 'component failed during SSR', { rid, path: url.pathname, comp: f.comp, where: f.w, stack: String((f.e && f.e.stack) || f.e) })
      const pm = params && Object.keys(params).length ? `<meta name="brain-params" content="${esc(JSON.stringify(params))}">` : ''
      end(pm + body, props)
    } catch (e) {
      log('error', 'render failed (streamed)', { rid, path: url.pathname, status: 200, stack: String((e && e.stack) || e) })
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
    if (!wantsHtml(req)) return failPage(res, 403, rid)
    if (mf.forbidden) return ssr(req, res, mf.forbidden, {}, url, rid, null, 403, { reason })
    const f = file('403.html')
    if (f) return html(res, 403, readFileSync(f, 'utf8'))
    return failPage(res, 403, rid)
  }
  // a failing load or guard → the folder's _error.brain (route `error`, props { message, rid }), else the generic page
  function errorPage(req, res, route, params, url, rid) {
    if (wantsHtml(req) && route.error) return ssr(req, res, route.error, params, url, rid, null, 500, { message: 'Something went wrong.', rid })
    return failPage(res, 500, rid)
  }
  async function render(route, props, forms, at = null) {
    const { install } = await load('_server/dom.js')
    const doc = install()
    const root = doc.createElement('div')
    const core = await load(mf.core)
    const mount = core.mount || core.m
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
    const body = root.innerHTML
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
    if (!sameOrigin(req)) return fail(403, 'Cross-origin request')
    const modPath = mf.rpc[hash]
    if (!modPath || !/^[A-Za-z_$][\w$]*$/.test(fn)) return fail(404, 'Unknown function')
    const ra = overLimit(ipOf(req) + ' ' + hash + '/' + fn, LIMIT) // before reading the body
    if (ra) return fail(429, 'Too many requests', { $h: { 'retry-after': String(ra) } })
    let raw
    try {
      raw = await body(req, MAX_BODY)
    } catch {
      return fail(413, 'Body larger than 64 KB')
    }
    if (native) await native.prepare(req, res, rid)
    const ctx = context(req, res, url, { params: {} })
    let f
    try {
      f = (await load(modPath))[fn]
    } catch (e) {
      log('error', 'rpc module failed to load', { rid, fn: hash + '/' + fn, stack: String((e && e.stack) || e) })
      return fail(500, 'Internal error', { rid })
    }
    const r = await callRpc({ fn: f, args: raw, ctx, timeout: TIMEOUT })
    if (r.cause) log('error', r.status === 504 ? 'rpc timeout' : 'rpc failed', { rid, fn: hash + '/' + fn, ms: TIMEOUT, stack: String((r.cause && r.cause.stack) || r.cause) })
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
    if (!sameOrigin(req, true)) return plain(403, 'Cross-origin form post')
    if (!/^application\/x-www-form-urlencoded/i.test(req.headers['content-type'] || '')) return plain(415, 'Forms are sent as application/x-www-form-urlencoded')
    let raw
    try {
      raw = await body(req, MAX_BODY)
    } catch {
      return plain(413, 'Form larger than 64 KB')
    }
    const id = new URLSearchParams(raw).get('_brain') || '', modPath = mf.rpc[id.split('/')[0]]
    if (!modPath) return plain(400, 'Unknown form')
    if (native) await native.prepare(req, res, rid)
    const ctx = context(req, res, url, { params })
    let module, rules
    try {
      module = await load(modPath)
      rules = await load('_server/brain-form.js')
    } catch (e) {
      log('error', 'form module failed to load', { rid, fn: id, stack: String((e && e.stack) || e) })
      return failPage(res, 500, rid)
    }
    const r = await callForm({ module, raw, ctx, rules, key: ipOf(req) + ' ' + id, limit: LIMIT, timeout: TIMEOUT })
    if (r.cause) log('error', 'form failed', { rid, fn: id, stack: String((r.cause && r.cause.stack) || r.cause) })
    if (r.status === 303) return send(res, 303, '', { location: withBase(r.location), 'cache-control': 'no-store', ...ctx.session.headers() })
    if (r.status === 422) return ssr(req, res, route, params, url, rid, r.state, 422)
    plain(r.status, r.body.error, r.headers)
  }

  // ---------- dynamic sitemap (§32) ----------
  // A dynamic view without paths() can export `sitemap()` (manifest route `sitemap: true`): it returns the params of its
  // pages, plus an optional `lastmod`, at request time. /sitemap.xml is the build's file with those URLs added (a sitemap
  // index gets one more child, /sitemap-dynamic.xml). A file holds at most 50 000 URLs (sitemaps.org): when the build's
  // URLs plus the dynamic ones pass that, /sitemap.xml becomes an index of /sitemap-static.xml (the build's file) and
  // /sitemap-dynamic.xml; dynamic URLs past 50 000 are dropped with a warning. Absolute URLs take the origin of the
  // build's sitemap (brain.json `site`); a build without a sitemap serves none. The list is kept 5 min (one sitemap() run
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
  async function collect(origin) {
    const urls = []
    let dropped = 0
    for (const r of DYN) {
      try {
        const sm = await load(r.server)
        const list = typeof sm.sitemap === 'function' ? await withTimeout(Promise.resolve().then(() => sm.sitemap({ env })), TIMEOUT) : []
        for (const p of Array.isArray(list) ? list : []) {
          const path = fill(r.path, p)
          if (!path) continue
          if (urls.length < MAX_URLS) urls.push({ loc: origin + withBase(path), lastmod: day(p.lastmod) })
          else dropped++
        }
      } catch (e) {
        log('error', 'sitemap() failed', { view: r.view, stack: String((e && e.stack) || e) })
      }
    }
    if (dropped) log('warn', 'sitemap(): more than 50000 URLs, the rest are not listed', { dropped })
    dynList = { at: Date.now(), urls }
    return urls
  }
  async function dynamicUrls(origin) {
    if (dynList && Date.now() - dynList.at < 300000) return dynList.urls
    // concurrent requests after expiry share one run of every sitemap()
    if (!dynRun) dynRun = collect(origin).finally(() => (dynRun = null))
    return dynRun
  }
  async function serveSitemap(req, res, url) {
    const f = file('/sitemap.xml')
    if (!f) return false
    const text = readFileSync(f, 'utf8')
    const origin = [...text.matchAll(/<loc>(https?:\/\/[^<]+)<\/loc>/g)].map(m => URL.canParse(m[1]) && new URL(m[1]).origin).find(Boolean)
    if (!origin) return false
    const index = text.includes('<sitemapindex')
    const urls = await dynamicUrls(origin)
    // a urlset that would pass 50 000 URLs with the dynamic ones is split: index → static file + dynamic file
    const split = !index && (text.match(/<url>/g) || []).length + urls.length > MAX_URLS
    const xml = body => send(res, 200, req.method === 'HEAD' ? '' : body, { 'content-type': MIME['.xml'], 'cache-control': 'public, max-age=300' })
    if (url.pathname === '/sitemap-static.xml') return split ? (xml(text), true) : false
    if (url.pathname === '/sitemap-dynamic.xml' && !index && !split) return false
    const child = p => `  <sitemap><loc>${esc(origin + withBase(p))}</loc></sitemap>\n`
    const set = urls.map(u => `  <url><loc>${esc(u.loc)}</loc>${u.lastmod ? `<lastmod>${esc(u.lastmod)}</lastmod>` : ''}</url>\n`).join('')
    xml(
      url.pathname === '/sitemap-dynamic.xml'
        ? `<?xml version="1.0" encoding="UTF-8"?>\n<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">\n${set}</urlset>\n`
        : index
          ? text.replace('</sitemapindex>', `${child('/sitemap-dynamic.xml')}</sitemapindex>`)
          : split
            ? `<?xml version="1.0" encoding="UTF-8"?>\n<sitemapindex xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">\n${child('/sitemap-static.xml')}${child('/sitemap-dynamic.xml')}</sitemapindex>\n`
            : text.replace('</urlset>', set + '</urlset>'),
    )
    return true
  }

  // ---------- static ----------
  // §33 content negotiation: `Accept` preferring text/markdown gets the page's Markdown twin (`/pricing` → pricing.md,
  // `/` → index.md) when the build published one (`seo.markdown`, default true; never for noindex pages).
  function publicStaticPath(pathname) {
    let path
    try { path = decodeURIComponent(pathname) } catch { return null }
    if (/[\\\x00]/.test(path)) return null
    const clean = normalize(path).replaceAll('\\', '/')
    const rel = clean.replace(/^\/+/, '')
    if (rel.startsWith('_server') || rel.startsWith('_brain') || rel.startsWith('server.mjs') || rel === '_headers' || rel === '_redirects' || clean.split('/').some(part => part.startsWith('.') && part !== '.well-known')) return null
    return path
  }
  function serveTwin(req, res, url) {
    if (!prefersMarkdown(req.headers.accept)) return false
    const path = publicStaticPath(url.pathname)
    if (path === null) return false
    const md = file(mdPath(path))
    if (!md) return false
    send(res, 200, req.method === 'HEAD' ? '' : readFileSync(md), { 'content-type': MIME['.md'], 'cache-control': 'public, max-age=3600', vary: 'accept' })
    return true
  }
  function serveStatic(req, res, url) {
    const p = publicStaticPath(url.pathname)
    if (p === null) return false
    const candidates = p.endsWith('/') ? [p + 'index.html'] : [p, p + '.html', p + '/index.html']
    for (const c of candidates) {
      const f = file(c)
      if (!f) continue
      const ext = extname(f), type = FEEDS[basename(f)] || MIME[ext] || 'application/octet-stream'
      if (ext === '.html') {
        // §33: the page's Markdown twin (`/pricing` → pricing.md) when the client prefers text/markdown
        if (serveTwin(req, res, url)) return true
        const md = file(mdPath(p))
        const page = readFileSync(f, 'utf8')
        // static HTML changes only with a deploy: its ETag is the build signature (content hash without a manifest)
        return html(res, 200, page, md ? { vary: 'accept', 'x-brain-static': '1' } : { 'x-brain-static': '1' }, '"' + (mf.build || tag(page)) + '"'), true
      }
      const headers = {
        'content-type': type,
        // the worker script must be revalidated on every check, or a CDN could keep an old worker for an hour (§35.7)
        'cache-control': p.startsWith('/_b/') ? 'public, max-age=31536000, immutable' : p === '/sw.js' ? 'no-cache' : 'public, max-age=3600',
        vary: 'accept-encoding',
      }
      for (const enc of acceptedEncodings(req.headers['accept-encoding'])) {
        const suffix = enc === 'br' ? '.br' : '.gz'
        const encoded = file(c + suffix)
        if (encoded) {
          send(res, 200, req.method === 'HEAD' ? '' : readFileSync(encoded), { ...headers, 'content-encoding': enc })
          return true
        }
      }
      send(res, 200, req.method === 'HEAD' ? '' : readFileSync(f), headers)
      return true
    }
    return false
  }

  // ---------- dispatch ----------
  const server = http(async (req, res) => {
    const rid = req.headers['x-request-id'] && /^[\w-]{1,64}$/.test(req.headers['x-request-id']) ? req.headers['x-request-id'] : randomUUID()
    res.setHeader('x-request-id', rid)
    const t0 = performance.now()
    res.on('finish', () => {
      const ms = performance.now() - t0
      if (ms > 1000) log('warn', 'slow request', { rid, path: req.$brainPath || String(req.url || '').split('?')[0], status: res.statusCode, ms: Math.round(ms) })
    })
    try {
      const full = new URL(req.url, 'http://' + (req.headers.host || 'localhost'))
      // /_health stays at the root: the supervisor calls it there whatever the base
      if (full.pathname === '/_health' || full.pathname === BASE + '_health') return send(res, 200, json({ ok: true, build: mf.build || null }), { 'content-type': MIME['.json'], 'cache-control': 'no-store' })
      let url = full
      if (BASE !== '/') {
        if (full.pathname === BASE.slice(0, -1)) return send(res, 301, '', { location: BASE + full.search })
        if (!full.pathname.startsWith(BASE)) return notFound(req, res)
        url = new URL('/' + full.pathname.slice(BASE.length) + full.search, full) // the app path
      }
      req.$brainPath = url.pathname
      if (native && /^\/(?:api(?:\/|$)|_brain(?:\/|$)|s\/)/.test(url.pathname)) return await native.proxy(req,res,url,rid,send)
      // redirects.json (manifest `redirects`, brain-seo server_json): exact sources first, then patterns, on the app path;
      // same-site targets get the base, the query is kept when the target has none
      const to = redirectOf(url.pathname)
      if (to && (req.method === 'GET' || req.method === 'HEAD')) {
        const loc = /^https?:\/\//.test(to[0]) ? to[0] : withBase(to[0]) + (to[0].includes('?') ? '' : url.search)
        return send(res, to[1], '', { location: loc, 'cache-control': to[1] === 301 || to[1] === 308 ? 'public, max-age=3600' : 'no-store' }, true)
      }
      const r = /^\/_r\/([\w-]+)\/([\w$]+)$/.exec(url.pathname)
      if (r) return await rpc(req, res, r[1], r[2], url, rid)
      if (url.pathname.startsWith('/_r/')) return send(res, 404, json({ error: 'Unknown function' }), { 'content-type': MIME['.json'] })
      if (url.pathname.startsWith('/_server/')) return notFound(req, res) // server code is never served
      const post = req.method === 'POST'
      if (!post && req.method !== 'GET' && req.method !== 'HEAD') return send(res, 405, 'Method not allowed', { allow: 'GET, HEAD, POST' })
      for (const route of routes) {
        const m = route.rx.exec(url.pathname)
        if (!m) continue
        // a form page without load is prerendered: GET serves that file (build ETag); only its POST needs the server
        if (!post && route.forms && serveStatic(req, res, url)) return
        if (!post && serveTwin(req, res, url)) return
        return await (post ? formPost : ssr)(req, res, route, m.groups ? { ...m.groups } : {}, url, rid)
      }
      if (post) return send(res, 405, 'Method not allowed', { allow: 'GET, HEAD' })
      if (DYN.length && (url.pathname === '/sitemap.xml' || url.pathname === '/sitemap-dynamic.xml' || url.pathname === '/sitemap-static.xml') && (await serveSitemap(req, res, url))) return
      if (serveStatic(req, res, url)) return
      notFound(req, res)
    } catch (e) {
      log('error', 'unhandled', { rid, path: req.url, stack: String((e && e.stack) || e) })
      if (!res.headersSent) failPage(res, intendedStatus(e) || 500, rid)
      else res.end()
    }
  })
  if (native) server.once('close', () => native.close())
  if (native && nativeSockets) server.$brainSocketStop = installNativeSockets(server,{target:o.backendURL ?? env.BRAIN_BACKEND_URL,base:BASE,cookie:native._socketCookie,log})
  server.headersTimeout = 15000
  server.requestTimeout = 30000
  return server
}

// ---------- session (signed cookie; shared with the dev server) ----------
const COOKIE = '__Host-session'
// The user of a request: the signed `__Host-session` cookie (HMAC-SHA256 with BRAIN_SECRET), plus the session handle
// to set/clear it. Same in `node server.mjs` and in `brain run` (the dev sidecar imports it).
export function sessionFrom(cookies, secret) {
  const sign = data => b64u(createHmac('sha256', secret).update(data).digest())
  let cur = null
  const v = cookies && cookies[COOKIE]
  if (v && secret) {
    const i = v.lastIndexOf('.')
    if (i > 0) {
      const data = v.slice(0, i), mac = Buffer.from(v.slice(i + 1)), good = Buffer.from(sign(data))
      if (mac.length === good.length && timingSafeEqual(mac, good))
        try {
          const s = JSON.parse(Buffer.from(data, 'base64url').toString())
          if (s && s.exp > Date.now()) cur = s
        } catch {}
    }
  }
  const set = []
  let cleared = false
  const cookie = (value, maxAge) => `${COOKIE}=${value}; Path=/; HttpOnly; Secure; SameSite=Lax; Max-Age=${maxAge}`
  const user = cur ? cur.u : null
  return {
    user,
    session: {
      user,
      set(u, { maxAge = 7 * 24 * 3600 } = {}) {
        if (!secret) throw new Error('BRAIN_SECRET is not set: sessions need it (32+ random bytes in .brain_env)')
        const data = b64u(JSON.stringify({ u, exp: Date.now() + maxAge * 1000 }))
        set.push(cookie(`${data}.${sign(data)}`, maxAge))
      },
      clear() {
        set.push(cookie('', 0))
        cleared = true
      },
      // logout wipes this origin's storage on the device: localStorage, IndexedDB, the Cache API and service workers
      // (§35: the next user sees nothing). Not "cache": Chrome clears the whole HTTP cache before it finishes the
      // response, so the logout call took ~18 s (F16). Nothing private is in that cache: RPC is no-store and SSR
      // HTML rendered with a session cookie is `private, no-store` (ssr() → keep).
      headers: () => ({ ...(set.length ? { 'set-cookie': set } : {}), ...(cleared ? { 'clear-site-data': '"storage"' } : {}) }),
    },
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
const HITS = new Map() // `${key}` → [windowStart, count]
let sweeping = null
// Fixed 60 s window per key (ip + function). Returns 0, or the seconds to wait (Retry-After).
export function overLimit(key, max = 60) {
  if (!sweeping)
    (sweeping = setInterval(() => {
      const now = Date.now()
      for (const [k, [t]] of HITS) if (now - t > 60000) HITS.delete(k)
    }, 60000)).unref()
  const now = Date.now()
  let h = HITS.get(key)
  if (!h || now - h[0] > 60000) HITS.set(key, (h = [now, 0]))
  return ++h[1] > max ? Math.ceil((h[0] + 60000 - now) / 1000) : 0
}
const withTimeout = (p, ms) => {
  let t
  return Promise.race([p, new Promise((_, rej) => (t = setTimeout(() => rej(new HttpError(504, 'timeout')), ms)))]).finally(() => clearTimeout(t))
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
  if (key) {
    const ra = overLimit(key, limit)
    if (ra) return { status: 429, body: { error: 'Too many requests' }, headers: { 'retry-after': String(ra) } }
  }
  if (typeof fn !== 'function') return { status: 404, body: { error: 'Unknown function' } }
  let a
  try {
    a = typeof args === 'string' || args == null ? parseArgs(args || '') : args
  } catch (e) {
    return { status: e.status || 400, body: { error: e.message } }
  }
  try {
    const v = await withTimeout(Promise.resolve(fn(ctx, ...a)), timeout)
    return { status: 200, body: v ?? null }
  } catch (e) {
    if (e && e.errors && typeof e.errors === 'object') return { status: 422, body: { error: e.message || 'Invalid input', errors: e.errors } }
    if (e instanceof HttpError && e.status === 504) return { status: 504, body: { error: 'Timeout' }, cause: e }
    const deliberate = intendedStatus(e)
    if (deliberate && deliberate < 500) return { status: deliberate, body: { error: e instanceof HttpError ? e.message : STATUS_CODES[deliberate] } }
    if (deliberate) return {status:deliberate,body:{error:STATUS_CODES[deliberate]},cause:e}
    return { status: 500, body: { error: 'Internal error' }, cause: e }
  }
}
// A form posted without JS (§38) → { status: 303, location } | { status: 422, state } | { status, body } (400/429).
// `module` exports the function and `$forms`; `rules` = { rule, check } (brain-schema / brain-form). `state` is what the
// page re-renders with: { '<hash>/<fn>': { values, errors, error } }.
export async function callForm({ module, raw, ctx, rules, key, limit = 60, timeout = 10000 }) {
  const q = new URLSearchParams(raw || ''), id = q.get('_brain') || '', fn = id.split('/')[1] || ''
  if (key) {
    const ra = overLimit(key, limit)
    if (ra) return { status: 429, body: { error: 'Too many requests' }, headers: { 'retry-after': String(ra) } }
  }
  if (!/^[A-Za-z_$][\w$]*$/.test(fn) || typeof (module && module[fn]) !== 'function') return { status: 400, body: { error: 'Unknown form' } }
  const schema = module.$forms && module.$forms[fn]
  const { values, errors } = schema ? coerce(rules, schema, q) : { values: Object.fromEntries([...q].filter(([k]) => k !== '_brain')), errors: {} }
  const again = (errs, error, cause) => ({ status: 422, state: { [id]: { values, errors: errs, error } }, cause })
  if (Object.keys(errors).length) return again(errors, '')
  let r
  try {
    r = await withTimeout(Promise.resolve(module[fn](ctx, values)), timeout)
  } catch (e) {
    if (e && e.errors && typeof e.errors === 'object') return again(e.errors, '')
    if (e instanceof HttpError && e.status >= 400 && e.status < 500) return again({}, e.message)
    const deliberate = intendedStatus(e)
    if (deliberate && deliberate < 500) return {status:deliberate,body:{error:STATUS_CODES[deliberate]}}
    if (deliberate) return {status:deliberate,body:{error:STATUS_CODES[deliberate]},cause:e}
    return again({}, 'Something went wrong. Try again.', e)
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
      if (k && !(k in out)) out[k] = decodeURIComponent(part.slice(i + 1).trim())
    }
  }
  return out
}
const ipOf = req => String(req.headers['x-forwarded-for'] || '').split(',')[0].trim() || req.socket.remoteAddress || '?'
function body(req, max) {
  return new Promise((ok, fail) => {
    let n = 0
    const parts = []
    // past the limit the rest is drained without keeping it (so the 413 reaches the client); far past it, cut
    req.on('data', c => {
      n += c.length
      if (n > 16 * max) req.destroy()
      else if (n <= max) parts.push(c)
    })
    req.on('end', () => (n > max ? fail(new Error('too large')) : ok(Buffer.concat(parts).toString('utf8'))))
    req.on('error', fail)
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
// SEO head from View.seo(props): title, description, canonical, og:*/twitter:*, JSON-LD and the view's <head> block
// F5: the build writes an SSR route's canonical/og:url from its pattern (/jobs/:slug); the request has the real path.
// A view's own <head> tags replace the generated ones with the same name/property (one tag per key, SPEC «SEO»).
function head(page, seo, path) {
  let h = page
  if (path) {
    // the attribute is already escaped HTML: only the request path that replaces a pattern is escaped
    const real = v => v.replace(/^(https?:\/\/[^/]+)?(\/[^"]*)$/, (m, o = '', p) => (/\/[:*]/.test(p) ? o + esc(path) : m))
    h = h.replace(/(<link[^>]*rel=["']canonical["'][^>]*href=")([^"]*)(")/i, (m, a, v, b) => a + real(v) + b)
    h = h.replace(/(<meta[^>]*property=["']og:url["'][^>]*content=")([^"]*)(")/i, (m, a, v, b) => a + real(v) + b)
  }
  if (!seo) return h
  const meta = (attr, key, val) => {
    const tag = `<meta ${attr}="${esc(key)}" content="${esc(val)}">`
    const rx = new RegExp(`<meta[^>]*\\b${attr}=["']${key.replace(/[.*+?^${}()|[\]\\:]/g, '\\$&')}["'][^>]*>`, 'i')
    h = rx.test(h) ? h.replace(rx, () => tag) : h.replace('</head>', () => tag + '</head>')
  }
  for (const [k, v] of Object.entries(seo)) {
    // `schema` becomes JSON-LD below and `lastmod` feeds it: neither is a meta tag (prerender seo_head does the same)
    if (v == null || k === '$head' || k === 'schema' || k === 'lastmod') continue
    if (k === 'title') h = /<title>[\s\S]*?<\/title>/i.test(h) ? h.replace(/<title>[\s\S]*?<\/title>/i, () => `<title>${esc(v)}</title>`) : h.replace('</head>', () => `<title>${esc(v)}</title></head>`)
    else if (k === 'canonical') {
      const tag = `<link rel="canonical" href="${esc(v)}">`
      h = /<link[^>]*rel=["']canonical["'][^>]*>/i.test(h) ? h.replace(/<link[^>]*rel=["']canonical["'][^>]*>/i, () => tag) : h.replace('</head>', () => tag + '</head>')
      if (seo['og:url'] == null) meta('property', 'og:url', v) // og:url follows the canonical unless the view sets its own
    } else if (k === 'jsonld') h = h.replace('</head>', () => `<script type="application/ld+json">${jsonInHtml(v)}</script></head>`)
    else if (/^(og|twitter|article):/.test(k)) meta('property', k, v)
    else if (typeof v === 'string') meta('name', k, v)
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
    if (seo['og:image'] != null) node.image = String(seo['og:image']).replace(/^\/(?!\/)/, () => (canon && /^https?:\/\/[^/]+/.exec(node.url) || [''])[0] + '/')
    if (seo.lastmod != null) node.dateModified = String(seo.lastmod)
    const own = new RegExp(`<script[^>]*type=["']application/ld\\+json["'][^>]*>\\s*\\{"@context":"https://schema\\.org","@type":"${T}"[\\s\\S]*?</script>`, 'i')
    const tag = `<script type="application/ld+json">${jsonInHtml(node)}</script>`
    h = own.test(h) ? h.replace(own, () => tag) : h.replace('</head>', () => tag + '</head>')
  }
  if (seo.$head) {
    for (const [, attr, key] of seo.$head.matchAll(/<meta[^>]*\b(property|name)=["']([^"']+)["'][^>]*>/gi))
      h = h.replace(new RegExp(`<meta[^>]*\\b${attr}=["']${key.replace(/[.*+?^${}()|[\]\\:]/g, '\\$&')}["'][^>]*>`, 'gi'), '')
    if (/<link[^>]*rel=["']canonical["']/i.test(seo.$head)) h = h.replace(/<link[^>]*rel=["']canonical["'][^>]*>/gi, '')
    h = h.replace('</head>', () => seo.$head + '</head>')
  }
  return h
}
// `/_b/<hash>.js` imports inside the chunks resolve to files in dist/ (same hook as the build's prerender)
let hooked = false
function hook(base, app = '/') {
  if (hooked) return
  hooked = true
  // chunks import '/_b/x.js', or '/app/_b/x.js' under a base path: both are dist/_b/x.js
  const own = s => {
    if (app !== '/' && s.startsWith(app + '_b/')) s = s.slice(app.length - 1)
    return s.startsWith('/_b/') && { url: new URL('.' + s, base).href, shortCircuit: true }
  }
  // registerHooks (Node 22.15+, in-thread) replaces the deprecated register (DEP0205)
  if (Module.registerHooks) return void Module.registerHooks({ resolve: (s, c, next) => own(s) || next(s, c) })
  Module.register(
    'data:text/javascript,' +
      encodeURIComponent(`export async function resolve(s, c, n) { const a = ${json(app)}; if (a !== '/' && s.startsWith(a + '_b/')) s = s.slice(a.length - 1); return s.startsWith('/_b/') ? { url: new URL('.' + s, ${json(base)}).href, shortCircuit: true } : n(s, c) }`)
  )
}

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
  const s = createServer({ dist: fileURLToPath(new URL('.', import.meta.url)) })
  const fd = ENV.BRAIN_LISTEN_FD
  const where = fd ? { fd: Number(fd) } : { port: +(ENV.PORT || 3000), host: ENV.HOST || '127.0.0.1' }
  s.listen(where, () => {
    process.stdout.write(`brain:ready ${process.pid}\n`) // the supervisor waits for this line (§34)
    console.log(json({ t: new Date().toISOString(), level: 'info', msg: 'listening', ...(fd ? { fd: Number(fd) } : where) }))
  })
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
