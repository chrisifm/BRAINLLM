// Outbound HTTP for server logic: bounded attempts, deadlines, safe retries and redacted failures.
// Built on node:http(s) with a resolver guard: every address a name resolves to is checked at connect time, so a
// public name that resolves (or rebinds) to a private address is refused. No client library is bundled.
const RETRY_STATUS = new Set([408, 425, 429, 500, 502, 503, 504])
const SAFE_METHODS = new Set(['GET', 'HEAD', 'OPTIONS', 'PUT', 'DELETE'])
const MAX_TIMEOUT = 60000, MAX_RETRIES = 5, MAX_BYTES = 16777216, MAX_WAIT = 30000

export class HttpCallError extends Error {
  constructor(code, message, detail = {}) {
    super(message)
    this.name = 'HttpCallError'
    this.code = code
    this.status = detail.status ?? null
    this.attempts = detail.attempts ?? 0
    this.url = detail.url ?? ''
  }
}

function ipv4(host) {
  const parts = host.split('.')
  if (parts.length !== 4 || !parts.every(p => /^\d{1,3}$/.test(p) && +p < 256)) return null
  return parts.map(Number)
}
// 0/8, 10/8, 127/8, 169.254/16 (cloud metadata), 172.16/12, 192.0.0/24, 192.168/16, 100.64/10 (CGNAT),
// 198.18/15 (benchmark), 224/4 multicast and 240/4 reserved
function privateV4([a, b, c]) {
  return a === 0 || a === 10 || a === 127 || (a === 169 && b === 254) || (a === 172 && b >= 16 && b < 32) || (a === 192 && b === 168) || (a === 192 && b === 0 && c === 0) || (a === 100 && b >= 64 && b < 128) || (a === 198 && (b === 18 || b === 19)) || a >= 224
}
function ipv6(host) {
  let text = host
  const tail = text.match(/:(\d+\.\d+\.\d+\.\d+)$/)
  if (tail) {
    const v4 = ipv4(tail[1])
    if (!v4) return null
    text = `${text.slice(0, -tail[1].length)}${((v4[0] << 8) | v4[1]).toString(16)}:${((v4[2] << 8) | v4[3]).toString(16)}`
  }
  const halves = text.split('::')
  if (halves.length > 2) return null
  const head = halves[0] ? halves[0].split(':') : [], rest = halves.length === 2 && halves[1] ? halves[1].split(':') : []
  const fill = 8 - head.length - rest.length
  if (halves.length === 1 ? fill !== 0 : fill < 1) return null
  const words = [...head, ...Array(halves.length === 2 ? fill : 0).fill('0'), ...rest]
  return words.every(w => /^[0-9a-f]{1,4}$/.test(w)) ? words.map(w => parseInt(w, 16)) : null
}
// Loopback, unspecified, IPv4-mapped/compatible, NAT64 and 6to4 (checked against the embedded IPv4), unique local,
// link-local, site-local, multicast, Teredo and documentation ranges
function privateV6(w) {
  const quad = (hi, lo) => [hi >> 8, hi & 255, lo >> 8, lo & 255]
  if (w.slice(0, 5).every(x => x === 0) && (w[5] === 0 || w[5] === 0xffff)) return privateV4(quad(w[6], w[7]))
  if (w[0] === 0x64 && w[1] === 0xff9b) return privateV4(quad(w[6], w[7]))
  if (w[0] === 0x2002) return privateV4(quad(w[1], w[2]))
  const top = w[0]
  return (top & 0xfe00) === 0xfc00 || (top & 0xffc0) === 0xfe80 || (top & 0xffc0) === 0xfec0 || (top & 0xff00) === 0xff00 || (top === 0x2001 && (w[1] === 0 || w[1] === 0xdb8))
}
// True for an IP literal (v4 or v6, brackets allowed) in a non-public range; false for names and public addresses
function privateAddress(address) {
  const host = String(address).toLowerCase().replace(/^\[|\]$/g, '').replace(/%.*$/, '')
  const v4 = ipv4(host)
  if (v4) return privateV4(v4)
  const v6 = host.includes(':') ? ipv6(host) : null
  return v6 ? privateV6(v6) : host.includes(':')
}
function privateHost(hostname) {
  const host = hostname.toLowerCase().replace(/^\[|\]$/g, '').replace(/\.$/, '')
  if (host === 'localhost' || /\.(localhost|internal|local|home\.arpa)$/.test(host)) return true
  return privateAddress(host)
}
function redact(url) {
  return `${url.origin}${url.pathname}`
}
function bounded(value, name, min, max, fallback) {
  if (value === undefined) return fallback
  if (!Number.isInteger(value) || value < min || value > max) throw RangeError(`httpCall: ${name} must be an integer in ${min}..${max}`)
  return value
}
function retryAfter(header) {
  if (header == null) return null
  const secs = Number(header)
  const ms = Number.isFinite(secs) ? secs * 1000 : Date.parse(header) - Date.now()
  return Number.isFinite(ms) ? Math.min(Math.max(ms, 0), MAX_WAIT) : null
}
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms))

