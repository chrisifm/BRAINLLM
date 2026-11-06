// Optional public SSR cache. Standalone Redis only; no sessions, locks, socket fanout or cluster coordination.
// RESP2, one bounded operation per owned connection, no pending queue, no retries and no provider dependencies.
import net from 'node:net'
import tls from 'node:tls'
import dns from 'node:dns'
import { validateHeaderValue } from 'node:http'
import { createHash } from 'node:crypto'

const ENTRY_BYTES = 1024 * 1024, MAX_ACTIVE = 4, DEADLINE_MS = 250, MAX_TTL_MS = 86400000
const digest = value => createHash('sha256').update(value).digest('hex')
const SAFE_REQUEST = new Set(['host', 'connection', 'accept', 'accept-encoding', 'accept-language', 'user-agent', 'if-none-match', 'if-modified-since', 'cache-control', 'pragma', 'upgrade-insecure-requests', 'sec-fetch-site', 'sec-fetch-mode', 'sec-fetch-dest', 'sec-fetch-user', 'sec-ch-ua', 'sec-ch-ua-mobile', 'sec-ch-ua-platform'])
const SAFE_RESPONSE = new Set(['content-type', 'content-language', 'cache-control', 'link', 'x-robots-tag', 'cross-origin-opener-policy', 'content-security-policy'])
const privateControl = value => /(?:^|,)\s*(?:private|no-store|no-cache)(?:\s|=|,|$)/i.test(String(value || ''))

/** Unknown application headers can carry identity. They bypass rather than becoming cache-key material. */
export function publicCacheRequest(req) {
  if (!req || !['GET', 'HEAD'].includes(req.method) || !req.headers || privateControl(req.headers['cache-control']) || /no-cache/i.test(req.headers.pragma || '')) return false
  for (const key of Object.keys(req.headers)) if (!SAFE_REQUEST.has(key.toLowerCase())) return false
  return true
}
function settings(raw, namespace, build, base) {
  if (typeof namespace !== 'string' || !/^[A-Za-z0-9][A-Za-z0-9:_-]{0,63}$/.test(namespace) || typeof build !== 'string' || !build || build.length > 256 || typeof base !== 'string' || base.length > 2048 || typeof raw !== 'string' || raw.length > 4096) return null
  try {
    const url = new URL(raw)
    if (!['redis:', 'rediss:'].includes(url.protocol) || !url.hostname || url.search || url.hash || !/^\/(?:\d{1,4})?$/.test(url.pathname || '/') || Number(url.pathname.slice(1)) > 1023) return null
    const user = decodeURIComponent(url.username), password = decodeURIComponent(url.password)
    if (Buffer.byteLength(user) > 128 || Buffer.byteLength(password) > 1024 || user && !password) return null
    if (url.port && Number(url.port) < 1) return null
    const host = url.hostname.replace(/^\[|\]$/g, '')
    return { host, port: Number(url.port || 6379), secure: url.protocol === 'rediss:', user, password, db: Number(url.pathname.slice(1) || 0), prefix: `brain:ssr:v1:${digest(namespace)}:${digest(JSON.stringify([build, base]))}:` }
  } catch { return null }
}
function frame(parts) {
  const chunks = [Buffer.from(`*${parts.length}\r\n`)]
  for (const part of parts) { const bytes = Buffer.isBuffer(part) ? part : Buffer.from(String(part)); chunks.push(Buffer.from(`$${bytes.length}\r\n`), bytes, Buffer.from('\r\n')) }
  return Buffer.concat(chunks)
}
// Only replies required by AUTH/SELECT/GET/SET/DEL. Unexpected aggregates/protocol types are a miss.
function reply(buffer) {
  const end = buffer.indexOf('\r\n')
  if (end < 0) return buffer.length > 1024 ? false : undefined
  if (end > 1024) return false
  // latin1 preserves every high bit; ascii decoding masks malformed bytes into valid digits or OK.
  const type = buffer[0], text = buffer.toString('latin1', 1, end)
  if (type === 36) {
    if (text === '-1') return buffer.length === end + 2 ? { value: null } : false
    if (!/^(?:0|[1-9]\d{0,6})$/.test(text)) return false
    const size = Number(text)
    if (size > ENTRY_BYTES) return false
    const full = end + 2 + size + 2
    if (buffer.length < full) return undefined
    if (buffer.length !== full || buffer[full - 2] !== 13 || buffer[full - 1] !== 10) return false
    return { value: Buffer.from(buffer.subarray(end + 2, full - 2)) }
  }
  if (buffer.length !== end + 2) return false
  if (type === 43 && text === 'OK') return { value: 'OK' }
  if (type === 58 && /^(?:0|1)$/.test(text)) return { value: Number(text) }
  return false // Errors include OOM/noeviction, NOAUTH, NOPERM and MOVED: no body/value is logged.
}
function publicHeaders(headers) {
  if (!headers || typeof headers !== 'object' || Array.isArray(headers) || Object.getPrototypeOf(headers) !== Object.prototype) return false
  let bytes = 0
  const entries = Object.entries(headers)
  if (entries.length > 32) return false
  for (const [key, value] of entries) {
    if (!SAFE_RESPONSE.has(key.toLowerCase()) || typeof value !== 'string' || /[\r\n\x00]/.test(value) || value.length > 4096) return false
    try { validateHeaderValue(key, value) } catch { return false }
    if (key.toLowerCase() === 'cache-control' && privateControl(value)) return false
    bytes += Buffer.byteLength(key) + Buffer.byteLength(value)
  }
  return bytes <= 16384
}
const lifetime = seconds => Number.isInteger(seconds) && seconds > 0 && seconds <= Number.MAX_SAFE_INTEGER / 2000 ? Math.min(seconds * 2000, MAX_TTL_MS) : 0
function record(value, seconds) {
  if (!value || value.v !== 1 || value.seconds !== seconds || typeof value.out !== 'string' || Buffer.byteLength(value.out) > ENTRY_BYTES || !publicHeaders(value.h) || !Number.isSafeInteger(value.t)) return null
  const age = Date.now() - value.t
  if (age < 0 || age >= lifetime(seconds)) return null
  return { out: value.out, h: value.h, t: value.t, bytes: Buffer.byteLength(value.out), cacheable: true, etag: 'W/"' + createHash('sha1').update(value.out).digest('base64url').slice(0, 22) + '"' }
}

