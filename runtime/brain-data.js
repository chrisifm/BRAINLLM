// BRAINLLM data module. Raw HTML (INTERNALS §19, B0210): `{@html sanitize(x)}` compiles to `html(el, () => sanitize(x))`.
// `sanitize` only marks the string as reviewed; the actual cleaning happens at insertion: native `Element.setHTML()`
// (Sanitizer API, Chrome 146+ / Firefox 148+) when present, else the minimal allowlist sanitizer below.
// Data with contract (§15, §35.3): `api(path, schema)` client and `resource(fn)` for `{#await}`.
import { sig, fx, batch, untrack, _rt } from './brain.js'
import { rule, check } from './brain-schema.js' // not brain-form.js: fetching data never loads the form module

const SAFE = Symbol('brain.safe')
export const sanitize = s => ({ [SAFE]: String(s == null ? '' : s) })

// Fallback allowlist: formatting, lists, links, images and tables. Everything else is unwrapped (text kept) or, for
// script-like elements, dropped with its content. Attributes: only the listed ones; URLs only http(s), mailto, tel,
// relative or #fragment.
const TAGS = new Set('a abbr b blockquote br caption code col colgroup dd div dl dt em figcaption figure h1 h2 h3 h4 h5 h6 hr i img ins kbd li mark ol p pre q s small span strong sub sup table tbody td tfoot th thead tr u ul'.split(' '))
const DROP = new Set('script style template iframe object embed noscript svg math link meta base frame frameset form input button select textarea'.split(' '))
const ATTRS = new Set('href src alt title width height colspan rowspan lang dir cite datetime start reversed'.split(' '))
const URL_ATTR = new Set(['href', 'src', 'cite'])
const okUrl = v => !/^\s*[a-z][a-z0-9+.-]*:/i.test(v) || /^\s*(https?|mailto|tel):/i.test(v)

export function clean(h) {
  const t = document.createElement('template')
  t.innerHTML = h // inert: nothing runs or loads inside a template
  walk(t.content)
  return t.content
}
function walk(p) {
  for (const n of [...p.childNodes]) {
    if (n.nodeType === 3) continue
    if (n.nodeType !== 1) {
      n.remove() // comments, processing instructions
      continue
    }
    const tag = n.tagName.toLowerCase()
    if (DROP.has(tag)) {
      n.remove()
      continue
    }
    walk(n)
    if (!TAGS.has(tag)) {
      n.replaceWith(...n.childNodes) // unknown wrapper: keep its (already cleaned) content
      continue
    }
    for (const a of [...(n.attributes || [])]) {
      const k = a.name.toLowerCase()
      if (!ATTRS.has(k) || (URL_ATTR.has(k) && !okUrl(a.value))) n.removeAttribute(a.name)
    }
    if (tag === 'a' && n.getAttribute('href')) n.setAttribute('rel', 'noopener noreferrer')
  }
}

// Inserts sanitized HTML as the content of `el`, reactively. A value not produced by `sanitize` is inserted as text.
export function html(el, f) {
  fx(() => {
    const v = f()
    if (!v || typeof v !== 'object' || !(SAFE in v)) return void (el.textContent = v == null ? '' : String(v))
    if (el.setHTML) el.setHTML(v[SAFE])
    else el.replaceChildren(clean(v[SAFE]))
  })
}