const PRIVATE = 'BRAIN_PRIVATE_ADDRESS'
const REDIRECT_STATUS = new Set([301, 302, 303, 307, 308])
const CREDENTIAL_HEADERS = ['authorization', 'cookie', 'proxy-authorization']
let net
const nodeNet = () => (net ??= Promise.all([import('node:http'), import('node:https'), import('node:dns')]).then(([http, https, dns]) => ({ http, https, dns })))

// Private targets are allowed by `allowPrivate: true`, by naming the host in `allowPrivate: [...]`, or in development
// only by BRAIN_OUTBOUND_ALLOW_PRIVATE=1 (ignored when NODE_ENV is production)
function allowsPrivate(option, hostname) {
  if (option === true) return true
  const host = hostname.toLowerCase().replace(/^\[|\]$/g, '')
  if (Array.isArray(option) && option.some(h => String(h).toLowerCase() === host)) return true
  const env = globalThis.process?.env ?? {}
  return env.BRAIN_OUTBOUND_ALLOW_PRIVATE === '1' && env.NODE_ENV !== 'production'
}
function checkTarget(url, option) {
  const safe = redact(url)
  if (!['http:', 'https:'].includes(url.protocol) || url.username || url.password) throw new HttpCallError('UNSAFE_URL', `httpCall: ${safe} must be http(s) without credentials`, { url: safe })
  const allow = allowsPrivate(option, url.hostname)
  if (!allow && privateHost(url.hostname)) throw new HttpCallError('UNSAFE_URL', `httpCall: ${safe} targets a private or loopback address; pass allowPrivate for internal services`, { url: safe })
  return allow
}
function guardedLookup(resolver, allow) {
  return (hostname, options, callback) => {
    resolver(hostname, { ...options, all: true }, (error, found) => {
      if (error) return callback(error)
      const list = (Array.isArray(found) ? found : [{ address: found, family: String(found).includes(':') ? 6 : 4 }]).filter(a => a && a.address)
      if (!list.length) return callback(Object.assign(new Error(`${hostname} has no address`), { code: 'ENOTFOUND' }))
      if (!allow && list.some(a => privateAddress(a.address))) return callback(Object.assign(new Error(`${hostname} resolves to a private address`), { code: PRIVATE }))
      return options.all ? callback(null, list) : callback(null, list[0].address, list[0].family)
    })
  }
}
function payload(body, headers) {
  if (body == null) return undefined
  if (typeof body === 'string' || body instanceof Uint8Array) return body
  if (body instanceof ArrayBuffer) return new Uint8Array(body)
  if (ArrayBuffer.isView(body)) return new Uint8Array(body.buffer, body.byteOffset, body.byteLength)
  if (body instanceof URLSearchParams) {
    if (!headers.has('content-type')) headers.set('content-type', 'application/x-www-form-urlencoded;charset=UTF-8')
    return body.toString()
  }
  throw TypeError('httpCall: body must be a string, bytes or URLSearchParams')
}
// One request on a fresh connection (no pooling, so a socket opened for an allowed private host is never reused)
async function exchange(url, { method, headers, body, signal, lookup, allow }) {
  const { http, https, dns } = await nodeNet()
  return new Promise((resolve, reject) => {
    const request = (url.protocol === 'https:' ? https : http).request(url, { method, headers: Object.fromEntries(headers), signal, agent: false, lookup: guardedLookup(lookup ?? dns.lookup, allow) })
    request.on('response', resolve)
    request.on('error', reject)
    request.end(body)
  })
}
function plainHeaders(raw) {
  return Object.fromEntries(Object.entries(raw).map(([k, v]) => [k, Array.isArray(v) ? v.join(', ') : v]))
}

async function readBody(response, maxBytes, url, attempts) {
  const chunks = []
  let size = 0
  for await (const value of response) {
    size += value.byteLength
    if (size > maxBytes) {
      response.destroy()
      throw new HttpCallError('TOO_LARGE', `httpCall: response from ${url} exceeds ${maxBytes} bytes`, { status: response.statusCode, attempts, url })
    }
    chunks.push(value)
  }
  const out = new Uint8Array(size)
  let at = 0
  for (const c of chunks) { out.set(c, at); at += c.byteLength }
  return out
}

/**
 * httpCall(url, {method, headers, json, body, timeout, retries, idempotencyKey, maxBytes, backoff, allowPrivate, redirects, lookup, signal})
 * → {status, ok, headers, text, json, attempts, url}. Network failures, deadlines, oversized bodies and final
 * retryable statuses (5xx, 408, 425, 429) throw HttpCallError; other statuses are returned.
 */
