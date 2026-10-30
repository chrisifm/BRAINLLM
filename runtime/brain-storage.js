// Native structured local data. Browser storage is untrusted, evictable and never an authentication authority.
const G = globalThis
const error = (code, fix, cause) => Object.assign(new Error(fix), { code, fix, cause })
const identifier = n => typeof n === 'string' && /^[a-zA-Z0-9][a-zA-Z0-9._-]{0,79}$/.test(n)

export async function storageInfo() {
  const s = G.navigator?.storage
  let estimate = null, persisted = false
  try { estimate = s?.estimate ? await s.estimate() : null; persisted = s?.persisted ? await s.persisted() : false } catch {}
  return { indexedDB: !!G.indexedDB, opfs: !!s?.getDirectory, persisted,
    usage: Number.isFinite(estimate?.usage) ? estimate.usage : null,
    quota: Number.isFinite(estimate?.quota) ? estimate.quota : null }
}

// Request only from a user action; browsers may refuse. Never promise permanent storage.
export async function persistStorage() {
  return G.navigator?.storage?.persist ? !!(await G.navigator.storage.persist()) : false
}

export function database(name, options = {}) {
  if (!identifier(name)) throw error('B0913', 'database: use a stable alphanumeric name of at most 80 characters')
  const namespace = options.namespace || 'app'
  if (!identifier(namespace)) throw error('B0913', 'database: use an explicit stable application namespace')
  const build = options.build == null ? '' : String(options.build)
  if (build.length > 128) throw error('B0913', 'database: build identifiers must be at most 128 characters')
  let opening = null, connection = null, closed = false, pending = 0
  const active = new Set()
  const deadline = Math.max(100, Math.min(30000, Number(options.timeout) || 10000))
  const fail = e => error(e?.name === 'QuotaExceededError' ? 'B0914' : 'B0913',
    e?.name === 'QuotaExceededError' ? 'Local storage quota exceeded; evict owned cache records or reduce stored data' : 'Local database unavailable; keep authoritative data on the server and retry the operation', e)
  function open() {
    if (closed) return Promise.reject(error('B0913', 'database: connection was closed; create a new handle'))
    if (opening) return opening
    if (!G.indexedDB) return Promise.reject(error('B0913', 'IndexedDB is unavailable; use a server-backed data source'))
    opening = new Promise((resolve, reject) => {
      let settled = false
      const r = G.indexedDB.open(`brain:db:${namespace}:${name}`, 1)
      const timer = setTimeout(() => done(error('B0913', 'Local database open timed out; close an older tab holding an upgrade lock')), deadline)
      const done = e => { if (!settled) { settled = true; clearTimeout(timer); reject(e) } }
      r.onupgradeneeded = () => { if (!r.result.objectStoreNames.contains('records')) r.result.createObjectStore('records') }
      r.onblocked = () => done(error('B0913', 'Local database upgrade blocked; close older tabs and retry'))
      r.onerror = () => done(fail(r.error))
      r.onsuccess = () => {
        if (settled || closed) { r.result.close(); return }
        settled = true; clearTimeout(timer)
        connection = r.result
        connection.onversionchange = () => { connection?.close(); connection = null; opening = null }
        connection.onclose = () => { connection = null; opening = null }
        resolve(connection)
      }
    }).catch(e => { opening = null; throw e })
    return opening
  }
  function key(k) {
    if (typeof k !== 'string' || !k.length || k.length > 256) throw error('B0913', 'database: keys must contain between 1 and 256 characters')
    return k
  }
  function record(value, ttl) {
    if (!Number.isFinite(ttl) || ttl < 0 || ttl > 31536000000) throw error('B0913', 'database: ttl must be between zero and one year in milliseconds')
    let bytes, json
    try { json = JSON.stringify(value); if (json === undefined) throw new TypeError('undefined'); bytes = new TextEncoder().encode(json).length } catch (e) { throw error('B0913', 'database: values must be JSON-serializable', e) }
    if (bytes > 1048576) throw error('B0914', 'database: split records larger than 1 MiB or use OPFS for large files')
    return { v: 1, build, expires: ttl ? Date.now() + ttl : 0, value: JSON.parse(json) }
  }
  const valid = r => r?.v === 1 && r.build === build && (!r.expires || r.expires > Date.now())
  async function transaction(mode, work) {
    if (pending >= 32) throw error('B0914', 'database: await pending local operations before scheduling more work')
    pending++
    try {
      const db = await open()
      if (closed) throw error('B0913', 'database: handle was closed')
      return await new Promise((resolve, reject) => {
        const tx = db.transaction('records', mode), store = tx.objectStore('records')
        active.add(tx)
        let result, reason
        const timer = setTimeout(() => { reason = error('B0913', 'Local transaction timed out; reduce the batch size'); try { tx.abort() } catch {} }, deadline)
        tx.oncomplete = () => { clearTimeout(timer); active.delete(tx); resolve(result) }
        tx.onabort = tx.onerror = () => { clearTimeout(timer); active.delete(tx); reject(reason || fail(tx.error)) }
        try { work(store, v => { result = v }) } catch (e) { reason = e; try { tx.abort() } catch { clearTimeout(timer); reject(e) } }
      })
    } catch (e) { throw e?.code ? e : fail(e) } finally { pending-- }
  }
  const api = {
    get(k) { key(k); return transaction('readonly', (s, done) => { const r = s.get(k); r.onsuccess = () => done(valid(r.result) ? r.result.value : null) }) },
    put(k, value, o = {}) { key(k); const r = record(value, o.ttl ?? 0); return transaction('readwrite', s => s.put(r, k)) },
    delete(k) { key(k); return transaction('readwrite', s => s.delete(k)) },
    clear() { return transaction('readwrite', s => s.clear()) },
    batch(operations) {
      if (!Array.isArray(operations) || operations.length > 128) throw error('B0914', 'database: atomic batches accept at most 128 operations')
      let bytes = 0
      const ops = operations.map(o => {
        key(o.key)
        if (o.op === 'delete') return { op: 'delete', key: o.key }
        if (o.op !== 'put') throw error('B0913', 'database: batch operations must be put or delete')
        const r = record(o.value, o.ttl ?? 0)
        bytes += new TextEncoder().encode(JSON.stringify(r)).length
        if (bytes > 4194304) throw error('B0914', 'database: atomic batch exceeds 4 MiB')
        return { op: 'put', key: o.key, record: r }
      })
      return transaction('readwrite', s => { for (const o of ops) o.op === 'put' ? s.put(o.record, o.key) : s.delete(o.key) })
    },
    scan(o = {}) {
      const limit = Math.max(1, Math.min(200, Number(o.limit) || 50)), prefix = o.prefix || '', after = o.after || ''
      if (typeof prefix !== 'string' || prefix.length > 256 || typeof after !== 'string' || after.length > 256) throw error('B0913', 'database: scan prefixes and cursors must be strings of at most 256 characters')
      return transaction('readonly', (s, done) => {
        const items = [], r = s.openCursor(after ? G.IDBKeyRange.lowerBound(after, true) : prefix ? G.IDBKeyRange.lowerBound(prefix) : null)
        let visited = 0, bytes = 0, previous = null
        r.onsuccess = () => {
          const c = r.result
          if (!c) return done({ items, cursor: null })
          const k = String(c.key)
          if (prefix && k < prefix) { c.continue(prefix); return }
          if (prefix && !k.startsWith(prefix)) return done({ items, cursor: null })
          visited++
          if (valid(c.value)) {
            bytes += new TextEncoder().encode(JSON.stringify(c.value.value)).length
            if (bytes > 2097152) return done({ items, cursor: previous })
            items.push({ key: k, value: c.value.value })
          }
          previous = k
          if (items.length >= limit || visited >= 1000) return done({ items, cursor: k })
          c.continue()
        }
      })
    },
    prune() {
      return transaction('readwrite', (s, done) => {
        let visited = 0, deleted = 0
        const r = s.openCursor()
        r.onsuccess = () => {
          const c = r.result
          if (!c || visited >= 1000) return done({ visited, deleted, truncated: !!c })
          visited++
          if (!valid(c.value)) { c.delete(); deleted++ }
          c.continue()
        }
      })
    },
    close() { closed = true; for (const tx of active) { try { tx.abort() } catch {} }; connection?.close(); connection = null },
    info: storageInfo,
  }
  return api
}