// ---------- api / resource ----------
// `const users = api('/users/:id', { get: { out: [User], cache: '10m' }, post: { in: User, out: User } })` →
// `users.get({ id: 7, q: 'x' })` (path params from the argument, the rest as query) · `users.post({ id, ...body })`.
// Every request (§40, §41.4): same-origin credentials, a timeout, JSON with __proto__/constructor/prototype dropped,
// `out` validated (a mismatch is reported with the shape diff), one typed ApiError, safe retries, identical GETs in
// flight shared, a per-origin queue with priorities, offline answers from the cache, `.cancel()`. GET responses are
// cached in memory per URL for the session (stale-while-revalidate, instant on back/forward); `cache: '10m'` also
// keeps them in the Cache API for the next visit (opt-in per endpoint; logout's Clear-Site-Data wipes it); `cache:
// 'no'` turns caching off. A successful write drops the cached GETs of its path and re-runs the resources reading it.
const G = globalThis, MEM = new Map(), I = _rt()
const MEM_BYTES = new Map(), MEM_MAX = 8 * 1024 * 1024, MEM_ENTRY = 512 * 1024
let memBytes = 0, cacheEpoch = 0
const forget = u => { memBytes -= MEM_BYTES.get(u) || 0; MEM_BYTES.delete(u); MEM.delete(u) }
const remember = (u, value) => {
  const size = new TextEncoder().encode(JSON.stringify(value)).length
  forget(u)
  if (size > MEM_ENTRY) return
  MEM.set(u, value); MEM_BYTES.set(u, size); memBytes += size
  while (MEM.size > 256 || memBytes > MEM_MAX) forget(MEM.keys().next().value)
}
// rec(f): runs f and returns the signal writes it made ([sig, previous, …]) so they can be undone. The core has no hook
// for it (#288): the setter of every signal is wrapped only while f runs.
const SP = sig(0).constructor.prototype
const rec = f => {
  const d = Object.getOwnPropertyDescriptor(SP, 'v'), w = []
  Object.defineProperty(SP, 'v', {
    configurable: true,
    get: d.get,
    set(x) {
      if (!this.f && !Object.is(x, this._)) w.push(this, this._)
      d.set.call(this, x)
    },
  })
  try {
    f()
  } finally {
    Object.defineProperty(SP, 'v', d)
  }
  return w
}
let CUR = null // abort signal of the resource() run that is calling an endpoint right now
const ttl = c => {
  const m = /^(\d+)(ms|s|m|h)$/.exec(c || '')
  return m ? m[1] * { ms: 1, s: 1e3, m: 6e4, h: 36e5 }[m[2]] : 0
}
// `cache.data: false` in brain.json → the build emits <meta name="brain-data-cache" content="off">: no memory SWR, no
// Cache API (§35 opt-out)
const dataOff = () => !!(G.document && document.querySelector('meta[name=brain-data-cache][content=off]'))
const reviver = (k, v) => (k === '__proto__' || k === 'constructor' || k === 'prototype' ? undefined : v)

// Every rejection: `kind` 'offline' · 'network' · 'timeout' · 'http' (status, errors on 422, retryAfter in seconds on
// 429/503) · 'schema' (diff); `id` is the server's request id (x-request-id) for support. `e.is(403)`.
// A cancelled request rejects with kind 'aborted' (the author's try/finally runs); framework surfaces ignore it:
// resource()'s {:catch}, the boundary toast, <ErrorState>, reportError and brain_errors.
export class ApiError extends Error {
  constructor(kind, message, x) {
    super(message)
    this.name = 'ApiError'
    this.kind = kind
    this.code = 'api'
    Object.assign(this, x)
  }
  is(status) {
    return this.status === status
  }
}

// App-wide policy, declared once: `<meta http base="/api" timeout="10s" retry="2" login="/login">` in app.brain
// compiles to `http({ … })`; `api/_http.jsbrain` adds headers that depend on state: `http({ headers: () => ({ … }) })`.
const P = { base: '', timeout: 10000, retry: 2, login: null, headers: null, mock: null }
// `mock: fn` (brain run --mock: TOOLCHAIN #266 loads brain-mock.js, dev only): api() answers with
// fn(method, url, def, body) instead of the network
export function http(o = {}) {
  if (o.base !== undefined || o.headers !== undefined || o.mock !== undefined) {
    cacheEpoch++; MEM.clear(); MEM_BYTES.clear(); memBytes = 0
  }
  if (o.mock !== undefined) P.mock = typeof o.mock === 'function' ? o.mock : null
  if (o.base != null) P.base = String(o.base).replace(/\/$/, '')
  if (o.timeout != null) P.timeout = typeof o.timeout === 'number' ? o.timeout : ttl(o.timeout) || P.timeout
  if (o.retry != null) P.retry = +o.retry
  if (o.login !== undefined) P.login = o.login
  if (o.headers !== undefined) P.headers = o.headers
  return P
}

