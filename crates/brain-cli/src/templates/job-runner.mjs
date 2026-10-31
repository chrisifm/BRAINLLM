// One trusted-project job per process. Report structural locations only, never payloads, env or exception text.
import { pathToFileURL, fileURLToPath } from 'node:url'
import { dirname, basename, resolve } from 'node:path'
import { readFileSync, statSync, realpathSync } from 'node:fs'
import { findSourceMap } from 'node:module'

const LIMIT = { frames: 24, causes: 6, indexBytes: 262144, indexEntries: 2048, outputBytes: 32768 }
let entry = resolve(process.argv[1])
try { entry = realpathSync(entry) } catch {}
const directory = dirname(entry)
const emit = process.stderr.write.bind(process.stderr)
Error.stackTraceLimit = LIMIT.frames + 1
const safePath = value => typeof value === 'string' && value.length <= 512 && !/^[\\/]|[\\?#\x00-\x1f]/.test(value) && value.split('/').every(part => part && part !== '.' && part !== '..')
const safeGenerated = value => typeof value === 'string' && /^[a-z0-9_-]{1,80}\.mjs$/i.test(value)
let modules = {}, indexUnavailable = false
try {
  const file = resolve(directory, 'trace-index.json')
  if (statSync(file).size > LIMIT.indexBytes) throw Error()
  const index = JSON.parse(readFileSync(file, 'utf8'))
  if (index.schemaVersion !== 1 || !index.modules || typeof index.modules !== 'object' || Array.isArray(index.modules) || Object.keys(index.modules).length > LIMIT.indexEntries) throw Error()
  modules = Object.fromEntries(Object.entries(index.modules).filter(([generated, source]) => safeGenerated(generated) && safePath(source)))
} catch { indexUnavailable = true }
const property = (value, name) => { try { return value?.[name] } catch { return null } }
const coordinate = value => Number.isSafeInteger(value) && value > 0 && value <= 10000000 ? value : null
function fileOf(value) {
  try {
    if (typeof value !== 'string') return null
    if (value.startsWith('file:')) { const url = new URL(value); url.search = ''; url.hash = ''; return fileURLToPath(url) }
    if (!value.startsWith('/')) return null
    return value.split(/[?#]/)[0]
  } catch { return null }
}
function fallback() {
  return { file: modules[basename(entry)] || null, line: null, column: null, precision: 'module-only', generated: null }
}
function frames(error, budget) {
  const saved = Object.getOwnPropertyDescriptor(Error, 'prepareStackTrace')
  let sites
  try {
    Object.defineProperty(Error, 'prepareStackTrace', { configurable: true, writable: true, value: (_error, structured) => structured })
    sites = property(error, 'stack')
  } catch { sites = null }
  finally { try { if (saved) Object.defineProperty(Error, 'prepareStackTrace', saved); else delete Error.prepareStackTrace } catch {} }
  const out = [], truncated = { frames: false, stackUnavailable: !Array.isArray(sites) }
  if (!Array.isArray(sites)) return { location: fallback(), frames: out, truncated }
  if (sites.length >= LIMIT.frames + 1) truncated.frames = true
  for (const site of sites.slice(0, LIMIT.frames + 1)) {
    try {
      const file = fileOf(site.getFileName())
      if (!file || dirname(file) !== directory) continue
      const generatedName = basename(file), source = modules[generatedName]
      if (!source) continue
      if (!budget.left) { truncated.frames = true; continue }
      const line = coordinate(site.getLineNumber()), column = coordinate(site.getColumnNumber())
      const generated = { file: `_server/${generatedName}`, line, column }
      let sourceLine = null
      if (line && column) {
        try {
          const map = findSourceMap(file), hit = map?.findEntry(line - 1, column - 1)
          // A different source or unmapped generated line is not proof of the app line in the private index.
          const original = typeof hit?.originalSource === 'string' ? decodeURIComponent(hit.originalSource).split(/[?#]/)[0] : ''
          if (hit?.generatedLine === line - 1 && original.endsWith('/' + source)) sourceLine = coordinate(hit.originalLine + 1)
        } catch { truncated.stackUnavailable = true }
      }
      out.push({ file: source, line: sourceLine, column: null, precision: sourceLine ? 'source-line' : 'module-only', generated })
      budget.left--
    } catch { truncated.stackUnavailable = true }
  }
  return { location: out[0] || fallback(), frames: out, truncated }
}
function trace(error, jobId) {
  const budget = { left: LIMIT.frames }, root = frames(error, budget), causes = [], seen = new Set([error])
  let next = property(error, 'cause'), causeTruncated = false
  while (next != null) {
    if (seen.has(next) || causes.length >= LIMIT.causes) { causeTruncated = true; break }
    seen.add(next)
    causes.push({ depth: causes.length + 1, ...frames(next, budget) })
    next = property(next, 'cause')
  }
  const result = { schemaVersion: 1, event: 'job-trace', jobId, location: root.location, frames: root.frames, causes, truncated: { frames: root.truncated.frames || causes.some(c => c.truncated.frames), causes: causeTruncated, stackUnavailable: root.truncated.stackUnavailable, indexUnavailable, output: false } }
  while (Buffer.byteLength(JSON.stringify(result)) > LIMIT.outputBytes && result.causes.length) { result.causes.pop(); result.truncated.output = result.truncated.causes = true }
  while (Buffer.byteLength(JSON.stringify(result)) > LIMIT.outputBytes && result.frames.length > 1) { result.frames.pop(); result.truncated.output = result.truncated.frames = true }
  return result
}
// ctx.backend for job handlers (docs/backend/job-backend.md): the native resource API over the fixed loopback
// BRAIN_BACKEND_URL, authenticated by the project API key in BRAIN_JOBS_API_KEY. The key's scopes and its owner's
// policy decide what a job may read or write; the option mapping matches the runtime's ctx.backend.
const LIST_OPTIONS = ['search', 'limit', 'offset', 'filter', 'sort', 'cursor', 'after', 'include', 'aggregate', 'count']
const FILTER_OPS = ['eq', 'ne', 'gt', 'gte', 'lt', 'lte', 'between', 'in', 'exists', 'prefix', 'iprefix', 'has']
const UUID = /^[a-f0-9]{8}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{12}$/i
const fail = (status, message, extra) => Object.assign(new Error(message), { status }, extra)
function jobBackend(env) {
  let target = null, problem = null
  const key = env.BRAIN_JOBS_API_KEY || ''
  try {
    if (!env.BRAIN_BACKEND_URL || !key) throw Error()
    target = new URL(env.BRAIN_BACKEND_URL)
    if (target.protocol !== 'http:' || !['127.0.0.1', '[::1]', 'localhost'].includes(target.hostname) || target.username || target.password || target.pathname !== '/' || target.search || target.hash) problem = 'BRAIN_BACKEND_URL must be an explicit loopback HTTP origin for job handlers'
    else if (!/^bk_live_[a-f0-9]{12}_[a-f0-9]{64}$/.test(key)) problem = 'BRAIN_JOBS_API_KEY must be a project API key (bk_live_<prefix>_<secret>)'
  } catch { problem = 'Set BRAIN_BACKEND_URL and BRAIN_JOBS_API_KEY (a scoped project API key) to use ctx.backend in job handlers' }
  const request = async (method, path, body, notFound) => {
    let bytes
    if (body !== undefined) {
      if (!body || typeof body !== 'object' || Array.isArray(body)) throw fail(400, 'Native resource values must be an object')
      try { bytes = JSON.stringify(body) } catch { throw fail(400, 'Native resource values must be JSON serializable') }
      if (Buffer.byteLength(bytes) > 65536) throw fail(413, 'Native resource values exceed their byte budget')
    }
    let response
    try {
      response = await fetch(new URL(path, target), { method, redirect: 'error', signal: AbortSignal.timeout(10000), headers: { accept: 'application/json', 'x-api-key': key, ...(bytes ? { 'content-type': 'application/json' } : {}) }, body: bytes })
    } catch { throw fail(503, 'Native backend is unavailable') }
    if (Number(response.headers.get('content-length')) > 2 * 1024 * 1024) { await response.body?.cancel(); throw fail(502, 'Native resource response exceeds its budget') }
    const raw = Buffer.from(await response.arrayBuffer())
    if (raw.length > 2 * 1024 * 1024) throw fail(502, 'Native resource response exceeds its budget')
    let value = null
    if (raw.length) { try { value = JSON.parse(raw.toString('utf8')) } catch { if (response.ok) throw fail(502, 'Native resource response is invalid') } }
    if (response.status === 404 && notFound) return notFound()
    if (response.status === 422) {
      const fields = value?.error?.fields
      const errors = fields && typeof fields === 'object' && !Array.isArray(fields) ? Object.fromEntries(Object.entries(fields).filter(([k, v]) => typeof v === 'string' && k.length <= 128).slice(0, 64).map(([k, v]) => [k, v.slice(0, 512)])) : {}
      throw fail(422, 'Invalid input', { errors })
    }
    if (!response.ok) {
      const code = typeof value?.error?.code === 'string' && /^[A-Z][A-Z0-9_]{0,63}$/.test(value.error.code) ? value.error.code : undefined
      throw fail(response.status >= 400 && response.status < 500 ? response.status : 503, `Native backend request failed with ${response.status}`, { code })
    }
    return response.status === 204 ? null : value
  }
  const names = (value, label) => {
    const list = typeof value === 'string' ? value.split(',') : value
    if (!Array.isArray(list) || list.length > 16 || list.some(item => typeof item !== 'string' || !/^[a-z][a-z0-9_]{0,47}$/.test(item))) throw fail(400, `Invalid native ${label} names`)
    return list.join(',')
  }
  const scalar = value => {
    if (typeof value === 'number' && Number.isFinite(value)) return String(value)
    if (typeof value === 'boolean') return String(value)
    if (typeof value === 'string' && Buffer.byteLength(value) <= 256) return value
    throw fail(400, 'Invalid native filter value')
  }
  const resource = name => {
    if (problem) throw fail(503, problem)
    if (typeof name !== 'string' || !/^[a-z][a-z0-9_]{0,47}$/.test(name)) throw fail(400, 'Invalid native resource name')
    const idPath = id => { if (typeof id !== 'string' || !UUID.test(id)) throw fail(400, 'Invalid native record identifier'); return '/api/' + name + '/' + id }
    return Object.freeze({
      list(options = {}) {
        if (!options || typeof options !== 'object' || Array.isArray(options) || Object.keys(options).some(k => !LIST_OPTIONS.includes(k))) throw fail(400, 'Invalid native list options')
        const { search, limit, offset, filter, sort, cursor, after, include, aggregate, count } = options
        if ((search !== undefined && (typeof search !== 'string' || Buffer.byteLength(search) > 256)) || (limit !== undefined && (!Number.isInteger(limit) || limit < 1 || limit > 100)) || (offset !== undefined && (!Number.isInteger(offset) || offset < 0 || offset > 10000))) throw fail(400, 'Invalid native list pagination or search')
        const q = new URLSearchParams()
        if (filter !== undefined) {
          if (!filter || typeof filter !== 'object' || Array.isArray(filter) || Object.keys(filter).length > 32) throw fail(400, 'Invalid native list filter')
          for (const [field, condition] of Object.entries(filter)) {
            if (!/^[a-z][a-z0-9_]{0,47}(?:\.[a-z][a-z0-9_]{0,47})?$/.test(field)) throw fail(400, 'Invalid native list filter field')
            const ops = condition && typeof condition === 'object' && !Array.isArray(condition) ? Object.entries(condition) : [['eq', condition]]
            if (!ops.length) throw fail(400, 'Invalid native filter value')
            for (const [op, value] of ops) {
              if (!FILTER_OPS.includes(op)) throw fail(400, 'Invalid native filter operator')
              let raw
              if (op === 'in' || op === 'between' || op === 'has') {
                const items = Array.isArray(value) ? value : [value]
                if (!items.length || items.length > 50 || (op === 'between' && items.length !== 2)) throw fail(400, 'Invalid native filter value')
                raw = items.map(scalar)
                if (raw.some(item => item.includes(','))) throw fail(400, 'Invalid native filter value')
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
          if (!Array.isArray(keys) || !keys.length || keys.length > 4 || keys.some(k => typeof k !== 'string' || !/^-?[a-z][a-z0-9_]{0,47}$/.test(k))) throw fail(400, 'Invalid native list sort')
          q.set('sort', keys.join(','))
        }
        if (cursor !== undefined && after !== undefined) throw fail(400, 'Use either cursor or after')
        for (const [k, value] of [['cursor', cursor], ['after', after]]) if (value !== undefined) {
          if (typeof value !== 'string' || !/^[A-Za-z0-9_-]{1,2048}$/.test(value)) throw fail(400, 'Invalid native list cursor')
          q.set(k, value)
        }
        for (const [k, value] of [['include', include], ['aggregate', aggregate], ['count', count]]) if (value !== undefined) q.set(k, names(value, k))
        const query = q.toString()
        return request('GET', '/api/' + name + (query ? '?' + query : ''))
      },
      get(id, options = {}) {
        if (!options || typeof options !== 'object' || Array.isArray(options) || Object.keys(options).some(k => k !== 'include')) throw fail(400, 'Invalid native get options')
        const path = idPath(id)
        return request('GET', options.include === undefined ? path : path + '?' + new URLSearchParams({ include: names(options.include, 'include') }))
      },
      create: values => request('POST', '/api/' + name, values),
      update: (id, values) => request('PUT', idPath(id), values),
      async patch(id, values) {
        if (values && typeof values === 'object' && !Array.isArray(values) && !Object.keys(values).length) throw fail(422, 'Send at least one field', { errors: { body: 'Send at least one field to change' } })
        return request('PATCH', idPath(id), values)
      },
      remove: id => request('DELETE', idPath(id)),
      slugRedirect(slug) {
        if (typeof slug !== 'string' || !/^[A-Za-z0-9._~-]{1,512}$/.test(slug)) throw fail(400, 'Invalid native slug')
        return request('GET', '/api/' + name + '/redirects/' + slug, undefined, () => null).then(value => typeof value?.slug === 'string' ? value.slug : null)
      },
    })
  }
  return Object.freeze({ resource, slugRedirect: (name, slug) => resource(name).slugRedirect(slug) })
}
let jobId = null
try {
  let input = '', bytes = 0
  process.stdin.setEncoding('utf8')
  for await (const chunk of process.stdin) {
    bytes += Buffer.byteLength(chunk)
    if (bytes > 128 * 1024) throw Error()
    input += chunk
  }
  const ctx = JSON.parse(input)
  if (typeof ctx.jobId === 'string' && /^[a-f0-9]{8}-(?:[a-f0-9]{4}-){3}[a-f0-9]{12}$/i.test(ctx.jobId)) jobId = ctx.jobId
  const env = Object.assign({}, process.env)
  delete env.BRAIN_JOBS_API_KEY
  ctx.env = Object.freeze(env)
  ctx.backend = jobBackend(process.env)
  const module = await import(pathToFileURL(entry).href)
  if (typeof module.default !== 'function') throw Error()
  await module.default(ctx)
} catch (error) {
  process.exitCode = 1
  try { emit(`JOB_TRACE ${JSON.stringify(trace(error, jobId))}\n`) } catch {}
}
