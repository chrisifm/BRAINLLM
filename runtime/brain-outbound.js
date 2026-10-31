// Outbound HTTP for server logic: bounded attempts, deadlines, safe retries and redacted failures.
// Built on the platform fetch; no client library is bundled.
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
function privateV4([a, b]) {
  return a === 0 || a === 10 || a === 127 || (a === 169 && b === 254) || (a === 172 && b >= 16 && b < 32) || (a === 192 && b === 168) || (a === 100 && b >= 64 && b < 128) || a >= 224
}
function privateHost(hostname) {
  const host = hostname.toLowerCase().replace(/^\[|\]$/g, '').replace(/\.$/, '')
  if (host === 'localhost' || host.endsWith('.localhost') || host.endsWith('.internal')) return true
  const v4 = ipv4(host)
  if (v4) return privateV4(v4)
  if (!host.includes(':')) return false
  if (host === '::' || host === '::1') return true
  const mapped = host.match(/^::ffff:(\d+\.\d+\.\d+\.\d+)$/)
  if (mapped) return privateV4(ipv4(mapped[1]) ?? [0])
  return /^f[cd]/.test(host) || /^fe[89ab]/.test(host)
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

async function readBody(response, maxBytes, url, attempts) {
  if (!response.body) return new Uint8Array()
  const reader = response.body.getReader()
  const chunks = []
  let size = 0
  for (;;) {
    const { done, value } = await reader.read()
    if (done) break
    size += value.byteLength
    if (size > maxBytes) {
      await reader.cancel().catch(() => {})
      throw new HttpCallError('TOO_LARGE', `httpCall: response from ${url} exceeds ${maxBytes} bytes`, { status: response.status, attempts, url })
    }
    chunks.push(value)
  }
  const out = new Uint8Array(size)
  let at = 0
  for (const c of chunks) { out.set(c, at); at += c.byteLength }
  return out
}

/**
 * httpCall(url, {method, headers, json, body, timeout, retries, idempotencyKey, maxBytes, backoff, allowPrivate, signal})
 * → {status, ok, headers, text, json, attempts}. Network failures, deadlines, oversized bodies and final
 * retryable statuses (5xx, 408, 425, 429) throw HttpCallError; other statuses are returned.
 */
export async function httpCall(target, options = {}) {
  const method = String(options.method ?? 'GET').toUpperCase()
  const timeout = bounded(options.timeout, 'timeout', 1, MAX_TIMEOUT, 10000)
  const retries = bounded(options.retries, 'retries', 0, MAX_RETRIES, 2)
  const maxBytes = bounded(options.maxBytes, 'maxBytes', 1, MAX_BYTES, 1048576)
  const backoff = bounded(options.backoff, 'backoff', 1, 10000, 250)
  let url
  try { url = new URL(target) } catch { throw new HttpCallError('UNSAFE_URL', 'httpCall: an absolute http(s) URL is required') }
  const safe = redact(url)
  if (!['http:', 'https:'].includes(url.protocol) || url.username || url.password) throw new HttpCallError('UNSAFE_URL', `httpCall: ${safe} must be http(s) without credentials`, { url: safe })
  if (!options.allowPrivate && privateHost(url.hostname)) throw new HttpCallError('UNSAFE_URL', `httpCall: ${safe} targets a private or loopback address; pass allowPrivate for internal services`, { url: safe })

  const headers = new Headers(options.headers ?? {})
  let body = options.body
  if (options.json !== undefined) {
    body = JSON.stringify(options.json)
    if (!headers.has('content-type')) headers.set('content-type', 'application/json')
    if (!headers.has('accept')) headers.set('accept', 'application/json')
  }
  if (options.idempotencyKey) headers.set('idempotency-key', options.idempotencyKey === true ? globalThis.crypto.randomUUID() : String(options.idempotencyKey))
  const replayable = SAFE_METHODS.has(method) || headers.has('idempotency-key')
  const allowed = replayable ? retries : 0

  for (let attempt = 1; ; attempt++) {
    const signals = [AbortSignal.timeout(timeout), options.signal].filter(Boolean)
    let response, wait = null
    try {
      response = await fetch(url, { method, headers, body, redirect: 'manual', signal: AbortSignal.any(signals) })
      if (RETRY_STATUS.has(response.status)) {
        if (attempt > allowed) {
          await response.body?.cancel().catch(() => {})
          throw new HttpCallError('HTTP_STATUS', `httpCall: ${safe} answered ${response.status}`, { status: response.status, attempts: attempt, url: safe })
        }
        wait = retryAfter(response.headers.get('retry-after'))
        await response.body?.cancel().catch(() => {})
        response = null
      }
    } catch (error) {
      if (error instanceof HttpCallError) throw error
      if (options.signal?.aborted) throw new HttpCallError('ABORTED', `httpCall: request to ${safe} was cancelled`, { attempts: attempt, url: safe })
      const timedOut = error?.name === 'TimeoutError' || error?.name === 'AbortError'
      if (attempt > allowed) throw new HttpCallError(timedOut ? 'TIMEOUT' : 'NETWORK', `httpCall: ${safe} ${timedOut ? `did not answer within ${timeout} ms` : 'is unreachable'}`, { attempts: attempt, url: safe })
    }
    if (response) {
      const bytes = await readBody(response, maxBytes, safe, attempt)
      const text = new TextDecoder().decode(bytes)
      let json = null
      if ((response.headers.get('content-type') ?? '').includes('json') && text) {
        try { json = JSON.parse(text) } catch { json = null }
      }
      return { status: response.status, ok: response.ok, headers: Object.fromEntries(response.headers), text, json, attempts: attempt }
    }
    await sleep(wait ?? Math.random() * Math.min(5000, backoff * 2 ** (attempt - 1)))
  }
}