// online(): navigator.onLine + events, as a signal read
const ON = sig(!G.navigator || G.navigator.onLine !== false)
if (G.addEventListener) G.addEventListener('online', () => (ON.v = true)), G.addEventListener('offline', () => (ON.v = false))
export const online = () => ON.v

// a version signal per path: a write bumps it and every resource() that read the path runs again
const VER = new Map()
const root = p => p.split('/:')[0].split('?')[0]
const ver = p => VER.get(p) || (VER.set(p, sig(0)), VER.get(p))

export function api(path, spec) {
  const out = {}
  for (const method of Object.keys(spec)) out[method] = (arg, o) => request(path, method.toUpperCase(), spec[method] || {}, arg || {}, o || {})
  return out
}

function request(path, M, def, arg, o) {
  const used = new Set(), file = /^\[?file\]?$/.test(def.in || '')
  let u = path.replace(/:(\w+)/g, (_, k) => (used.add(k), encodeURIComponent(arg[k] == null ? '' : arg[k])))
  let body
  if (M === 'GET' || M === 'HEAD') {
    const q = new URLSearchParams()
    for (const k in arg) if (!used.has(k) && arg[k] != null) q.set(k, arg[k])
    if (String(q)) u += (u.includes('?') ? '&' : '?') + q
    ver(root(path)).v // tracked by the resource() calling it
  } else if (file) {
    body = new FormData()
    for (const k in arg) if (!used.has(k)) for (const v of [].concat(arg[k])) body.append(k, v)
  } else {
    const b = {}
    for (const k in arg) if (!used.has(k)) b[k] = arg[k]
    const bad = def.in && diff(def.in, b)
    if (bad) return Promise.reject(new ApiError('schema', `${M} ${path}: invalid input: ${bad}`, { diff: bad }))
    body = JSON.stringify(b)
  }
  // Dynamic headers may carry identity. Do not share or retain their responses. Cache the resolved URL, so API
  // base/origin changes cannot reuse another endpoint's data. Servers must mark cookie-auth responses no-store.
  const cacheKey = full(u), epoch = cacheEpoch
  const get = M === 'GET', off = dataOff() || def.cache === 'no' || !!P.headers || !!(G.document && document.cookie), keep = get && !off ? ttl(def.cache) : 0
  const ctl = new AbortController(), up = sig(0)
  // optimistic: () => (list = [...list, item]) — applied now, undone if the request fails
  const undo = o.optimistic ? rec(o.optimistic) : null
  // a call made from an event handler is cancelled when its component unmounts (brain-guard keeps `$c` on its nodes)
  if (!CUR && G.event && G.event.target) for (let el = G.event.target; el; el = el.parentNode) if (el.$c) {
    const c = el.$c
    c.add(ctl)
    ctl.signal.addEventListener('abort', () => c.delete(ctl))
    break
  }
  const sg = o.signal || CUR ? any([ctl.signal, o.signal || CUR]) : ctl.signal
  // action (a user write or an imperative GET) > visible (a resource on screen) > background (prefetch)
  const pri = o.priority === 'low' || o.prefetch ? 2 : CUR && get ? 1 : 0
  let p
  if (P.mock) {
    p = Promise.resolve(P.mock(M, u, def, typeof body === 'string' ? JSON.parse(body) : {}))
  } else if (!ON.v) {
    // offline: GETs answer from the cache (marked stale), writes fail at once
    const err = new ApiError('offline', 'You are offline.')
    p = get && !off && MEM.has(cacheKey) ? Promise.resolve(MEM.get(cacheKey)) : get && keep ? load(cacheKey, keep).then(v => (v === undefined ? Promise.reject(err) : v)) : Promise.reject(err)
    p.stale = get
  } else {
    const go = s => run(u, M, body, s, pri, up)
    p = (get && !off ? shared(M + ' ' + cacheKey + ' ' + epoch, go, sg) : go(sg)).then(answer => {
      const j = answer.value
      const bad = def.out && diff(def.out, j)
      if (bad) {
        const e = new ApiError('schema', `${M} ${path}: the response does not match its schema: ${bad}`, { diff: bad })
        G.reportError && G.reportError(e)
        throw e
      }
      if (get) {
        if (answer.cacheable && epoch === cacheEpoch) {
          if (!off) remember(cacheKey, j)
          if (keep) save(cacheKey, j)
        } else if (!answer.cacheable) {
          forget(cacheKey)
          dropStored(cacheKey)
        }
      } else {
        cacheEpoch++ // An older GET cannot repopulate data after a successful write.
        for (const k of MEM.keys()) if (k.split('?')[0].startsWith(full(root(path)))) forget(k)
        dropStored(full(root(path)), true)
        ver(root(path)).v++
      }
      return j
    })
    if (get && !off) {
      if (MEM.has(cacheKey)) p.$stale = MEM.get(cacheKey)
      else if (keep) p.$stored = load(cacheKey, keep)
    }
  }
  if (undo) p = p.catch(e => {
    batch(() => { for (let i = undo.length - 2; i >= 0; i -= 2) undo[i].v = undo[i + 1] })
    throw e
  })
  p.cancel = () => ctl.abort()
  if (file) Object.defineProperty(p, 'progress', { get: () => up.v }) // 0–1, a signal read
  return p
}
const any = a => (AbortSignal.any ? AbortSignal.any(a) : a[1] || a[0])

