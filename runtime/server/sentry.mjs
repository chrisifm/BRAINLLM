// Server-only, single-project error transport. It accepts our schema, never arbitrary Sentry envelopes.
import { request as httpsRequest } from 'node:https'
import { randomUUID } from 'node:crypto'
import { STATUS_CODES } from 'node:http'
import dns from 'node:dns'

const TYPES = new Set(['Error', 'TypeError', 'RangeError', 'ReferenceError', 'SyntaxError', 'URIError', 'EvalError', 'AggregateError'])
const UUID = /^[a-f0-9]{8}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{12}$/
const MAX_BODY = 32768
const ERROR_CODES = { 400: 'SENTRY_INVALID_EVENT', 403: 'SENTRY_ORIGIN_REFUSED', 405: 'SENTRY_METHOD_NOT_ALLOWED', 408: 'SENTRY_BODY_TIMEOUT', 413: 'SENTRY_BODY_TOO_LARGE', 415: 'SENTRY_UNSUPPORTED_MEDIA', 429: 'SENTRY_RATE_LIMITED', 503: 'SENTRY_UNAVAILABLE' }
const setupError = () => Error('Sentry requires a valid private HTTPS BRAIN_SENTRY_DSN; self-hosted targets also require the exact private BRAIN_SENTRY_HOST. No DSN value was logged.')
export function parseDSN(raw, approvedHost) {
  try {
    if (typeof raw !== 'string' || raw.length > 2048 || /[\s\\]/.test(raw)) throw 0
    const url = new URL(raw)
    if (url.href !== raw || url.protocol !== 'https:' || url.password || url.search || url.hash || !/^[a-f0-9]{32}$/.test(url.username) || !/^\/[1-9][0-9]{0,19}$/.test(url.pathname)) throw 0
    const hosted = /^(?:[a-z0-9-]+\.)*ingest(?:\.[a-z]{2})?\.sentry\.io$/.test(url.hostname) && !url.port
    if (!hosted && (typeof approvedHost !== 'string' || url.host !== approvedHost)) throw 0
    return Object.freeze({ endpoint: new URL('/api' + url.pathname + '/envelope/', url.origin), key: url.username })
  } catch { throw setupError() }
}
const keys = (value, names) => value && typeof value === 'object' && !Array.isArray(value) && Object.keys(value).length === names.length && names.every(key => Object.hasOwn(value, key))
export function sanitizeEvent(value, assets) {
  if (!keys(value, ['schemaVersion', 'type', 'frames']) || value.schemaVersion !== 1 || !TYPES.has(value.type) || !Array.isArray(value.frames) || value.frames.length < 1 || value.frames.length > 20) return null
  const frames = []
  for (const f of value.frames) {
    if (!keys(f, ['asset', 'line', 'column', 'debugId']) || typeof f.asset !== 'string' || f.asset.length > 512 || !/^\/(?:[a-zA-Z0-9_.-]+\/)*[a-z0-9]{8}\.(?:js|jsbrain)$/.test(f.asset) || !UUID.test(f.debugId) || !Object.hasOwn(assets, f.asset) || assets[f.asset] !== f.debugId || !Number.isSafeInteger(f.line) || f.line < 1 || f.line > 9999999 || !Number.isSafeInteger(f.column) || f.column < 1 || f.column > 9999999) return null
    frames.push({ filename: f.asset, abs_path: f.asset, lineno: f.line, colno: f.column, in_app: true })
  }
  const images = [...new Set(value.frames.map(f => f.asset))].map(asset => ({ type: 'sourcemap', code_file: asset, debug_id: assets[asset] }))
  return { platform: 'javascript', level: 'error', exception: { values: [{ type: value.type, value: 'Application error', stacktrace: { frames: frames.reverse() } }] }, debug_meta: { images } }
}
function retrySeconds(headers, status, now) {
  let seconds = 0
  const limits = headers['x-sentry-rate-limits']
  if (typeof limits === 'string') for (const part of limits.slice(0, 2048).split(',').slice(0, 32)) {
    const [delay, categories] = part.trim().split(':')
    if (categories !== undefined && (!categories || categories.split(';').includes('error'))) {
      const n = Number(delay)
      if (Number.isFinite(n) && n >= 0) seconds = Math.max(seconds, n)
    }
  }
  if (status === 429) {
    const raw = headers['retry-after']
    const number = typeof raw === 'string' && raw.length <= 128 ? Number(raw) : NaN
    const date = typeof raw === 'string' && raw.length <= 128 ? Date.parse(raw) : NaN
    seconds = Math.max(seconds, Number.isFinite(number) ? number : Number.isFinite(date) ? (date - now) / 1000 : 60)
  }
  return Math.min(3600, Math.max(0, Math.ceil(seconds)))
}
export function createSentryTunnel({ dsn, approvedHost, assets, publicOrigin, request = httpsRequest, now = Date.now }) {
  const target = parseDSN(dsn, approvedHost)
  let ownOrigin
  if (publicOrigin !== undefined) {
    try {
      const url = new URL(publicOrigin)
      if (!['http:', 'https:'].includes(url.protocol) || url.username || url.password || url.pathname !== '/' || url.search || url.hash) throw 0
      ownOrigin = url.origin
    } catch { throw Error('Sentry needs one canonical public HTTP(S) origin; set PUBLIC_SITE_URL for a TLS reverse proxy') }
  }
  if (!assets || typeof assets !== 'object' || Array.isArray(assets) || Object.keys(assets).length > 4096) throw Error('Sentry requires the build asset catalog; rebuild the app with sentry.enabled true')
  const catalog = Object.freeze({ ...assets })
  const clients = new Map(), pending = new Set(), leases = new Set()
  let closed = false, inFlight = 0, cooldown = 0, minute = -1, total = 0
  const answer = (res, status, wait) => {
    if (res.destroyed || res.writableEnded) return
    const body = status >= 400 ? JSON.stringify({ error: { code: ERROR_CODES[status] || 'SENTRY_REQUEST_FAILED', message: STATUS_CODES[status] || 'Request failed' } }) : ''
    res.writeHead(status, { 'cache-control': 'no-store', ...(body ? { 'content-type': 'application/json; charset=utf-8', 'content-length': Buffer.byteLength(body) } : {}), ...(status === 405 ? { allow: 'POST' } : {}), ...(wait ? { 'retry-after': String(wait) } : {}) })
    res.end(body)
  }
  const forward = (event, lease) => new Promise(resolve => {
    if (closed) return resolve()
    const eventId = randomUUID().replaceAll('-', '')
    const payload = JSON.stringify({ event_id: eventId, timestamp: now() / 1000, ...event })
    const envelope = JSON.stringify({ event_id: eventId, sent_at: new Date(now()).toISOString() }) + '\n' + JSON.stringify({ type: 'event', length: Buffer.byteLength(payload) }) + '\n' + payload + '\n'
    let outgoing, response, settled = false
    const finish = () => {
      if (settled) return
      settled = true; clearTimeout(timer); pending.delete(cancel)
      outgoing?.destroy(); response?.destroy(); resolve()
    }
    const cancel = () => finish()
    const timer = setTimeout(finish, 3000)
    pending.add(cancel)
    // Destroying a request cannot cancel libc/libuv DNS. Keep its admission lease until every native
    // callback completes, even after timeout or close. An expired callback must never start a connection.
    const lookup = (host, options, callback) => {
      if (settled || closed) return callback(Object.assign(Error('Sentry lookup expired'), { code: 'ECANCELED' }))
      lease.lookups++
      try {
        dns.lookup(host, options, (...args) => {
          lease.lookups--
          if (lease.finished && !lease.lookups) leases.delete(lease)
          if (settled || closed) callback(Object.assign(Error('Sentry lookup expired'), { code: 'ECANCELED' }))
          else callback(...args)
        })
      } catch (error) { lease.lookups--; throw error }
    }
    try {
      outgoing = request(target.endpoint, {
        method: 'POST', agent: false, rejectUnauthorized: true, maxHeaderSize: 8192, lookup,
        headers: { 'content-type': 'application/x-sentry-envelope', 'content-length': Buffer.byteLength(envelope), 'x-sentry-auth': 'Sentry sentry_version=7,sentry_key=' + target.key + ',sentry_client=brain.native/1' },
      }, incoming => {
        response = incoming
        const delay = retrySeconds(incoming.headers, incoming.statusCode, now())
        if (delay) cooldown = Math.max(cooldown, now() + delay * 1000)
        // No response body, redirects or retries. Destroy rather than buffering an untrusted body.
        incoming.on('error', finish); finish()
      })
      outgoing.on('error', finish); outgoing.end(envelope)
    } catch { finish() }
  })
  const handle = async (req, res, client) => {
    if (closed) return answer(res, 503, 60)
    if (req.method !== 'POST') return answer(res, 405)
    let origin
    try { origin = new URL(req.headers.origin) } catch { return answer(res, 403) }
    const expected = ownOrigin || (req.socket.encrypted ? 'https://' : 'http://') + req.headers.host
    if (origin.origin !== expected || origin.host !== req.headers.host || origin.origin !== req.headers.origin || req.headers['sec-fetch-site'] && req.headers['sec-fetch-site'] !== 'same-origin') return answer(res, 403)
    if (req.headers['content-encoding'] || req.headers['content-length'] && (!/^\d{1,6}$/.test(req.headers['content-length']) || +req.headers['content-length'] > MAX_BODY)) return answer(res, 413)
    const media = req.headers['content-type']
    if (typeof media !== 'string' || !/^application\/json(?:\s*;\s*charset\s*=\s*(?:utf-8|"utf-8"))?$/i.test(media.trim())) return answer(res, 415)
    const at = now(), bucket = Math.floor(at / 60000)
    if (minute !== bucket) { minute = bucket; total = 0; clients.clear() }
    if (cooldown > at) return answer(res, 429, Math.max(1, Math.ceil((cooldown - at) / 1000)))
    const key = typeof client === 'string' && client.length <= 128 ? client : 'unknown'
    if (inFlight >= 4 || leases.size >= 4 || total >= 120 || !clients.has(key) && clients.size >= 1024 || (clients.get(key) || 0) >= 8) return answer(res, 429, 60)
    clients.set(key, (clients.get(key) || 0) + 1); total++; inFlight++
    const lease = { lookups: 0, finished: false }
    leases.add(lease)
    let timer
    try {
      const raw = await new Promise(resolve => {
        let size = 0, chunks = [], done = false
        const finish = result => { if (done) return; done = true; clearTimeout(timer); pending.delete(cancel); req.removeListener('data', data); req.removeListener('end', end); req.removeListener('error', error); req.removeListener('aborted', error); chunks = []; resolve(result) }
        const cancel = () => { finish(null); answer(res, 503, 60); req.destroy() }
        const data = chunk => { size += chunk.length; if (size > MAX_BODY) { finish(null); req.resume() } else chunks.push(chunk) }
        const end = () => finish(Buffer.concat(chunks).toString('utf8'))
        const error = () => finish(null)
        timer = setTimeout(() => { finish(null); answer(res, 408); req.destroy() }, 3000)
        pending.add(cancel)
        req.on('data', data); req.once('end', end); req.once('error', error); req.once('aborted', error)
      })
      if (closed) return answer(res, 503, 60)
      if (raw === null) return answer(res, 413)
      let value
      try { value = JSON.parse(raw) } catch { return answer(res, 400) }
      const event = sanitizeEvent(value, catalog)
      value = null
      if (!event) return answer(res, 400)
      await forward(event, lease)
      return answer(res, closed ? 503 : 202, closed ? 60 : undefined)
    } finally { clearTimeout(timer); inFlight--; lease.finished = true; if (!lease.lookups) leases.delete(lease) }
  }
  return { handle, close() { closed = true; for (const cancel of pending) cancel(); pending.clear(); clients.clear() } }
}
