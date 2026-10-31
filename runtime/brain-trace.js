// `brain trace`, the browser half (INTERNALS §49). Development only: the dev client (crates/brain-serve/src/client.js)
// imports it and nothing a production build ships does. It records navigation, prefetch, fetch and RPC with payload and
// response, console warnings, CSP and CORS failures, sockets and leak findings, and sends them over the HMR socket.
// Values are redacted by the store before they reach disk. A same-origin request carries `x-brain-trace`, the id the
// server and the backend then use, so one id follows it through every layer.
// A new module records without touching any of this: `globalThis.__brainTrace('mymodule', 'mykind', { title, level, data })`.
const G = globalThis
const BODY = 65536
const uuid = () => (G.crypto?.randomUUID ? G.crypto.randomUUID() : '00000000-0000-4000-8000-000000000000'.replace(/0/g, () => (Math.random() * 16 | 0).toString(16)))
const lasting = () => {
  try {
    const known = G.sessionStorage.getItem('brain:trace')
    if (known) return known
    const made = uuid()
    G.sessionStorage.setItem('brain:trace', made)
    return made
  } catch { return uuid() }
}
export const ids = { session: lasting(), tab: uuid() }

let queue = [], send = null, timer = null
const flush = () => {
  timer = null
  if (!queue.length || !send) return
  const events = queue.slice(0, 200)
  if (send(JSON.stringify({ t: 'brain:trace', ...ids, events }))) queue = queue.slice(events.length)
  if (queue.length) timer = setTimeout(flush, 250)
}
export function note(module, kind, fields = {}) {
  if (queue.length >= 1000) queue.shift()
  queue.push({ ts: Date.now(), module, kind, route: G.location?.pathname, ...fields })
  timer ??= setTimeout(flush, 100)
}
export function attach(sender) {
  send = sender
  timer ??= setTimeout(flush, 0)
}
G.__brainTrace = (module, kind, fields) => note(String(module), String(kind), fields)