// identical GETs in flight share one request; it is aborted only when every caller has left
const FLY = new Map()
function shared(key, go, signal) {
  let e = FLY.get(key)
  if (!e) {
    const c = new AbortController()
    e = { n: 0, c, p: go(c.signal) }
    e.p.then(() => {}, () => {}).then(() => FLY.get(key) === e && FLY.delete(key))
    FLY.set(key, e)
  }
  e.n++
  return new Promise((ok, no) => {
    let left = false
    const leave = () => {
      if (left) return
      left = true
      no(new ApiError('aborted', 'The request was cancelled.'))
      if (--e.n === 0) FLY.get(key) === e && FLY.delete(key), e.c.abort()
    }
    if (signal.aborted) return leave()
    signal.addEventListener('abort', leave, { once: true })
    const settle = (fn, value) => {
      if (left) return
      left = true
      signal.removeEventListener('abort', leave)
      fn(value)
    }
    e.p.then(v => settle(ok, v), x => settle(no, x))
  })
}

// retries only where safe: GET/HEAD/PUT/DELETE on network, timeout, 502/503/504, backoff 300 ms ×2 with jitter, up to
// `retry`; 429/503 wait Retry-After (up to 10 s). POST and PATCH are never retried.
async function run(u, M, body, sg, pri, up) {
  for (let i = 0; ; i++) {
    try {
      return await once(u, M, body, sg, pri, up)
    } catch (e) {
      const again = /^(GET|HEAD|PUT|DELETE)$/.test(M) && i < P.retry && (e.kind === 'network' || e.kind === 'timeout' || (e.kind === 'http' && /^(429|502|503|504)$/.test(e.status)))
      const wait = e.retryAfter != null ? e.retryAfter * 1e3 : e.status === 429 ? Infinity : 300 * 2 ** i * (0.75 + Math.random() / 2)
      if (!again || wait > 1e4) throw e
      await new Promise((ok, no) => {
        const t = setTimeout(ok, wait)
        sg.addEventListener('abort', () => (clearTimeout(t), no(new ApiError('aborted', 'The request was cancelled.'))), { once: true })
      })
    }
  }
}

