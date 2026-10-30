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
  ctx.env = Object.freeze(Object.assign({}, process.env))
  const module = await import(pathToFileURL(entry).href)
  if (typeof module.default !== 'function') throw Error()
  await module.default(ctx)
} catch (error) {
  process.exitCode = 1
  try { emit(`JOB_TRACE ${JSON.stringify(trace(error, jobId))}\n`) } catch {}
}
