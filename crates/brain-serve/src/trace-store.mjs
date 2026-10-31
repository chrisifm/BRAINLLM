// `brain trace` store (INTERNALS §48). Development only: `brain run` starts it as a child and `brain trace` reads through
// it; nothing a production build ships imports this file. One SQLite file, `.brain/trace.db`, with two generic tables:
// a new traceability module adds a module name and kinds, never a column.
//
//   node -e <this> writer <config>         NDJSON events on stdin; every line is redacted here, before it reaches disk
//   node -e <this> query <config> <args>   one JSON or text answer on stdout
//
// Other processes (the backend) append NDJSON lines to `.brain/trace-spool/<module>.ndjson`; both modes drain them.
import { DatabaseSync } from 'node:sqlite'
import { chmodSync, existsSync, mkdirSync, readFileSync, readdirSync, renameSync, rmSync } from 'node:fs'
import { createHash } from 'node:crypto'
import { createInterface } from 'node:readline'
import { join, relative } from 'node:path'

const [mode, rawConfig, rawArgs] = process.argv.slice(1)
const config = JSON.parse(rawConfig || '{}')
const MAX_BODY = Math.max(256, Number(config.maxBodyKB ?? 16) * 1024)
const MAX_BYTES = Math.max(1, Number(config.maxMB ?? 64)) * 1048576
const MAX_AGE = Math.max(1, Number(config.maxAgeHours ?? 72)) * 3600000
const MAX_DATA = MAX_BODY * 4
const ROOT = process.cwd()
const DB_PATH = join(ROOT, '.brain', 'trace.db')
const SPOOL = join(ROOT, '.brain', 'trace-spool')

// ---------- redaction: the only way into the store ----------
const SENSITIVE = /pass(?:word|wd|phrase)|pwd|secret|token|cookie|authorization|api[-_]?key|credential|private[-_]?key|session[-_]?id|bearer|jwt|otp|cvv/i
const CONFIGURED = new Set((Array.isArray(config.redact) ? config.redact : []).map(name => String(name).toLowerCase()))
const hidden = name => SENSITIVE.test(name) || CONFIGURED.has(String(name).toLowerCase())
const MASK = '[redacted]'
// bounded and anchored: a long run of word characters must not cost quadratic time
const PAIRS = /(?<![\w.-])([\w.-]{1,128})(=|%3D)([^&\s"';,]{1,4096})/g
const QUOTED = /"([\w.-]+)"\s*:\s*"(?:[^"\\]|\\.)*"/g
const PATTERNS = [
  [/\b(Bearer|Basic)\s+[\w.~+/=-]{4,}/gi, '$1 ' + MASK],
  [/\beyJ[\w-]{4,}\.[\w-]{4,}\.[\w-]*/g, MASK],
  [/\b(?:sk|pk|rk)[-_](?:live|test|ant|proj)?[-_]?[\w-]{12,}/g, MASK],
  [/\b(?:gh[pousr]_[A-Za-z0-9]{20,}|xox[abprs]-[\w-]{10,}|AKIA[A-Z0-9]{16})\b/g, MASK],
]
export function redactText(value) {
  let text = String(value)
  for (const [pattern, replacement] of PATTERNS) text = text.replace(pattern, replacement)
  text = text.replace(PAIRS, (all, key, sign) => (hidden(key) ? key + sign + MASK : all))
  return text.replace(QUOTED, (all, key) => (hidden(key) ? `"${key}":"${MASK}"` : all))
}
const clip = text => (Buffer.byteLength(text) > MAX_BODY ? `${Buffer.from(text).subarray(0, MAX_BODY).toString().replace(/�$/, '')}…[truncated, ${Buffer.byteLength(text)} bytes]` : text)
export function redact(value, depth = 0) {
  if (typeof value === 'string') return clip(redactBody(value))
  if (value === null || typeof value !== 'object') return value
  if (depth > 8) return '[depth limit]'
  if (Array.isArray(value)) return value.slice(0, 64).map(item => redact(item, depth + 1))
  const out = {}
  for (const [key, item] of Object.entries(value).slice(0, 128)) out[key] = hidden(key) ? MASK : redact(item, depth + 1)
  return out
}
// A body that is JSON is walked by key, so a secret keeps its shape; anything else goes through the text patterns.
function redactBody(text) {
  const start = text.trimStart()[0]
  if ((start === '{' || start === '[') && text.length <= 4 * 1048576) {
    try { return JSON.stringify(redact(JSON.parse(text), 1)) } catch {}
  }
  return redactText(text.slice(0, MAX_BODY))
}
// Server-function arguments are positional, so a password has no key to match: top-level strings keep their length only.
const positional = text => {
  try {
    const args = JSON.parse(text)
    if (!Array.isArray(args)) return text
    return JSON.stringify(args.map(item => (typeof item === 'string' ? `[string, ${item.length} chars]` : item)))
  } catch { return text }
}