// per-origin concurrency queue (6) by priority; navigation aborts the previous route's background requests
const Q = new Map(), LOW = new Set(), LIMIT = 6, WAIT_MAX = 128
if (G.addEventListener) G.addEventListener('brain:navigate', () => LOW.forEach(c => c.abort()))
function slot(origin, pri, sg) {
  if (sg.aborted) return Promise.reject(new ApiError('aborted', 'The request was cancelled.'))
  const q = Q.get(origin) || (Q.set(origin, { n: 0, w: [] }), Q.get(origin))
  let done = false
  const release = () => {
    if (done) return
    done = true
    const w = q.w.shift()
    w ? w.go() : q.n--
    if (!q.n && !q.w.length) Q.delete(origin)
  }
  if (q.n < LIMIT) return q.n++, Promise.resolve(release)
  if (q.w.length >= WAIT_MAX) return Promise.reject(new ApiError('http', 'The request queue is full.', { status: 503, retryAfter: 1 }))
  return new Promise((ok, no) => {
    let timer
    const cleanup = () => { clearTimeout(timer); sg.removeEventListener('abort', abort) }
    const fail = error => {
      const j = q.w.indexOf(w)
      if (j >= 0) { q.w.splice(j, 1); cleanup(); no(error) }
    }
    const abort = () => fail(new ApiError('aborted', 'The request was cancelled.'))
    const w = { pri, go: () => { cleanup(); ok(release) } }, i = q.w.findIndex(x => x.pri > pri)
    q.w.splice(i < 0 ? q.w.length : i, 0, w)
    sg.addEventListener('abort', abort, { once: true })
    timer = setTimeout(() => fail(new ApiError('timeout', 'The request expired while waiting for a connection.')), P.timeout)
  })
}

// paths in api() are the app's own (`/users`): `<meta http base>` then the app's base path (<meta name="brain-base">)
const appBase = () => {
  const m = G.document && document.querySelector('meta[name=brain-base]')
  return m ? m.getAttribute('content').replace(/\/$/, '') : ''
}
const full = u => (u.startsWith('/') && !u.startsWith('//') ? appBase() + P.base + u : u)

async function once(u, M, body, sg, pri, up) {
  const url = full(u), origin = new URL(url, G.location ? location.href : 'http://localhost').origin
  const low = pri === 2 && new AbortController()
  if (low) LOW.add(low), (sg = any([sg, low.signal]))
  let release
  try { release = await slot(origin, pri, sg) } catch (e) { if (low) LOW.delete(low); throw e }
  if (sg.aborted) { release(); if (low) LOW.delete(low); throw new ApiError('aborted', 'The request was cancelled.') }
  const tm = AbortSignal.timeout(P.timeout), t0 = Date.now()
  const headers = { accept: 'application/json', ...(typeof body === 'string' ? { 'content-type': 'application/json' } : {}), ...(P.headers ? P.headers() : {}) }
  let st = 0, text = '', h = () => null
  try {
    if (typeof body === 'object' && G.XMLHttpRequest) ({ st, text, h } = await xhr(url, M, body, headers, any([sg, tm]), up))
    else {
      const r = await fetch(url, { method: M, credentials: 'same-origin', headers, body, signal: any([sg, tm]) })
      st = r.status
      h = k => (r.headers && r.headers.get ? r.headers.get(k) : null)
      text = await r.text()
      up.v = 1
    }
  } catch (e) {
    throw sg.aborted ? new ApiError('aborted', 'The request was cancelled.') : tm.aborted ? new ApiError('timeout', 'The server took too long to answer.') : new ApiError('network', 'Connection lost.', { cause: e })
  } finally {
    release()
    if (low) LOW.delete(low)
    // a write tells the router's prefetch that pages it fetched may be stale (§19)
    if (M !== 'GET' && G.dispatchEvent && G.Event) G.dispatchEvent(new Event('brain:write'))
    if (G.__brainDev && G.dispatchEvent && G.CustomEvent) G.dispatchEvent(new CustomEvent('brain:net', { detail: { method: M, url, status: st, ms: Date.now() - t0, size: text.length } }))
  }
  let j
  try {
    j = text ? JSON.parse(text, reviver) : null
  } catch {
    j = null
  }
  if (st >= 200 && st < 300) return { value: j, cacheable: !/no-store|private/i.test(h('cache-control') || '') }
  const ra = h('retry-after'), retryAfter = ra == null ? undefined : isNaN(ra) ? Math.max(0, (Date.parse(ra) - Date.now()) / 1e3) : +ra
  if (st === 401 && P.login && G.location && !location.pathname.endsWith(P.login)) location.assign(appBase() + P.login + '?next=' + encodeURIComponent(location.pathname + location.search))
  throw new ApiError('http', (j && j.error) || `${M} ${u}: ${st}`, { status: st, errors: j && j.errors, retryAfter, id: h('x-request-id') || undefined })
}