/** Both environment variables and a real build identity are required. Invalid/partial opt-ins stay disabled. */
export function createRedisCache({ url, namespace, build, base = '/' } = {}) {
  if (!url || !namespace) return null
  const config = settings(url, namespace, build, base)
  if (!config) return null
  const owned = new Set()
  // libc/libuv DNS cannot be canceled by destroying a socket. A timed-out lookup keeps its admission
  // lease until its callback returns, so repeated misses cannot queue unlimited resolver work.
  const leases = new Set()
  let stopped = false
  const key = identity => typeof identity === 'string' && identity.length <= 16384 ? config.prefix + digest(identity) : null
  async function command(parts) {
    if (stopped || leases.size >= MAX_ACTIVE) return null
    const lease = { lookups: 0 }
    leases.add(lease)
    let socket, finish, receive, offset = 0, done = false
    let buffer = Buffer.allocUnsafe(4096)
    const completed = new Promise(resolve => { finish = value => {
      if (done) return
      done = true; clearTimeout(timer); const pending = receive; receive = null; pending?.(null)
      if (socket) { owned.delete(socket); socket.destroy() }
      if (!lease.lookups) leases.delete(lease)
      resolve(value)
    } })
    const timer = setTimeout(() => finish(null), DEADLINE_MS)
    const exchange = values => new Promise(resolve => {
      if (done || stopped) return resolve(null)
      offset = 0; receive = resolve
      socket.write(frame(values), error => { if (error) finish(null) })
    })
    const lookup = (host, options, callback) => {
      lease.lookups++
      try {
        dns.lookup(host, options, (...args) => {
          lease.lookups--
          if (done || stopped) {
            if (!lease.lookups) leases.delete(lease)
            callback(Object.assign(new Error('Cache lookup expired'), { code: 'ECANCELED' }))
          } else callback(...args)
        })
      } catch (error) { lease.lookups--; throw error }
    }
    try {
      socket = config.secure
        ? tls.connect({ host: config.host, port: config.port, lookup, servername: net.isIP(config.host) ? undefined : config.host, rejectUnauthorized: true, checkServerIdentity: (_host, cert) => tls.checkServerIdentity(config.host, cert) })
        : net.createConnection({ host: config.host, port: config.port, lookup })
      owned.add(socket)
      socket.on('error', () => finish(null)); socket.on('end', () => finish(null)); socket.on('close', () => finish(null))
      socket.on('data', chunk => {
        if (done) return
        if (!receive || offset + chunk.length > ENTRY_BYTES + 1028) return finish(null)
        if (offset + chunk.length > buffer.length) {
          const grown = Buffer.allocUnsafe(Math.min(ENTRY_BYTES + 1028, Math.max(offset + chunk.length, buffer.length * 2)))
          buffer.copy(grown, 0, 0, offset); buffer = grown
        }
        chunk.copy(buffer, offset); offset += chunk.length
        const parsed = reply(buffer.subarray(0, offset))
        if (parsed === false) return finish(null)
        if (parsed) { const resolve = receive; receive = null; resolve(parsed.value) }
      })
      socket.once(config.secure ? 'secureConnect' : 'connect', async () => {
        try {
          if (config.password && await exchange(config.user ? ['AUTH', config.user, config.password] : ['AUTH', config.password]) !== 'OK') return finish(null)
          if (config.db && await exchange(['SELECT', config.db]) !== 'OK') return finish(null)
          finish(await exchange(parts))
        } catch { finish(null) }
      })
    } catch { finish(null) }
    return completed
  }
  return {
    async get(identity, seconds) {
      const target = key(identity)
      if (!target || !lifetime(seconds)) return null
      const body = await command(['GET', target])
      if (!Buffer.isBuffer(body) || body.length > ENTRY_BYTES) return null
      try { return record(JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(body)), seconds) } catch { return null }
    },
    async set(identity, value, seconds) {
      const target = key(identity)
      if (!target || !value?.cacheable || !lifetime(seconds)) return false
      const envelope = { v: 1, out: value.out, h: value.h, t: value.t, seconds }
      if (!record(envelope, seconds)) return false
      let body
      try { body = JSON.stringify(envelope) } catch { return false }
      const remaining = lifetime(seconds) - (Date.now() - value.t)
      if (Buffer.byteLength(body) > ENTRY_BYTES || remaining < 1) return false
      return await command(['SET', target, body, 'PX', remaining]) === 'OK'
    },
    async delete(identity) { const target = key(identity); if (target) await command(['DEL', target]) },
    close() { stopped = true; for (const socket of owned) socket.destroy(); owned.clear() },
  }
}