// ---------- normalization: every source becomes the same row ----------
const NAME = /^[a-z][a-z0-9_.-]{0,39}$/
const short = (value, max = 512) => (value == null ? null : String(value).slice(0, max))
const number = value => (Number.isFinite(value) ? value : null)
const relativeFile = file => {
  if (!file) return null
  const clean = String(file).replace(/^file:\/\//, '').replace(/[?#].*$/, '')
  return clean.startsWith('/') && clean.startsWith(ROOT) ? relative(ROOT, clean) : clean.replace(/^\//, '')
}
// The first frame of an original source: Node maps stacks with --enable-source-maps, the runtime's own copies are skipped.
function frameOf(stack) {
  for (const line of String(stack || '').split('\n')) {
    const found = /((?:file:\/\/)?\/[^\s()]+?\.(?:jsbrain|brain|mjs|js)(?:\?[^\s():]*)?):(\d+):(\d+)/.exec(line)
    if (!found) continue
    const file = relativeFile(found[1])
    if (file.includes('node_modules') || file.includes('.brain/server/_rt') || file.startsWith('node:')) continue
    if (/\.(?:jsbrain|brain)$/.test(file)) return { file: file.replace('.brain/server/', ''), line: Number(found[2]), generated: false }
    if (file.startsWith('.brain/server/')) return { file: file.replace('.brain/server/', '').replace(/\.mjs$/, ''), line: Number(found[2]), generated: true }
  }
  return null
}
function fromError(raw) {
  const location = raw.location || {}, src = /^(.+?):(\d+)/.exec(String(raw.src || ''))
  const file = src ? src[1] : location.file, line = src ? Number(src[2]) : location.line
  return { ts: raw.at, session: raw.session, tab: raw.tab, request_id: raw.rid, module: 'browser', kind: 'error', level: 'error', route: raw.url, component: raw.comp, file, line, title: raw.message, code: raw.code, fix: raw.fix, fingerprint_hint: raw.fp,
    data: { type: raw.type, where: raw.where, stack: raw.stack, frames: raw.frames, causes: raw.causes, mapping: raw.mapping, componentChain: raw.componentChain, crumbs: raw.crumbs, count: raw.count, generatedLocation: raw.generatedLocation } }
}
function incoming(raw) {
  if (!raw || typeof raw !== 'object') return []
  if (raw.t === 'brain:trace') return (Array.isArray(raw.events) ? raw.events : []).map(event => ({ session: raw.session, tab: raw.tab, ...event }))
  if (raw.t === 'brain:error') return [fromError(raw)]
  return [raw]
}
// Kinds with a known diagnosis. A module with its own diagnosis sends `code` and `fix` on the event instead.
const failedRequest = event => {
  const { status, kind } = event
  if (!['rpc', 'fetch', 'page', 'prefetch', 'xhr', 'request', 'native', 'form'].includes(kind) || status == null) return false
  return status >= 400 || status === 0
}
function diagnose(event, rid) {
  const see = rid ? `\`brain trace ${rid}\`` : '`brain trace --failed`'
  if (event.kind === 'exception') return ['B1841', `Open the mapped location; ${see} shows the request, its payload and the stack`]
  if (event.kind === 'csp') return ['B1842', 'Production blocks this too: serve it from this origin, or move inline code into a module. See `brain explain B1842`']
  if (event.kind === 'cors') return ['B1843', 'Call a cross-origin service from a server function, or make that service answer with this origin in Access-Control-Allow-Origin']
  if (event.kind === 'socket') return ['B1844', `The socket closed abnormally; check that its endpoint is running and the browser origin is allowed. ${see}`]
  if (failedRequest(event)) return ['B1840', `${see} shows the browser payload and the server events of this request`]
  return [null, null]
}
const stable = text => String(text ?? '').replace(/[0-9a-f]{8}-[0-9a-f-]{27,}/gi, '#').replace(/\b[0-9a-f]{12,}\b/gi, '#').replace(/\d+/g, '#').slice(0, 200)
const fingerprint = parts => createHash('sha256').update(parts.join('\u0001')).digest('hex').slice(0, 16)
function normalize(raw) {
  const data = raw.data && typeof raw.data === 'object' ? { ...raw.data } : {}
  const module = NAME.test(raw.module) ? raw.module : 'unknown'
  const kind = NAME.test(raw.kind) ? raw.kind : 'event'
  if (typeof data.payload === 'string') data.payload = kind === 'rpc' ? positional(data.payload) : data.payload
  const event = {
    ts: Number.isSafeInteger(raw.ts) ? raw.ts : Date.now(), session: short(raw.session, 64), tab: short(raw.tab, 64),
    request_id: short(raw.request_id, 64), module, kind,
    level: ['error', 'warn', 'info', 'debug'].includes(raw.level) ? raw.level : 'info',
    route: raw.route == null ? null : redactText(short(raw.route)), component: short(raw.component, 160),
    file: relativeFile(short(raw.file, 1024)), line: number(raw.line), duration_ms: number(raw.duration_ms), status: number(raw.status),
    title: raw.title == null ? null : redactText(short(raw.title, 1024)), code: short(raw.code, 16), fix: raw.fix == null ? null : redactText(short(raw.fix, 2048)),
  }
  if (typeof data.stack === 'string') {
    const frame = frameOf(data.stack)
    if (frame && !event.file) { event.file = frame.file; event.line = frame.line; if (frame.generated) data.locationPrecision = 'generated' }
  }
  if (failedRequest(event) && event.level === 'info') event.level = event.status >= 500 || event.status === 0 ? 'error' : 'warn'
  let body = JSON.stringify(redact({ title: event.title, ...data }))
  if (Buffer.byteLength(body) > MAX_DATA) body = JSON.stringify({ title: event.title, truncated: true, preview: Buffer.from(body).subarray(0, MAX_BODY).toString() })
  event.data = body
  if (event.level === 'error' || event.level === 'warn') {
    const [code, fix] = diagnose(event, event.request_id)
    event.issue = {
      code: (/^B\d{4}$/.test(event.code || '') ? event.code : null) || code || /\bB\d{4}\b/.exec(event.title || '')?.[0] || null,
      fix: event.fix || fix,
      title: failedRequest(event) ? `${data.method || 'GET'} ${String(event.route).split('?')[0]} answered ${event.status}` : event.title || `${module} ${kind}`,
    }
    event.fingerprint = failedRequest(event)
      ? fingerprint(['request', data.method || 'GET', stable(event.route), event.status])
      // a coded message is one problem, whichever channel reported it (brain-err and the console both print it)
      : event.issue.code ? fingerprint([event.issue.code, stable(event.title)])
      : fingerprint([module, kind, stable(event.title), event.file, event.line])
  } else event.fingerprint = null
  // brain-err also reports a failed server function: the failed request is that issue, with its id and payload
  if (kind === 'error' && data.type === 'rpc') event.fingerprint = null
  return event
}

// ---------- store ----------
function open() {
  mkdirSync(join(ROOT, '.brain'), { recursive: true })
  const db = new DatabaseSync(DB_PATH)
  db.exec(`PRAGMA busy_timeout=3000; PRAGMA auto_vacuum=INCREMENTAL; PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;
    CREATE TABLE IF NOT EXISTS events(id INTEGER PRIMARY KEY, ts INTEGER NOT NULL, session TEXT, tab TEXT, request_id TEXT, module TEXT NOT NULL, kind TEXT NOT NULL, level TEXT NOT NULL, fingerprint TEXT, route TEXT, component TEXT, file TEXT, line INTEGER, duration_ms REAL, status INTEGER, data TEXT NOT NULL);
    CREATE INDEX IF NOT EXISTS events_ts ON events(ts);
    CREATE INDEX IF NOT EXISTS events_request ON events(request_id);
    CREATE INDEX IF NOT EXISTS events_module ON events(module, ts);
    CREATE TABLE IF NOT EXISTS issues(fingerprint TEXT PRIMARY KEY, title TEXT NOT NULL, module TEXT NOT NULL, kind TEXT NOT NULL, level TEXT NOT NULL, count INTEGER NOT NULL, first_seen INTEGER NOT NULL, last_seen INTEGER NOT NULL, b_code TEXT, fix TEXT, route TEXT, component TEXT, file TEXT, line INTEGER, last_request_id TEXT);`)
  try { chmodSync(DB_PATH, 0o600) } catch {}
  return db
}
function writerFor(db) {
  const event = db.prepare('INSERT INTO events(ts,session,tab,request_id,module,kind,level,fingerprint,route,component,file,line,duration_ms,status,data) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)')
  // The browser and the server both report one failed request: it counts once, by request id.
  const issue = db.prepare(`INSERT INTO issues(fingerprint,title,module,kind,level,count,first_seen,last_seen,b_code,fix,route,component,file,line,last_request_id) VALUES(?,?,?,?,?,1,?,?,?,?,?,?,?,?,?)
    ON CONFLICT(fingerprint) DO UPDATE SET count=count+(CASE WHEN excluded.last_request_id IS NOT NULL AND excluded.last_request_id=issues.last_request_id THEN 0 ELSE 1 END), last_seen=excluded.last_seen, level=CASE WHEN excluded.level='error' THEN 'error' ELSE issues.level END, last_request_id=excluded.last_request_id, fix=COALESCE(excluded.fix,issues.fix), b_code=COALESCE(issues.b_code,excluded.b_code), file=COALESCE(excluded.file,issues.file), line=COALESCE(excluded.line,issues.line)`)
  return lines => {
    if (!lines.length) return
    db.exec('BEGIN IMMEDIATE')
    try {
      for (const line of lines) {
        let raw
        try { raw = JSON.parse(line) } catch { continue }
        for (const item of incoming(raw)) {
          const e = normalize(item)
          event.run(e.ts, e.session, e.tab, e.request_id, e.module, e.kind, e.level, e.fingerprint, e.route, e.component, e.file, e.line, e.duration_ms, e.status, e.data)
          if (e.fingerprint) issue.run(e.fingerprint, e.issue.title, e.module, e.kind, e.level, e.ts, e.ts, e.issue.code, e.issue.fix, e.route, e.component, e.file, e.line, e.request_id)
        }
      }
      db.exec('COMMIT')
    } catch (error) { db.exec('ROLLBACK'); throw error }
  }
}
// Age first, then size: the oldest events go until the file fits. Issues outlive their events until they age out too.
function prune(db) {
  const cut = Date.now() - MAX_AGE
  db.prepare('DELETE FROM events WHERE ts<?').run(cut)
  db.prepare('DELETE FROM issues WHERE last_seen<?').run(cut)
  const size = () => { const pages = db.prepare('PRAGMA page_count').get().page_count, free = db.prepare('PRAGMA freelist_count').get().freelist_count; return (pages - free) * db.prepare('PRAGMA page_size').get().page_size }
  for (let round = 0; round < 64 && size() > MAX_BYTES; round++) {
    const total = db.prepare('SELECT count(*) AS n FROM events').get().n
    if (!total) break
    db.prepare('DELETE FROM events WHERE id IN (SELECT id FROM events ORDER BY id LIMIT ?)').run(Math.max(64, Math.ceil(total / 8)))
  }
  db.exec('PRAGMA incremental_vacuum; PRAGMA wal_checkpoint(PASSIVE)')
}
function drainSpool(write) {
  if (!existsSync(SPOOL)) return
  for (const name of readdirSync(SPOOL)) {
    if (!name.endsWith('.ndjson')) continue
    const claimed = join(SPOOL, `${name}.${process.pid}.claimed`)
    try { renameSync(join(SPOOL, name), claimed) } catch { continue }
    try { write(readFileSync(claimed, 'utf8').split('\n').filter(Boolean)) } finally { rmSync(claimed, { force: true }) }
  }
}

// ---------- writer ----------
async function writer() {
  const db = open(), write = writerFor(db)
  let batch = []
  const flush = () => { const lines = batch; batch = []; write(lines) }
  const guarded = work => { try { work() } catch (error) { console.error(`brain trace: ${error.message}`) } }
  const timers = [
    setInterval(() => guarded(flush), 200),
    setInterval(() => guarded(() => drainSpool(write)), 1000),
    setInterval(() => guarded(() => prune(db)), 30000),
  ]
  guarded(() => prune(db))
  guarded(() => drainSpool(write))
  for await (const line of createInterface({ input: process.stdin })) if (line) { batch.push(line); if (batch.length >= 512) guarded(flush) }
  for (const timer of timers) clearInterval(timer)
  guarded(flush)
  guarded(() => drainSpool(write))
  db.close()
}

// ---------- reader ----------
const SPAN = { s: 1000, m: 60000, h: 3600000, d: 86400000 }
function since(text) {
  const found = /^(\d+)(s|m|h|d)$/.exec(String(text))
  if (!found) throw new Error(`invalid --since \`${text}\`; fix: a number and a unit, e.g. 30s, 5m, 2h, 1d`)
  return Date.now() - Number(found[1]) * SPAN[found[2]]
}
const row = r => { const { data, ...rest } = r; let parsed = {}; try { parsed = JSON.parse(data) } catch {} ; return { ...rest, data: parsed } }
const clock = ts => new Date(ts).toISOString().slice(11, 23)
const where = r => (r.file ? `${r.file}${r.line ? `:${r.line}` : ''}` : '')
function line(r) {
  const parts = [clock(r.ts), r.level.padEnd(5), `${r.module}/${r.kind}`]
  if (r.status != null) parts.push(String(r.status))
  if (r.duration_ms != null) parts.push(`${Math.round(r.duration_ms)}ms`)
  if (r.data.method) parts.push(r.data.method)
  if (r.route) parts.push(r.route)
  if (where(r)) parts.push(where(r))
  if (r.data.title) parts.push(`— ${r.data.title}`)
  if (r.request_id) parts.push(`[${r.request_id}]`)
  return parts.join(' ')
}
function details(r) {
  const out = [line(r)]
  for (const key of ['payload', 'response', 'statement', 'violation', 'stack']) {
    const value = r.data[key]
    if (value != null) out.push(`  ${key}: ${typeof value === 'string' ? value.split('\n').slice(0, 12).join('\n    ') : JSON.stringify(value)}`)
  }
  if (Array.isArray(r.data.crumbs) && r.data.crumbs.length) out.push(`  crumbs: ${r.data.crumbs.map(c => `${c.k} ${c.d}`).join(' · ')}`)
  return out.join('\n')
}
function query(args) {
  if (!existsSync(DB_PATH) && !existsSync(SPOOL)) {
    if (args.json) process.stdout.write(JSON.stringify({ schemaVersion: 1, [args.mode === 'issues' ? 'issues' : 'events']: [], more: false, empty: true }) + '\n')
    else process.stdout.write('No trace yet. Start the app with `brain run`, open it in a browser, and ask again.\n')
    return
  }
  const db = open()
  drainSpool(writerFor(db))
  prune(db)
  const limit = Math.min(Math.max(1, Number(args.limit) || 100), 1000)
  if (args.mode === 'issues') {
    const clauses = [], values = []
    if (args.since) { clauses.push('last_seen>=?'); values.push(since(args.since)) }
    if (args.module) { clauses.push('module=?'); values.push(String(args.module)) }
    const issues = db.prepare(`SELECT * FROM issues${clauses.length ? ' WHERE ' + clauses.join(' AND ') : ''} ORDER BY last_seen DESC LIMIT ?`).all(...values, limit + 1)
    const more = issues.length > limit
    issues.length = Math.min(issues.length, limit)
    if (args.json) process.stdout.write(JSON.stringify({ schemaVersion: 1, issues, more }) + '\n')
    else if (!issues.length) process.stdout.write('No issues.\n')
    else process.stdout.write(issues.map(i => `${i.b_code || '-----'} ${i.level.padEnd(5)} ×${i.count} ${i.title}${i.file ? ` (${i.file}${i.line ? `:${i.line}` : ''})` : ''}\n  last ${new Date(i.last_seen).toISOString()}${i.last_request_id ? ` [${i.last_request_id}]` : ''}${i.fix ? `\n  fix: ${i.fix}` : ''}`).join('\n') + '\n')
    return
  }
  const clauses = [], values = []
  if (args.requestId) { clauses.push('(request_id=? OR request_id LIKE ?)'); values.push(String(args.requestId), String(args.requestId).replace(/[%_]/g, '') + '%') }
  if (args.since) { clauses.push('ts>=?'); values.push(since(args.since)) }
  if (args.module) { clauses.push('module=?'); values.push(String(args.module)) }
  if (args.failed) clauses.push("(level='error' OR status>=400 OR status=0)")
  // The newest events, shown oldest first: a request reads top to bottom.
  const rows = db.prepare(`SELECT * FROM events${clauses.length ? ' WHERE ' + clauses.join(' AND ') : ''} ORDER BY ts DESC, id DESC LIMIT ?`).all(...values, limit + 1)
  const more = rows.length > limit
  rows.length = Math.min(rows.length, limit)
  const events = rows.reverse().map(row)
  if (args.json) process.stdout.write(JSON.stringify({ schemaVersion: 1, events, more }) + '\n')
  else if (!events.length) process.stdout.write(args.requestId ? `No events for request ${args.requestId}.\n` : 'No events match.\n')
  else process.stdout.write(events.map(args.requestId || args.failed ? details : line).join('\n') + (more ? `\n… more; narrow with --since or --limit\n` : '\n'))
}

try {
  if (mode === 'writer') await writer()
  else if (mode === 'query') query(JSON.parse(rawArgs || '{}'))
  else throw new Error(`unknown store mode ${mode}`)
} catch (error) {
  console.error(`error: ${error.message}`)
  process.exitCode = 2
}