// multipart upload with progress (fetch has no upload progress)
function xhr(url, M, body, headers, sg, up) {
  return new Promise((ok, no) => {
    const x = new XMLHttpRequest()
    x.open(M, url)
    x.withCredentials = false
    for (const k in headers) if (k !== 'content-type') x.setRequestHeader(k, headers[k])
    if (x.upload) x.upload.onprogress = e => e.lengthComputable && (up.v = e.loaded / e.total)
    x.onload = () => ((up.v = 1), ok({ st: x.status, text: x.responseText || '', h: k => x.getResponseHeader(k) }))
    x.onerror = x.ontimeout = () => no(new TypeError('upload failed'))
    x.onabort = () => no(new DOMException('aborted', 'AbortError'))
    sg.addEventListener('abort', () => x.abort(), { once: true })
    x.send(body)
  })
}

// Shape check with form()'s mini-language: 'num!' · '[str]' · { field: rule | schema } · [Schema] (list of objects).
// Returns the first difference as text, or ''. Names the compiler could not resolve ('User') are not checked.
function diff(s, v, at = '') {
  if (Array.isArray(s)) {
    if (!Array.isArray(v)) return `${at || 'value'}: expected a list`
    for (let i = 0; i < v.length; i++) {
      const d = diff(s[0], v[i], `${at}[${i}]`)
      if (d) return d
    }
    return ''
  }
  if (s && typeof s === 'object') {
    if (!v || typeof v !== 'object' || Array.isArray(v)) return `${at || 'value'}: expected an object`
    for (const k of Object.keys(s)) {
      const d = diff(s[k], v[k], at ? `${at}.${k}` : k)
      if (d) return d
    }
    return ''
  }
  let r
  try {
    r = rule(s)
    const m = r.t === 'obj' ? '' : check(r, v)
    return m ? `${at || 'value'}: ${m}` : ''
  } catch {
    return ''
  }
}

const CACHE = 'brain:data' // not `brain-…`: those are per-build caches the service worker rotates
let storageTail = Promise.resolve(), storagePending = 0, storageBlocked = false
const storageTask = work => {
  if (storageBlocked) return Promise.resolve()
  if (storagePending >= 128) {
    // Coalesce overload into one full invalidation after queued writes. Never drop an invalidation and retain
    // data that a successful mutation made stale. Readers stay disabled until cleanup succeeds.
    storageBlocked = true
    storageTail = storageTail.then(async () => { try { if (G.caches) await G.caches.delete(CACHE); storageBlocked = false } catch {} })
    return Promise.resolve()
  }
  storagePending++
  const task = storageTail.then(work, work)
  storageTail = task.catch(() => {}).finally(() => storagePending--)
  return task
}
function save(u, j) { return storageTask(() => saveNow(u, j)) }
async function saveNow(u, j) {
  try {
    if (!G.caches) return
    const c = await G.caches.open(CACHE)
    const text = JSON.stringify(j)
    if (new TextEncoder().encode(text).length > MEM_ENTRY) return
    await c.put(new URL(u, G.location && location.href).href, new Response(text, { headers: { 'content-type': 'application/json', 'x-brain-t': String(Date.now()) } }))
    const keys = await c.keys()
    for (let i = 0; i < keys.length - 128; i++) await c.delete(keys[i])
  } catch {}
}
function dropStored(u, prefix = false) { return storageTask(() => dropStoredNow(u, prefix)) }
async function dropStoredNow(u, prefix = false) {
  try {
    if (!G.caches) return
    const c = await G.caches.open(CACHE), url = new URL(u, G.location && location.href).href
    if (!prefix) await c.delete(url)
    else for (const key of await c.keys()) if (key.url.startsWith(url)) await c.delete(key)
  } catch {}
}
async function load(u, keep) {
  try {
    if (!G.caches || storageBlocked) return undefined
    await storageTail
    const r = await (await G.caches.open(CACHE)).match(new URL(u, G.location && location.href).href)
    if (!r) return undefined
    const stamp = Number(r.headers.get('x-brain-t')), now = Date.now()
    if (!Number.isFinite(stamp) || stamp <= 0 || stamp > now + 60000 || now - stamp > keep) { await dropStored(u); return undefined }
    return JSON.parse(await r.text(), reviver)
  } catch {
    return undefined
  }
}