// ---------- locations: a generated line of a dev module back to its .brain line (the module's inline source map) ----------
const RUNTIME = '/@brain/'
const nativeFetch = G.fetch?.bind(G)
let failedId = null
// the id of the last failed call, for the error report that follows it (brain-err cannot see the request)
export const lastFailed = () => failedId
const MAPS = new Map()
function mapOf(url) {
  if (!MAPS.has(url)) {
    if (MAPS.size > 64) MAPS.clear()
    MAPS.set(url, (async () => {
      const text = await (await nativeFetch(url)).text()
      const found = /sourceMappingURL=data:application\/json;base64,([A-Za-z0-9+/=]+)/.exec(text)
      if (!found) return null
      const { decode } = await import('./brain-err.js')
      return decode(JSON.parse(atob(found[1])).mappings)
    })().catch(() => null))
  }
  return MAPS.get(url)
}
export async function mapping(url, line) {
  try {
    const u = new URL(url, G.location.href)
    if (u.origin !== G.location.origin || u.pathname.includes(RUNTIME)) return null
    u.search = ''
    const lines = await mapOf(u.href)
    const original = lines && lines[line - 1]
    return { file: u.pathname.replace(/^\//, ''), line: original || null, precision: original ? 'source-line' : 'module' }
  } catch { return null }
}
// The first frame outside the runtime: who made this call.
const callSite = () => {
  for (const row of String(new Error().stack).split('\n').slice(2)) {
    const found = /((?:https?):\/\/[^\s)]+):(\d+):\d+/.exec(row)
    if (found && !found[1].includes(RUNTIME)) return { url: found[1], line: +found[2] }
  }
  return null
}
const located = async (site, fields) => {
  const where = site && await mapping(site.url, site.line)
  return where ? { file: where.file, line: where.line, ...fields } : fields
}

// ---------- fetch, RPC and prefetch ----------
// source modules are not responses anyone debugs, and their literals are not data
const textual = type => /json|text\/(?!event-stream|javascript)|xml|form-urlencoded/i.test(type || '')
async function bodyOf(response) {
  const type = response.headers.get('content-type')
  if (!textual(type)) return type ? `[${type}]` : null
  const reader = response.clone().body?.getReader()
  if (!reader) return null
  const decoder = new TextDecoder()
  let text = ''
  try {
    while (text.length < BODY) {
      const { done, value } = await reader.read()
      if (done) return text + decoder.decode()
      text += decoder.decode(value, { stream: true })
    }
    return text.slice(0, BODY) + '…[truncated by the browser]'
  } finally { reader.cancel().catch(() => {}) }
}
const payloadOf = (input, init) => {
  const body = init?.body
  if (typeof body === 'string') return body.slice(0, BODY)
  if (body instanceof G.URLSearchParams) return String(body).slice(0, BODY)
  if (body != null) return `[${body.constructor?.name || typeof body}]`
  return input instanceof G.Request && input.method !== 'GET' && input.method !== 'HEAD' ? '[Request body]' : null
}
const kindOf = (url, init, headers) =>
  url.pathname.includes('/_r/') ? 'rpc'
    : init?.priority === 'low' || headers.has('x-brain-prefetch') ? 'prefetch'
      : /text\/html/.test(headers.get('accept') || '') ? 'page' : 'fetch'
function wrapFetch() {
  if (!nativeFetch || G.fetch.$trace) return
  const wrapped = function (input, init) {
    let url
    try { url = new URL(input instanceof G.Request ? input.url : String(input), G.location.href) } catch { return nativeFetch(input, init) }
    if (url.pathname.includes(RUNTIME) || !/^https?:$/.test(url.protocol) || /\.(?:js|mjs|brain|jsbrain|cssbrain)$/.test(url.pathname)) return nativeFetch(input, init)
    const same = url.origin === G.location.origin, id = uuid(), site = callSite(), started = performance.now()
    const headers = new Headers(init?.headers ?? (input instanceof G.Request ? input.headers : undefined))
    const kind = kindOf(url, init, headers), method = (init?.method || (input instanceof G.Request ? input.method : 'GET')).toUpperCase()
    const payload = payloadOf(input, init)
    if (same) headers.set('x-brain-trace', id)
    const route = same ? url.pathname + url.search : url.href
    return nativeFetch(input, same ? { ...init, headers } : init).then(response => {
      const elapsed = +(performance.now() - started).toFixed(1)
      if (same && response.status >= 400) failedId = id
      bodyOf(response).catch(() => null).then(async text => {
        const fields = { request_id: same ? id : null, route, status: response.status, duration_ms: elapsed, data: { method, payload, response: text } }
        note('browser', kind, response.status >= 400 ? await located(site, fields) : fields)
      })
      return response
    }, async error => {
      const elapsed = +(performance.now() - started).toFixed(1), aborted = error?.name === 'AbortError'
      const fields = { request_id: same ? id : null, route, status: aborted ? null : 0, duration_ms: elapsed, level: aborted ? 'info' : 'error', title: aborted ? 'Request aborted' : same ? String(error?.message || error) : `Cross-origin request to ${url.origin} failed: ${error?.message || error}`, data: { method, payload, aborted } }
      note('browser', aborted ? 'abort' : same ? kind : 'cors', aborted ? fields : await located(site, fields))
      throw error
    })
  }
  wrapped.$trace = 1
  G.fetch = wrapped
}
// Uploads with a progress callback use XMLHttpRequest (brain-data).
function wrapXhr() {
  const X = G.XMLHttpRequest?.prototype
  if (!X || X.$trace) return
  X.$trace = 1
  const open = X.open, sendRequest = X.send
  X.open = function (method, url, ...rest) {
    this.$traced = { method: String(method).toUpperCase(), url: new URL(String(url), G.location.href) }
    return open.call(this, method, url, ...rest)
  }
  X.send = function (body) {
    const info = this.$traced, started = performance.now(), id = uuid()
    if (info) {
      const same = info.url.origin === G.location.origin
      if (same) this.setRequestHeader('x-brain-trace', id)
      this.addEventListener('loadend', () => note('browser', 'xhr', { request_id: same ? id : null, route: same ? info.url.pathname + info.url.search : info.url.href, status: this.status, duration_ms: +(performance.now() - started).toFixed(1), data: { method: info.method, payload: typeof body === 'string' ? body.slice(0, BODY) : body == null ? null : `[${body.constructor?.name}]` } }))
    }
    return sendRequest.call(this, body)
  }
}

// ---------- navigation ----------
function watchNavigation() {
  const entry = G.performance?.getEntriesByType?.('navigation')?.[0]
  const server = entry?.serverTiming?.find(item => item.name === 'rid')?.description
  // the document itself: the server's id comes back in Server-Timing, the only header a page can read about its own load
  const first = () => {
    const done = G.performance?.getEntriesByType?.('navigation')?.[0]
    note('browser', 'navigation', { request_id: server || null, route: G.location.pathname + G.location.search, status: done?.responseStatus || null, duration_ms: done?.loadEventEnd ? +done.loadEventEnd.toFixed(1) : null, data: { type: done?.type || 'navigate' } })
  }
  if (G.document.readyState === 'complete') first()
  else G.addEventListener('load', () => setTimeout(first), { once: true })
  G.addEventListener('brain:navigate', () => note('browser', 'navigation', { route: G.location.pathname + G.location.search, data: { type: 'client' } }))
  G.addEventListener('popstate', () => note('browser', 'navigation', { data: { type: 'history' } }))
  // <link rel="prefetch"> and speculation never pass through fetch()
  try {
    new PerformanceObserver(list => {
      for (const item of list.getEntries()) {
        if (item.initiatorType !== 'link' || !G.document.querySelector(`link[rel="prefetch"][href="${new URL(item.name).pathname}"]`)) continue
        note('browser', 'prefetch', { route: new URL(item.name).pathname, duration_ms: +item.duration.toFixed(1), status: item.responseStatus || null, data: { initiator: 'link' } })
      }
    }).observe({ type: 'resource', buffered: true })
  } catch {}
}

// ---------- console, CSP ----------
function watchConsole() {
  for (const [method, level] of [['warn', 'warn'], ['error', 'error']]) {
    const original = G.console?.[method]
    if (!original) continue
    G.console[method] = function (...args) {
      // brain-err prints the errors it records: those are already `browser/error` events
      if (!(args[0] instanceof Error) && !String(args[0]).startsWith('BRAIN_ERROR')) {
        const site = callSite(), title = args.map(a => (typeof a === 'string' ? a : (() => { try { return JSON.stringify(a) } catch { return String(a) } })())).join(' ').slice(0, 1024)
        located(site, { level, title }).then(fields => note('browser', 'console', fields))
      }
      return original.apply(this, args)
    }
  }
}
function watchCsp() {
  G.document.addEventListener('securitypolicyviolation', async event => {
    const { violatedDirective, effectiveDirective, blockedURI, sourceFile, lineNumber, columnNumber, disposition, sample } = event
    const where = sourceFile && lineNumber ? await mapping(sourceFile, lineNumber) : null
    note('browser', 'csp', {
      level: 'error', title: `CSP ${effectiveDirective || violatedDirective} refused ${blockedURI || 'inline code'}`,
      // a call made through the runtime's own wrappers says nothing about the app: the event keeps its route
      file: where?.file ?? (sourceFile && !sourceFile.includes(RUNTIME) ? new URL(sourceFile, G.location.href).pathname : null), line: where ? where.line : sourceFile?.includes(RUNTIME) ? null : lineNumber || null,
      data: { violation: { directive: effectiveDirective || violatedDirective, blocked: blockedURI, disposition, sample, sourceFile, line: lineNumber, column: columnNumber, precision: where?.precision || 'generated' } },
    })
  })
}

// ---------- sockets ----------
function wrapSockets() {
  const C = G.WebSocket
  if (!C || C.$trace) return
  const W = new Proxy(C, {
    construct(target, args, next) {
      const socket = Reflect.construct(target, args, next), url = String(args[0])
      if (url.includes(RUNTIME)) return socket
      const route = (() => { try { return new URL(url).pathname } catch { return url } })(), opened = performance.now()
      let received = 0, sent = 0
      socket.addEventListener('message', () => received++)
      const write = socket.send
      socket.send = function (...rest) { sent++; return write.apply(this, rest) }
      socket.addEventListener('open', () => note('browser', 'socket-open', { route, data: { url } }))
      socket.addEventListener('close', event => {
        const clean = event.wasClean || event.code === 1000 || event.code === 1001
        note('browser', clean ? 'socket-close' : 'socket', { route, level: clean ? 'info' : 'error', title: clean ? null : `Socket ${route} closed with code ${event.code}`, duration_ms: +(performance.now() - opened).toFixed(1), data: { url, code: event.code, reason: event.reason, received, sent } })
      })
      return socket
    },
  })
  W.$trace = 1
  G.WebSocket = W
  const S = G.EventSource
  if (!S) return
  G.EventSource = new Proxy(S, {
    construct(target, args, next) {
      const source = Reflect.construct(target, args, next), route = String(args[0])
      source.addEventListener('open', () => note('browser', 'socket-open', { route, data: { transport: 'sse' } }))
      // a stream the browser will not retry is the failure; the ordinary reconnect is not
      source.addEventListener('error', () => { if (source.readyState === 2) note('browser', 'socket', { route, level: 'error', title: `Event stream ${route} failed`, data: { transport: 'sse' } }) })
      return source
    },
  })
}

// brain-inspect finds the leaks and already sends them to the same socket; `inspected` reads its batch for this store.
export function inspected(batch) {
  if (!batch.includes('"leak"')) return
  try {
    for (const record of JSON.parse(batch).d || []) {
      if (record.k === 'leak') note('memory', 'leak', { level: 'warn', title: record.msg, code: record.code, fix: record.fix, component: record.name, file: record.file, line: record.line, data: { fx: record.fx } })
    }
  } catch {}
}

if (G.document) {
  wrapFetch()
  wrapXhr()
  wrapSockets()
  watchConsole()
  watchCsp()
  watchNavigation()
}
