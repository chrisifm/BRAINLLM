// Official SQLite Wasm adapter for a project-owned worker. IndexedDB remains the dependency-free default.
const G = globalThis
const fault = (code, fix) => Object.assign(new Error(fix), { code, fix })
const nameOK = x => typeof x === 'string' && /^[a-zA-Z0-9][a-zA-Z0-9_-]{0,47}$/.test(x)

export function sqlite(initialize, options = {}) {
  const name = options.name || 'app', namespace = options.namespace || 'app'
  if (!nameOK(name) || !nameOK(namespace) || typeof initialize !== 'function') throw fault('B0916', 'sqlite: provide the official initializer and stable names')
  let opening, db, pool, release, closed = false, pending = 0, chain = Promise.resolve()
  const lease = new Promise(resolve => { release = resolve })
  async function open() {
    if (closed) throw fault('B0916', 'Local SQL handle is closed; create a new worker')
    if (opening) return opening
    if (G.document || !G.navigator?.storage?.getDirectory || !G.navigator?.locks?.request || typeof G.postMessage !== 'function') throw fault('B0916', 'Persistent SQLite needs an owned worker with OPFS and Web Locks; use database() when unavailable')
    opening = new Promise((resolve, reject) => {
      G.navigator.locks.request(`brain:sqlite:${namespace}:${name}`, { ifAvailable: true }, async lock => {
        if (!lock) throw fault('B0916', 'This local SQL database is open in another worker; close its handle before retrying')
        let stage = 'official SDK initialization'
        try {
          const engine = await initialize({ print() {}, printErr() {} })
          if (closed) throw fault('B0916', 'Local SQL handle closed while initializing')
          if (!engine.capi || engine.capi.sqlite3_libversion_number() < 3054000) throw fault('B0916', 'SQLite 3.54 or newer is required for non-destructive OPFS initialization')
          stage = 'non-destructive OPFS initialization'
          pool = await engine.installOpfsSAHPoolVfs({ name: `brain-${namespace}-${name}`, directory: `/brain-sqlite/${namespace}/${name}`, initialCapacity: 6, clearOnInit: false, preserveOnInitFailure: true })
          stage = 'OPFS database open'
          db = new pool.OpfsSAHPoolDb(`/${name}.sqlite3`)
          stage = 'SQLite resource limits'
          const c = engine.capi
          for (const [constant, value] of [['SQLITE_LIMIT_LENGTH',1048576],['SQLITE_LIMIT_SQL_LENGTH',65536],['SQLITE_LIMIT_COLUMN',128],['SQLITE_LIMIT_EXPR_DEPTH',100],['SQLITE_LIMIT_COMPOUND_SELECT',16],['SQLITE_LIMIT_VARIABLE_NUMBER',128],['SQLITE_LIMIT_ATTACHED',0],['SQLITE_LIMIT_TRIGGER_DEPTH',16],['SQLITE_LIMIT_VDBE_OP',100000]]) {
            if (typeof c[constant] !== 'number') throw fault('B0916', 'Official SQLite limit API is incomplete; verify the installed SDK version')
            c.sqlite3_limit(db.pointer, c[constant], value)
          }
          db.exec('PRAGMA page_size=4096; PRAGMA cache_size=-2048; PRAGMA foreign_keys=ON; PRAGMA busy_timeout=1000;')
          const pageSize = Number(db.selectValue('PRAGMA page_size')), pages = Number(db.selectValue('PRAGMA page_count'))
          if (!Number.isSafeInteger(pageSize) || pageSize < 512 || pageSize > 65536 || pages * pageSize > 67108864) throw fault('B0914', 'Local SQL database exceeds its 64 MiB budget; archive or migrate it explicitly')
          db.exec('PRAGMA max_page_count=' + Math.floor(67108864 / pageSize))
          if (closed) throw fault('B0916', 'Local SQL handle closed while initializing')
          resolve({ db, c })
          await lease
        } catch (e) {
          reject(e?.code === 'B0916' ? e : fault('B0916', 'Persistent SQLite failed during ' + stage + '; existing data is preserved. Check storage capabilities and the official SDK assets'))
        } finally {
          try { db?.close() } catch {}
          try { pool?.pauseVfs() } catch {}
          db = null
        }
      }).catch(e => reject(e?.code === 'B0916' ? e : fault('B0916', 'Local SQL lock failed; close competing workers and retry')))
    })
    return opening
  }
  function enqueue(work) {
    if (closed) return Promise.reject(fault('B0916', 'Local SQL handle is closed'))
    if (pending >= 32) return Promise.reject(fault('B0914', 'Local SQL queue is full; await pending queries'))
    pending++
    const result = chain.then(async () => { const state = await open(); if (closed) throw fault('B0916', 'Local SQL handle is closed'); return work(state) })
    chain = result.catch(() => {}).finally(() => pending--)
    return result
  }
  function statement(state, sql, bind = [], readonly = false) {
    if (typeof sql !== 'string' || !sql.trim() || new TextEncoder().encode(sql).length > 65536 || sql.includes('\0')) throw fault('B0916', 'Local SQL needs one statement of at most 64 KiB; use bound values')
    if (!Array.isArray(bind) || bind.length > 128 || bind.some(v => !(v == null || ['string','number','bigint'].includes(typeof v) || v instanceof Uint8Array))) throw fault('B0916', 'Local SQL bindings must be at most 128 scalar or binary values')
    let bytes = 0
    for (const value of bind) {
      if (typeof value === 'number' && !Number.isFinite(value)) throw fault('B0916', 'Local SQL numeric bindings must be finite')
      bytes += value instanceof Uint8Array ? value.byteLength : typeof value === 'string' ? new TextEncoder().encode(value).length : 8
      if (bytes > 1048576) throw fault('B0914', 'Local SQL bindings exceed 1 MiB; split the operation')
    }
    const leading = sql.replace(/^(?:\s+|--[^\n]*(?:\n|$)|\/\*[\s\S]*?\*\/)+/, '')
    if (/^(?:begin|commit|rollback|savepoint|release|attach|detach|pragma|vacuum)\b/i.test(leading)) throw fault('B0916', 'Use native batch() for transactions; attachment, pragma and maintenance commands are not public query operations')
    let stmt, phase = "prepare"
    try {
      stmt = state.db.prepare(sql)
      phase = "statement boundary"
      const consumed = state.c.sqlite3_sql(stmt.pointer)
      if (typeof consumed !== 'string' || !/^[\s;]*$/.test(sql.slice(consumed.length))) throw fault('B0916', 'Use batch() for multiple SQL statements')
      phase = "read-only validation"
      if (readonly && !state.c.sqlite3_stmt_readonly(stmt.pointer)) throw fault('B0916', 'query() accepts read-only statements; use execute() or batch() for mutations')
      phase = "binding"
      if (stmt.parameterCount !== bind.length) throw fault('B0916', 'Local SQL binding count does not match the prepared statement')
      if (bind.length) stmt.bind(bind)
      phase = "execution"
      const rows = []
      let size = 0
      while (stmt.step()) {
        if (rows.length >= 200) throw fault('B0914', 'Local SQL returned more than 200 rows; add LIMIT and keyset pagination')
        const row = stmt.get(Object.create(null))
        for (const key of Object.keys(row)) {
          if (typeof row[key] === 'bigint') row[key] = String(row[key])
          const value = row[key]
          if (typeof value === 'number' && !Number.isFinite(value)) throw fault('B0916','Local SQL returned a non-finite number; cast it to a bounded application value')
          size += key.length * 2 + (value instanceof Uint8Array ? value.byteLength : typeof value === 'string' ? new TextEncoder().encode(value).length : 8)
        }
        if (size > 2097152) throw fault('B0914', 'Local SQL response exceeds 2 MiB; select fewer columns or rows')
        rows.push(row)
      }
      return { rows, changes: state.db.changes(), bytes:size }
    } catch (e) {
      throw /^B091[46]$/.test(e?.code || '') ? e : fault('B0916', 'Local SQL operation failed during ' + phase + '; inspect the query and schema in development without exposing SQL values in production')
    } finally { try { stmt?.finalize() } catch { throw fault('B0916','Local SQL statement cleanup failed; close the owned worker before retrying') } }
  }
  return {
    query(sql, bind) { return enqueue(state => statement(state, sql, bind, true).rows) },
    execute(sql, bind) { return enqueue(state => { let result; state.db.transaction(() => { result = statement(state, sql, bind) }); return result }) },
    batch(operations) {
      if (!Array.isArray(operations) || operations.length > 64) return Promise.reject(fault('B0914', 'Local SQL batches accept at most 64 operations'))
      return enqueue(state => {
        let result
        state.db.transaction(() => {
          let bytes=0, rows=0
          result=operations.map(o => {
            const value=statement(state,o.sql,o.bind); bytes+=value.bytes; rows+=value.rows.length
            if (bytes>2097152 || rows>200) throw fault('B0914','Local SQL batch response exceeds 200 rows or 2 MiB; paginate the operation')
            return value
          })
        })
        return result
      })
    },
    close() { closed = true; release(); return chain.then(() => true) },
    info() { return { engine:'official-sqlite-wasm', storage:'opfs-sahpool', namespace, name, pending, closed, maxRows:200, maxDatabaseBytes:67108864 } },
  }
}