// `resource(fn, { page, every })`: a read-only node whose value is a Promise, for `{#await x}`. Signals read inside fn
// make it refetch (so does a write to the same path, or `r.refetch()`); the previous request is aborted and its answer
// ignored. A cached answer is shown at once and replaced only if the network brings something different; a network
// error keeps the cached answer on screen.
// `page: true` → fn is called with `{ page: 1 }`, `r.more()` appends the next page, `r.done` turns true on a short or
// empty page. `every: '30s'` refetches on an interval, paused while the tab is hidden or offline.
// a cancelled run is not an error to show: {#await} keeps waiting for the run that replaced it
const quiet = p => p.catch(e => (e && e.kind === 'aborted' ? new Promise(() => {}) : Promise.reject(e)))
export function resource(f, o = {}) {
  const n = sig(new Promise(() => {})), force = sig(0), done = sig(false)
  let tok = 0, acc = [], pg = 1, size = 0, loading = false
  const call = (a, c) => {
    CUR = c
    try {
      return f(a)
    } catch (e) {
      return Promise.reject(e)
    } finally {
      CUR = null
    }
  }
  fx(() => {
    force.v // refetch()
    const t = ++tok, c = new AbortController()
    let p = call(o.page ? { page: (pg = 1) } : undefined, c.signal)
    if (o.page) {
      untrack(() => (done.v = false))
      p = Promise.resolve(p).then(l => ((acc = l || []), (size = acc.length), t === tok && untrack(() => (done.v = !size)), acc))
    }
    const same = (a, b) => JSON.stringify(a) === JSON.stringify(b)
    const later = (old, q) => q.then(v => t === tok && !same(v, old) && (n.v = Promise.resolve(v)), () => {})
    if (p && p.$stale !== undefined) (n.v = Promise.resolve(p.$stale)), later(p.$stale, p)
    else if (p && p.$stored) n.v = p.$stored.then(s => (s === undefined ? quiet(p) : (later(s, p), s)))
    else n.v = quiet(Promise.resolve(p))
    return () => c.abort() // re-run or unmount: the request is cancelled
  })
  n.refetch = () => force.v++
  n.more = () => {
    if (!o.page || loading || untrack(() => done.v)) return Promise.resolve()
    const t = tok
    loading = true
    return Promise.resolve(untrack(() => call({ page: pg + 1 }, null)))
      .then(l => {
        if (t !== tok) return
        l = l || []
        pg++
        if (!l.length || l.length < size) done.v = true
        n.v = Promise.resolve((acc = [...acc, ...l]))
      })
      .finally(() => (loading = false))
  }
  Object.defineProperty(n, 'done', { get: () => done.v })
  if (o.every && !(G.__brainSSR || G.__brainRoute)) {
    const id = setInterval(() => ON.v && !(G.document && document.hidden) && force.v++, ttl(o.every) || +o.every)
    fx(() => () => clearInterval(id))
  }
  return n
}

// `let feed = stream('/events')` (§41.4): Server-Sent Events as a read-only node. `.v` is the last message (JSON when
// it parses, else the text; `{ reduce: (acc, msg) => … , initial }` accumulates instead), `.connected` follows the
// connection. EventSource reconnects by itself; the stream closes with the component. Nothing while the server renders.
export function stream(path, o = {}) {
  const n = sig(o.initial ?? null), up = sig(false), out = { get v() { return n.v }, get connected() { return up.v } }
  if (!G.EventSource || G.__brainSSR || G.__brainRoute) return out
  const es = new EventSource(full(path))
  es.onopen = () => (up.v = true)
  es.onerror = () => (up.v = false)
  es.addEventListener(o.event || 'message', e => {
    let m
    try {
      m = JSON.parse(e.data, reviver)
    } catch {
      m = e.data
    }
    n.v = o.reduce ? o.reduce(n.v, m) : m
  })
  fx(() => () => es.close())
  return out
}
