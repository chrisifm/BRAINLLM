// ctx.backend for job and route handlers (docs/backend/job-backend.md, routes.md): the native resource and file APIs
// over the fixed loopback BRAIN_BACKEND_URL, authenticated by one project API key. The key's scopes and identity decide
// what the handler may read or write; the option mapping matches the runtime's ctx.backend.
// A hoisted function with everything inside: the route sidecar's top-level loop starts before later consts initialize.
// `observe(error, path)`, when given, hears every failure the backend answered (or never answered), even one the
// handler later catches: the job runner reports those for a job that still succeeds.
function keyBackend(env, keyName, kind, observe) {
  const LIST_OPTIONS = ['search', 'limit', 'offset', 'filter', 'sort', 'cursor', 'after', 'include', 'aggregate', 'count', 'facets', 'fields', 'owner']
  const FILTER_OPS = ['eq', 'ne', 'gt', 'gte', 'lt', 'lte', 'between', 'in', 'exists', 'prefix', 'iprefix', 'has']
  const UUID = /^[a-f0-9]{8}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{12}$/i
  const fail = (status, message, extra) => Object.defineProperty(Object.assign(new Error(message), { status }, extra), Symbol.for('brain.backendError'), { value: true })
  const answered = (path, error) => { try { observe?.(error, path) } catch {} return error }
  let target = null, problem = null
  const key = env[keyName] || ''
  try {
    if (!env.BRAIN_BACKEND_URL || !key) throw Error()
    target = new URL(env.BRAIN_BACKEND_URL)
    if (target.protocol !== 'http:' || !['127.0.0.1', '[::1]', 'localhost'].includes(target.hostname) || target.username || target.password || target.pathname !== '/' || target.search || target.hash) problem = `BRAIN_BACKEND_URL must be an explicit loopback HTTP origin for ${kind} handlers`
    else if (!/^bk_live_[a-f0-9]{12}_[a-f0-9]{64}$/.test(key)) problem = `${keyName} must be a project API key (bk_live_<prefix>_<secret>)`
  } catch { problem = `Set BRAIN_BACKEND_URL and ${keyName} (a scoped project API key) to use ctx.backend in ${kind} handlers` }
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
    } catch { throw answered(path, fail(503, 'Native backend is unavailable')) }
    if (Number(response.headers.get('content-length')) > 2 * 1024 * 1024) { await response.body?.cancel(); throw answered(path, fail(502, 'Native resource response exceeds its budget')) }
    const raw = Buffer.from(await response.arrayBuffer())
    if (raw.length > 2 * 1024 * 1024) throw answered(path, fail(502, 'Native resource response exceeds its budget'))
    let value = null
    if (raw.length) { try { value = JSON.parse(raw.toString('utf8')) } catch { if (response.ok) throw answered(path, fail(502, 'Native resource response is invalid')) } }
    if (response.status === 404 && notFound) return notFound()
    if (response.status === 422) {
      const fields = value?.error?.fields
      const errors = fields && typeof fields === 'object' && !Array.isArray(fields) ? Object.fromEntries(Object.entries(fields).filter(([k, v]) => typeof v === 'string' && k.length <= 128).slice(0, 64).map(([k, v]) => [k, v.slice(0, 512)])) : {}
      throw answered(path, fail(422, 'Invalid input', { errors }))
    }
    if (!response.ok) {
      const code = typeof value?.error?.code === 'string' && /^[A-Z][A-Z0-9_]{0,63}$/.test(value.error.code) ? value.error.code : undefined
      throw answered(path, fail(response.status >= 400 && response.status < 500 ? response.status : 503, `Native backend request failed with ${response.status}`, { code }))
    }
    return response.status === 204 ? null : value
  }
  const names = (value, label) => {
    const list = typeof value === 'string' ? value.split(',') : value
    const shape = label === 'include' ? /^[a-z][a-z0-9_]{0,47}(?:\.[a-z][a-z0-9_]{0,47}){0,2}$/ : /^[a-z][a-z0-9_]{0,47}$/
    if (!Array.isArray(list) || list.length > 16 || list.some(item => typeof item !== 'string' || !shape.test(item))) throw fail(400, `Invalid native ${label} names`)
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
    const idPath = id => { if (typeof id !== 'string' || !UUID.test(id)) throw fail(404, 'No record has this id; record ids are UUIDs'); return '/api/' + name + '/' + id }
    return Object.freeze({
      list(options = {}) {
        if (!options || typeof options !== 'object' || Array.isArray(options) || Object.keys(options).some(k => !LIST_OPTIONS.includes(k))) throw fail(400, 'Invalid native list options')
        const { search, limit, offset, filter, sort, cursor, after, include, aggregate, count, facets, fields } = options
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
        for (const [k, value] of [['include', include], ['aggregate', aggregate], ['count', count], ['facets', facets], ['fields', fields]]) if (value !== undefined) q.set(k, names(value, k))
        if (options.owner !== undefined) {
          if (options.owner !== 'me' && !(typeof options.owner === 'string' && UUID.test(options.owner))) throw fail(400, "Invalid native list owner; use 'me' or an account id")
          q.set('owner', options.owner)
        }
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
      // a declared atomic action or state transition (docs/backend/atomic-updates.md); the key still needs `<resource>:write`
      action: (id, action, input = {}) => request('POST', idPath(id) + '/actions/' + encodeURIComponent(action), input),
      transition: (id, transition, input = {}) => request('POST', idPath(id) + '/transitions/' + encodeURIComponent(transition), input),
      remove: id => request('DELETE', idPath(id)),
      slugRedirect(slug) {
        if (typeof slug !== 'string' || !/^[A-Za-z0-9._~-]{1,512}$/.test(slug)) throw fail(400, 'Invalid native slug')
        return request('GET', '/api/' + name + '/redirects/' + slug, undefined, () => null).then(value => typeof value?.slug === 'string' ? value.slug : null)
      },
    })
  }
  // A declared bucket, under `files.<bucket>:read|write` on the key and the bucket's own grants (docs/backend/files.md).
  const files = bucket => {
    if (problem) throw fail(503, problem)
    if (typeof bucket !== 'string' || !/^[a-z][a-z0-9_]{0,47}$/.test(bucket)) throw fail(400, 'Invalid native bucket name')
    const base = '/_brain/files/' + bucket
    const idPath = id => { if (typeof id !== 'string' || !UUID.test(id)) throw fail(404, 'No file has this id; file ids are UUIDs'); return base + '/' + id }
    const send = async (method, path, body, timeout) => {
      let response
      try {
        response = await fetch(new URL(path, target), { method, redirect: 'error', signal: AbortSignal.timeout(timeout), headers: { 'x-api-key': key }, body })
      } catch { throw answered(path, fail(503, 'Native backend is unavailable')) }
      if (response.ok) return response
      await response.body?.cancel()
      throw answered(path, fail(response.status >= 400 && response.status < 500 ? response.status : 503, `Native file request failed with ${response.status}`))
    }
    const json = async response => { try { return await response.json() } catch { throw fail(502, 'Native file response is invalid') } }
    return Object.freeze({
      async list(options = {}) {
        if (!options || typeof options !== 'object' || Array.isArray(options) || Object.keys(options).some(k => !['limit', 'offset', 'record'].includes(k))) throw fail(400, 'Invalid native file list options')
        const { limit, offset, record } = options
        if ((limit !== undefined && (!Number.isInteger(limit) || limit < 1 || limit > 100)) || (offset !== undefined && (!Number.isInteger(offset) || offset < 0 || offset > 10000)) || (record !== undefined && !UUID.test(String(record)))) throw fail(400, 'Invalid native file list options')
        const q = new URLSearchParams(Object.entries({ limit, offset, record }).filter(([, v]) => v !== undefined))
        return json(await send('GET', base + (q.size ? '?' + q : ''), undefined, 10000))
      },
      async meta(id) { return json(await send('GET', idPath(id) + '/meta', undefined, 10000)) },
      async read(id, { maxBytes = 16 * 1024 * 1024 } = {}) {
        if (!Number.isInteger(maxBytes) || maxBytes < 1 || maxBytes > 64 * 1024 * 1024) throw fail(400, 'Invalid native file read limit')
        const response = await send('GET', idPath(id), undefined, 30000)
        if (Number(response.headers.get('content-length')) > maxBytes) { await response.body?.cancel(); throw fail(413, 'The file exceeds the read limit; raise maxBytes up to 64 MiB') }
        const bytes = Buffer.from(await response.arrayBuffer())
        if (bytes.length > maxBytes) throw fail(413, 'The file exceeds the read limit; raise maxBytes up to 64 MiB')
        return { bytes, contentType: response.headers.get('content-type'), etag: response.headers.get('etag') }
      },
      async upload({ filename, contentType = 'application/octet-stream', data, record } = {}) {
        if (typeof filename !== 'string' || !filename || Buffer.byteLength(filename) > 255 || typeof contentType !== 'string' || (record !== undefined && !UUID.test(String(record)))) throw fail(400, 'Invalid native file upload')
        if (typeof data !== 'string' && !(data instanceof Uint8Array)) throw fail(400, 'File data must be a string, Buffer or Uint8Array')
        const form = new FormData()
        form.set('file', new Blob([data], { type: contentType }), filename)
        return json(await send('POST', base + (record === undefined ? '' : '?record=' + record), form, 60000))
      },
      async remove(id) { await send('DELETE', idPath(id), undefined, 10000); return null },
    })
  }
  // A declared operation (docs/backend/transactions.md): every step in one transaction, under the operation's roles.
  const operation = (name, input = {}) => {
    if (problem) throw fail(503, problem)
    if (typeof name !== 'string' || !/^[a-z][a-z0-9_]{0,47}$/.test(name)) throw fail(400, 'Invalid operation name')
    return request('POST', '/_brain/operations/' + name, input)
  }
  return Object.freeze({ resource, files, operation, slugRedirect: (name, slug) => resource(name).slugRedirect(slug) })
}