export async function httpCall(target, options = {}) {
  const method = String(options.method ?? 'GET').toUpperCase()
  const timeout = bounded(options.timeout, 'timeout', 1, MAX_TIMEOUT, 10000)
  const retries = bounded(options.retries, 'retries', 0, MAX_RETRIES, 2)
  const maxBytes = bounded(options.maxBytes, 'maxBytes', 1, MAX_BYTES, 1048576)
  const backoff = bounded(options.backoff, 'backoff', 1, 10000, 250)
  const redirects = bounded(options.redirects, 'redirects', 0, 10, 0)
  if (options.lookup !== undefined && typeof options.lookup !== 'function') throw TypeError('httpCall: lookup must be a dns.lookup-compatible function')
  let url
  try { url = new URL(target) } catch { throw new HttpCallError('UNSAFE_URL', 'httpCall: an absolute http(s) URL is required') }
  const safe = redact(url)
  const allowFirst = checkTarget(url, options.allowPrivate)

  const headers = new Headers(options.headers ?? {})
  let body = options.body
  if (options.json !== undefined) {
    body = JSON.stringify(options.json)
    if (!headers.has('content-type')) headers.set('content-type', 'application/json')
    if (!headers.has('accept')) headers.set('accept', 'application/json')
  }
  body = payload(body, headers)
  if (options.idempotencyKey) headers.set('idempotency-key', options.idempotencyKey === true ? globalThis.crypto.randomUUID() : String(options.idempotencyKey))
  const replayable = SAFE_METHODS.has(method) || headers.has('idempotency-key')
  const allowed = replayable ? retries : 0

  // One attempt: the request plus up to `redirects` hops, each target checked again before it is contacted
  async function attemptOnce(signal) {
    let hop = { url, method, headers: new Headers(headers), body, allow: allowFirst }
    for (let hops = 0; ; hops++) {
      const response = await exchange(hop.url, { ...hop, signal, lookup: options.lookup })
      const location = response.headers.location
      if (!redirects || !REDIRECT_STATUS.has(response.statusCode) || !location) return { response, url: hop.url }
      response.resume()
      if (hops >= redirects) throw new HttpCallError('REDIRECT', `httpCall: ${safe} redirected more than ${redirects} times`, { status: response.statusCode, url: safe })
      let next
      try { next = new URL(location, hop.url) } catch { throw new HttpCallError('REDIRECT', `httpCall: ${redact(hop.url)} sent an invalid redirect`, { status: response.statusCode, url: safe }) }
      if (hop.url.protocol === 'https:' && next.protocol === 'http:') throw new HttpCallError('UNSAFE_URL', `httpCall: ${redact(hop.url)} redirected from https to http`, { url: redact(next) })
      const allow = checkTarget(next, options.allowPrivate)
      const nextHeaders = new Headers(hop.headers)
      let nextMethod = hop.method, nextBody = hop.body
      if (response.statusCode === 303 || ((response.statusCode === 301 || response.statusCode === 302) && hop.method === 'POST')) {
        nextMethod = hop.method === 'HEAD' ? 'HEAD' : 'GET'
        nextBody = undefined
        for (const h of ['content-type', 'content-length']) nextHeaders.delete(h)
      }
      if (next.origin !== hop.url.origin) for (const h of CREDENTIAL_HEADERS) nextHeaders.delete(h)
      hop = { url: next, method: nextMethod, headers: nextHeaders, body: nextBody, allow }
    }
  }

  for (let attempt = 1; ; attempt++) {
    const signal = AbortSignal.any([AbortSignal.timeout(timeout), options.signal].filter(Boolean))
    let result = null, wait = null
    try {
      result = await attemptOnce(signal)
      const status = result.response.statusCode
      if (RETRY_STATUS.has(status)) {
        result.response.resume()
        if (attempt > allowed) throw new HttpCallError('HTTP_STATUS', `httpCall: ${safe} answered ${status}`, { status, attempts: attempt, url: safe })
        wait = retryAfter(result.response.headers['retry-after'])
        result = null
      } else {
        const bytes = await readBody(result.response, maxBytes, safe, attempt)
        const text = new TextDecoder().decode(bytes)
        const headersOut = plainHeaders(result.response.headers)
        let json = null
        if ((headersOut['content-type'] ?? '').includes('json') && text) {
          try { json = JSON.parse(text) } catch { json = null }
        }
        return { status, ok: status >= 200 && status < 300, headers: headersOut, text, json, attempts: attempt, url: redact(result.url) }
      }
    } catch (error) {
      if (error instanceof HttpCallError) throw error.attempts ? error : Object.assign(error, { attempts: attempt })
      if (error?.code === PRIVATE || error?.cause?.code === PRIVATE) throw new HttpCallError('UNSAFE_URL', `httpCall: ${safe} resolves to a private or loopback address; pass allowPrivate for internal services`, { attempts: attempt, url: safe })
      if (options.signal?.aborted) throw new HttpCallError('ABORTED', `httpCall: request to ${safe} was cancelled`, { attempts: attempt, url: safe })
      const timedOut = error?.name === 'TimeoutError' || error?.name === 'AbortError'
      if (attempt > allowed) throw new HttpCallError(timedOut ? 'TIMEOUT' : 'NETWORK', `httpCall: ${safe} ${timedOut ? `did not answer within ${timeout} ms` : 'is unreachable'}`, { attempts: attempt, url: safe })
    }
    await sleep(wait ?? Math.random() * Math.min(5000, backoff * 2 ** (attempt - 1)))
  }
}
