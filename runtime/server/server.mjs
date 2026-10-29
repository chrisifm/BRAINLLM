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
import { createServer as http } from 'node:http'
import net from 'node:net'
import { createHash, createHmac, randomBytes, randomUUID, timingSafeEqual } from 'node:crypto'
import { existsSync, readFileSync, realpathSync, statSync } from 'node:fs'
import * as Module from 'node:module'
import { basename, extname, join, normalize, resolve, sep } from 'node:path'
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
// `throw redirect('/login')` (or return it) from load/guards; only same-origin paths (open-redirect guard, §31)
export const redirect = (url, status = 303) => {
  const u = String(url)
  if (!u.startsWith('/') || u.startsWith('//') || u.startsWith('/\\')) throw new HttpError(500, 'redirect to another origin is not allowed: ' + u)
  return { redirect: u, status }
}

export function createServer(o = {}) {
  const dist = resolve(o.dist || process.cwd())
  const env = Object.freeze({ ...(o.env || ENV) }) // ctx.env: what the supervisor passed, read-only
  const TIMEOUT = +(o.timeout ?? env.BRAIN_TIMEOUT_MS ?? 10000)
  const LIMIT = +(o.rateLimit ?? env.BRAIN_RPC_LIMIT ?? 60) // per IP + function, per minute
  const MAX_BODY = 64 * 1024
  const secret = o.secret ?? env.BRAIN_SECRET ?? ''
  const log = o.log || ((level, msg, extra) => console[level === 'error' ? 'error' : 'log'](json({ t: new Date().toISOString(), level, msg, ...extra })))
  const mf = readManifest(dist)
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
  const send = (res, status, body, headers = {}) => {
    res.writeHead(status, { ...SECURITY, ...(server.draining ? { connection: 'close' } : {}), ...headers })
    res.end(body)
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
    send(res, status, body, { 'content-type': MIME['.html'], 'content-security-policy': withNonce(mf.csp || CSP0, nonce), 'cache-control': 'no-cache', ...(mf.build ? { 'x-brain-build': mf.build } : {}), ...headers })
  }
  const notFound = (req, res) => {
    const f = mf.notFound && file(mf.notFound)
    if (f) return html(res, 404, readFileSync(f, 'utf8'))
    html(res, 404, '<!doctype html><title>Not found</title><main><h1>Not found</h1></main>')
  }
  const failPage = (res, status, rid) =>
    html(res, status, `<!doctype html><title>Error</title><main><h1>${status === 504 ? 'The page took too long' : 'Something went wrong'}</h1><p>Reference: ${esc(rid)}</p></main>`)
  // a file inside dist/ (never outside: no `..`, no absolute paths)
  function file(rel) {
    const p = normalize(join(dist, rel))
    if (p !== dist && !p.startsWith(dist + sep)) return null
    try {
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
    const { user, session: ses } = sessionFrom(cookies, secret)
    return {
      url, query: Object.fromEntries(url.searchParams), cookies, user, session: ses, env,
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
    const ctx = context(req, res, url, { params })
    // a page rendered for a signed-in user is never stored (§35): after logout, Back must not show the previous
    // user's page from the HTTP cache or bfcache. Anonymous pages keep `no-cache` + ETag.
    const own = ctx.user != null || ctx.cookies[COOKIE] !== undefined
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
      if (r.redirect) return send(res, r.status, '', { location: withBase(r.redirect), 'cache-control': 'no-store', ...ctx.session.headers() })
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
      if (e && e.redirect) return send(res, e.status || 303, '', { location: withBase(e.redirect), ...ctx.session.headers() })
      if ((e instanceof HttpError && e.status === 404) || (e && e.notFound === true)) return notFound(req, res) // `throw notFound()` (F11)
      const status = e instanceof HttpError && e.status === 504 ? 504 : 500
      log('error', 'load failed', { rid, path: url.pathname, status, stack: String((e && e.stack) || e) })
      return status === 500 ? errorPage(req, res, route, params, url, rid) : failPage(res, status, rid)
    }
    if (props && props.redirect) return send(res, props.status || 303, '', { location: withBase(props.redirect), ...ctx.session.headers() })
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
      if (e && e.redirect) return end(`<script>location.replace(${jsonInHtml(withBase(e.redirect))})</script>`)
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
    if (mf.forbidden) return ssr(req, res, mf.forbidden, {}, url, rid, null, 403, { reason })
    const f = file('403.html')
    if (f) return html(res, 403, readFileSync(f, 'utf8'))
    send(res, 403, 'Forbidden', { 'content-type': MIME['.txt'] })
  }
  // a failing load or guard → the folder's _error.brain (route `error`, props { message, rid }), else the generic page
  function errorPage(req, res, route, params, url, rid) {
    if (route.error) return ssr(req, res, route.error, params, url, rid, null, 500, { message: 'Something went wrong.', rid })
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
  // index gets one more child, /sitemap-dynamic.xml). Absolute URLs take the origin of the build's sitemap (brain.json
  // `site`); a build without a sitemap serves none. The list is kept 5 min; a sitemap() that throws is logged and skipped.
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
  async function dynamicUrls(origin) {
    if (dynList && Date.now() - dynList.at < 300000) return dynList.urls
    const urls = []
    for (const r of DYN) {
      try {
        const sm = await load(r.server)
        const list = typeof sm.sitemap === 'function' ? await withTimeout(Promise.resolve().then(() => sm.sitemap({ env })), TIMEOUT) : []
        for (const p of Array.isArray(list) ? list : []) {
          const path = fill(r.path, p)
          if (path && urls.length < 50000) urls.push({ loc: origin + withBase(path), lastmod: day(p.lastmod) })
        }
      } catch (e) {
        log('error', 'sitemap() failed', { view: r.view, stack: String((e && e.stack) || e) })
      }
    }
    dynList = { at: Date.now(), urls }
    return urls
  }
  async function serveSitemap(req, res, url) {
    const f = file('/sitemap.xml')
    if (!f) return false
    const text = readFileSync(f, 'utf8')
    const origin = [...text.matchAll(/<loc>(https?:\/\/[^<]+)<\/loc>/g)].map(m => URL.canParse(m[1]) && new URL(m[1]).origin).find(Boolean)
    if (!origin) return false
    const index = text.includes('<sitemapindex')
    if (url.pathname === '/sitemap-dynamic.xml' && !index) return false
    const set = (await dynamicUrls(origin)).map(u => `  <url><loc>${esc(u.loc)}</loc>${u.lastmod ? `<lastmod>${esc(u.lastmod)}</lastmod>` : ''}</url>\n`).join('')
    const out = url.pathname === '/sitemap-dynamic.xml'
      ? `<?xml version="1.0" encoding="UTF-8"?>\n<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">\n${set}</urlset>\n`
      : index
        ? text.replace('</sitemapindex>', `  <sitemap><loc>${esc(origin + withBase('/sitemap-dynamic.xml'))}</loc></sitemap>\n</sitemapindex>`)
        : text.replace('</urlset>', set + '</urlset>')
    send(res, 200, req.method === 'HEAD' ? '' : out, { 'content-type': MIME['.xml'], 'cache-control': 'public, max-age=3600' })
    return true
  }

  // ---------- static ----------
  function serveStatic(req, res, url) {
    let p = decodeURIComponent(url.pathname)
    const candidates = p.endsWith('/') ? [p + 'index.html'] : [p, p + '.html', p + '/index.html']
    for (const c of candidates) {
      const f = file(c)
      if (!f) continue
      const ext = extname(f), type = FEEDS[basename(f)] || MIME[ext] || 'application/octet-stream'
      if (ext === '.html') {
        const page = readFileSync(f, 'utf8')
        // static HTML changes only with a deploy: its ETag is the build signature (content hash without a manifest)
        return html(res, 200, page, {}, '"' + (mf.build || tag(page)) + '"'), true
      }
      const headers = {
        'content-type': type,
        // the worker script must be revalidated on every check, or a CDN could keep an old worker for an hour (§35.7)
        'cache-control': p.startsWith('/_b/') ? 'public, max-age=31536000, immutable' : p === '/sw.js' ? 'no-cache' : 'public, max-age=3600',
        vary: 'accept-encoding',
      }
      const ae = String(req.headers['accept-encoding'] || '')
      for (const [enc, suffix] of [['br', '.br'], ['gzip', '.gz']]) {
        if (ae.includes(enc) && existsSync(f + suffix)) {
          send(res, 200, req.method === 'HEAD' ? '' : readFileSync(f + suffix), { ...headers, 'content-encoding': enc })
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
      if (ms > 1000) log('warn', 'slow request', { rid, path: req.url, status: res.statusCode, ms: Math.round(ms) })
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
      // redirects.json (manifest `redirects`, brain-seo server_json): exact sources first, then patterns, on the app path;
      // same-site targets get the base, the query is kept when the target has none
      const to = redirectOf(url.pathname)
      if (to && (req.method === 'GET' || req.method === 'HEAD')) {
        const loc = /^https?:\/\//.test(to[0]) ? to[0] : withBase(to[0]) + (to[0].includes('?') ? '' : url.search)
        return send(res, to[1], '', { location: loc, 'cache-control': to[1] === 301 || to[1] === 308 ? 'public, max-age=3600' : 'no-store' })
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
        return await (post ? formPost : ssr)(req, res, route, m.groups ? { ...m.groups } : {}, url, rid)
      }
      if (post) return send(res, 405, 'Method not allowed', { allow: 'GET, HEAD' })
      if (DYN.length && (url.pathname === '/sitemap.xml' || url.pathname === '/sitemap-dynamic.xml') && (await serveSitemap(req, res, url))) return
      if (serveStatic(req, res, url)) return
      notFound(req, res)
    } catch (e) {
      log('error', 'unhandled', { rid, path: req.url, stack: String((e && e.stack) || e) })
      if (!res.headersSent) failPage(res, 500, rid)
      else res.end()
    }
  })
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
const samePath = u => typeof u === 'string' && u.startsWith('/') && !u.startsWith('//') && !u.startsWith('/\\')
function verdict(v) {
  if (v === false) return { deny: 'Forbidden' }
  if (typeof v === 'string') v = { redirect: v }
  if (!v || typeof v !== 'object') return null
  if (v.redirect !== undefined) {
    if (!samePath(v.redirect)) throw new HttpError(500, 'redirect to another origin is not allowed: ' + v.redirect) // §31 open redirect
    return { redirect: v.redirect, status: v.status || 303 }
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
    if (e instanceof HttpError && e.status >= 400 && e.status < 500) return { status: e.status, body: { error: e.message } }
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
    return again({}, 'Something went wrong. Try again.', e)
  }
  if (r && r.errors) return again(r.errors, '')
  const to = r && typeof r.redirect === 'string' && r.redirect.startsWith('/') && !r.redirect.startsWith('//') ? r.redirect : ctx.url.pathname + ctx.url.search
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
    const one = v => (r.t === 'num' ? (v === '' || v == null ? null : Number(v)) : r.t === 'bool' ? v === 'on' || v === 'true' || v === '1' : v == null ? '' : v)
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
    if (v == null || k === '$head') continue
    if (k === 'title') h = /<title>[\s\S]*?<\/title>/i.test(h) ? h.replace(/<title>[\s\S]*?<\/title>/i, () => `<title>${esc(v)}</title>`) : h.replace('</head>', () => `<title>${esc(v)}</title></head>`)
    else if (k === 'canonical') {
      const tag = `<link rel="canonical" href="${esc(v)}">`
      h = /<link[^>]*rel=["']canonical["'][^>]*>/i.test(h) ? h.replace(/<link[^>]*rel=["']canonical["'][^>]*>/i, () => tag) : h.replace('</head>', () => tag + '</head>')
      if (seo['og:url'] == null) meta('property', 'og:url', v) // og:url follows the canonical unless the view sets its own
    } else if (k === 'jsonld') h = h.replace('</head>', () => `<script type="application/ld+json">${jsonInHtml(v)}</script></head>`)
    else if (/^(og|twitter|article):/.test(k)) meta('property', k, v)
    else if (typeof v === 'string') meta('name', k, v)
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
