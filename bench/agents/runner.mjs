// Provider-neutral benchmark plumbing. No provider, model or paid invocation is selected here.
import { spawn } from 'node:child_process'
import { createHash, randomUUID } from 'node:crypto'
import { createReadStream } from 'node:fs'
import { mkdir, readFile, writeFile, readdir, lstat, mkdtemp, rm, realpath } from 'node:fs/promises'
import { resolve, join, relative, isAbsolute } from 'node:path'
import { tmpdir } from 'node:os'

const MAX_FILE = 4 * 1024 * 1024
const MAX_TOTAL = 128 * 1024 * 1024
export function command(value, label = 'command') {
  if (!Array.isArray(value) || !value.length || value.some(v => typeof v !== 'string' || !v || v.includes('\0'))) throw Error(`${label} must be a nonempty JSON array of strings`)
  return value
}
export function config(value) {
  if (!value || typeof value !== 'object') throw Error('configuration is required')
  command(value.command)
  for (const key of ['model', 'effort', 'binary', 'out']) if (typeof value[key] !== 'string' || !value[key].trim()) throw Error(`explicit ${key} is required`)
  const timeoutMs = value.timeoutMs ?? 120000, maxOutputBytes = value.maxOutputBytes ?? 1024 * 1024
  if (!Number.isInteger(timeoutMs) || timeoutMs < 50 || timeoutMs > 600000) throw Error('timeoutMs must be 50..600000')
  if (!Number.isInteger(maxOutputBytes) || maxOutputBytes < 1024 || maxOutputBytes > 4 * 1024 * 1024) throw Error('maxOutputBytes must be 1024..4194304')
  if (!Array.isArray(value.validators) || !value.validators.length || value.validators.length > 8) throw Error('explicit validators are required')
  value.validators.forEach((v, i) => command(v, `validators[${i}]`))
  if (value.contextPaths != null && (!Array.isArray(value.contextPaths) || value.contextPaths.some(p => typeof p !== 'string'))) throw Error('contextPaths must be an array of paths')
  return { ...value, timeoutMs, maxOutputBytes }
}
const sha = data => createHash('sha256').update(data).digest('hex')
export async function hashFile(path) {
  const stat = await lstat(path)
  if (!stat.isFile() || stat.size > MAX_TOTAL) throw Error(`Invalid or oversized hash input: ${path}`)
  const hash = createHash('sha256')
  for await (const chunk of createReadStream(path)) hash.update(chunk)
  return hash.digest('hex')
}
export async function sourceHash(root) {
  const hash = createHash('sha256')
  let total = 0, count = 0
  async function walk(dir) {
    const entries = await readdir(dir, { withFileTypes: true })
    entries.sort((a, b) => a.name.localeCompare(b.name, 'en'))
    for (const entry of entries) {
      if (entry.name.startsWith('.') || ['node_modules', 'target', 'results', 'dist', 'cache', 'caches', 'coverage', 'secrets', 'credentials', 'private'].includes(entry.name) || /(?:\.pem|\.key|\.p12|\.pfx|\.sqlite|\.db|\.log)$/.test(entry.name)) continue
      const path = join(dir, entry.name)
      if (entry.isSymbolicLink()) throw Error(`Symlink source is unsupported: ${path}`)
      if (entry.isDirectory()) await walk(path)
      else if (entry.isFile()) {
        const stat = await lstat(path)
        total += stat.size
        if (++count > 20000 || total > MAX_TOTAL) throw Error('Source snapshot exceeds limits')
        hash.update(relative(root, path)).update('\0').update(await hashFile(path)).update('\0')
      }
    }
  }
  for (const name of ['crates', 'runtime', 'SPEC.md', 'Cargo.lock', 'Cargo.toml', 'bench/agents']) {
    const path = join(root, name)
    if ((await lstat(path)).isDirectory()) await walk(path)
    else hash.update(name).update('\0').update(await hashFile(path)).update('\0')
  }
  return { sha256: hash.digest('hex'), files: count, bytes: total }
}

// Each invocation owns a process group on POSIX. No shell and no retries.
export function execute(argv, cwd, input, limits) {
  command(argv)
  return new Promise((resolveResult, reject) => {
    const child = spawn(argv[0], argv.slice(1), { cwd, shell: false, detached: process.platform !== 'win32', stdio: ['pipe', 'pipe', 'pipe'] })
    let stdout = '', stderr = '', bytes = 0, reason = null, escalation, finished = false
    const started = performance.now()
    const kill = signal => {
      try { if (process.platform === 'win32') child.kill(signal); else process.kill(-child.pid, signal) } catch (error) { if (error.code !== 'ESRCH') reason ||= 'termination-failed' }
    }
    const stop = why => {
      if (finished || reason) return
      reason = why
      kill('SIGTERM')
      escalation = setTimeout(() => kill('SIGKILL'), 250)
    }
    const timer = setTimeout(() => stop('timeout'), limits.timeoutMs)
    const capture = stream => chunk => {
      bytes += chunk.length
      if (bytes > limits.maxOutputBytes) return stop('output-limit')
      if (stream === 'stdout') stdout += chunk.toString('utf8')
      else stderr += chunk.toString('utf8')
    }
    child.stdout.on('data', capture('stdout'))
    child.stderr.on('data', capture('stderr'))
    const interrupt = () => stop('interrupted')
    process.once('SIGINT', interrupt)
    process.once('SIGTERM', interrupt)
    child.stdin.on('error', () => {}) // adapters may intentionally stop reading
    child.stdin.end(input)
    child.once('error', error => { reason ||= 'spawn-failed'; stderr = error.message })
    child.once('close', (code, signal) => {
      // Kill remaining descendants while this process group is still owned.
      kill('SIGKILL')
      finished = true
      clearTimeout(timer)
      clearTimeout(escalation)
      process.removeListener('SIGINT', interrupt)
      process.removeListener('SIGTERM', interrupt)
      resolveResult({ code, signal, reason, stdout, stderr, durationMs: Math.round(performance.now() - started), outputBytes: bytes })
    })
  })
}
export function metrics(report) {
  const result = { firstPass: null, turns: null, inputTokens: null, outputTokens: null, interventions: null, provenance: 'not-reported' }
  if (report == null) return result
  if (typeof report !== 'object' || Array.isArray(report)) throw Error('metrics must be an object')
  for (const key of ['turns', 'inputTokens', 'outputTokens', 'interventions']) {
    if (report[key] == null) continue
    if (!Number.isSafeInteger(report[key]) || report[key] < 0) throw Error(`Invalid reported metric: ${key}`)
    result[key] = report[key]
  }
  if (report.firstPass != null) {
    if (typeof report.firstPass !== 'boolean') throw Error('firstPass must be boolean')
    result.firstPass = report.firstPass
  }
  if (Object.values(result).some(v => typeof v === 'number' || typeof v === 'boolean')) result.provenance = 'adapter-reported'
  return result
}
export function fixture(value) {
  const kinds = ['create', 'edit', 'repair', 'atomic-conflict', 'CRUD']
  if (!value || !kinds.includes(value.kind) || typeof value.prompt !== 'string' || !value.prompt || value.prompt.length > 65536) throw Error('Invalid fixture kind/prompt')
  if (!value.files || typeof value.files !== 'object' || Array.isArray(value.files)) throw Error('Fixture files are required')
  for (const [path, content] of Object.entries(value.files)) {
    if (!path || isAbsolute(path) || path.split(/[\\/]/).some(p => !p || p === '.' || p === '..') || typeof content !== 'string' || Buffer.byteLength(content) > MAX_FILE) throw Error(`Unsafe fixture file: ${path}`)
  }
  if (Object.keys(value.files).length > 64) throw Error('Too many fixture files')
  if (Object.values(value.files).reduce((bytes, content) => bytes + Buffer.byteLength(content), 0) > 16 * 1024 * 1024) throw Error('Fixture total exceeds 16 MiB')
  return value
}
const substitute = (args, values) => args.map(arg => arg.replace(/\{(binary|workspace|model|effort)\}/g, (_, key) => values[key]))

export async function run(options, fixtures, root) {
  if (!Array.isArray(fixtures) || !fixtures.length || fixtures.length > 20) throw Error('fixtures must contain 1..20 tasks')
  const c = config(options), binary = await realpath(resolve(c.binary))
  root = await realpath(root)
  const before = await sourceHash(root), binaryHash = await hashFile(binary)
  const spec = await readFile(join(root, 'SPEC.md'), 'utf8')
  if (Buffer.byteLength(spec) > MAX_FILE) throw Error('SPEC exceeds context limit')
  const context = []
  for (const path of c.contextPaths || []) {
    const absolute = await realpath(resolve(root, path)), stat = await lstat(absolute)
    if (!stat.isFile() || stat.size > MAX_FILE || context.length >= 32) throw Error('Context limits exceeded')
    const content = await readFile(absolute, 'utf8')
    context.push({ path, sha256: sha(content), content })
    if (context.reduce((bytes, item) => bytes + Buffer.byteLength(item.content), 0) > 16 * 1024 * 1024) throw Error('Context total exceeds 16 MiB')
  }
  const runId = randomUUID(), results = [], startedAt = new Date().toISOString(), deadline = Date.now() + 1800000
  for (const raw of fixtures) {
    if (Date.now() >= deadline) throw Error('Run deadline exceeded (30 minutes)')
    const task = fixture(raw), taskDeadline = Math.min(deadline, Date.now() + c.timeoutMs), workspace = await mkdtemp(join(tmpdir(), 'brain-agent-bench-'))
    try {
      for (const [path, content] of Object.entries(task.files)) {
        const dest = join(workspace, path)
        await mkdir(resolve(dest, '..'), { recursive: true })
        await writeFile(dest, content, { flag: 'wx' })
      }
      const input = JSON.stringify({ schemaVersion: 1, task, workspace, model: c.model, effort: c.effort, binary, context: [{ path: 'SPEC.md', sha256: sha(spec), content: spec }, ...context] }) + '\n'
      const values = { binary, workspace, model: c.model, effort: c.effort }
      const execution = await execute(substitute(c.command, values), workspace, input, { ...c, timeoutMs: Math.max(1, taskDeadline - Date.now()) })
      let report = null, protocolError = null
      if (execution.code === 0 && !execution.reason) {
        try { report = JSON.parse(execution.stdout); if (!report || typeof report !== 'object' || Array.isArray(report)) throw Error('Response must be an object'); metrics(report.metrics) }
        catch (error) { protocolError = error.message }
      }
      const validation = []
      if (!execution.reason && execution.code === 0 && !protocolError) {
        for (const args of c.validators) {
          const result = await execute(substitute(args, values), workspace, JSON.stringify({ task, report }) + '\n', { ...c, timeoutMs: Math.max(1, taskDeadline - Date.now()) })
          validation.push({ ...result, passed: result.code === 0 && !result.reason })
        }
      }
      results.push({ kind: task.kind, fixtureSha256: sha(JSON.stringify(task)), inputSha256: sha(input), execution, protocolError, evidenceMode: report?.replay === true ? 'replay' : 'adapter', metrics: protocolError ? metrics(null) : metrics(report?.metrics), validation, passed: validation.length === c.validators.length && validation.every(v => v.passed) })
      if (execution.reason === 'interrupted' || validation.some(v => v.reason === 'interrupted')) break
    } finally { await rm(workspace, { recursive: true, force: true }) }
  }
  const after = await sourceHash(root), binaryHashEnd = await hashFile(binary)
  const result = { schemaVersion: 1, runId, startedAt, finishedAt: new Date().toISOString(), model: c.model, effort: c.effort, identityProvenance: 'explicit-configuration', adapterCommand: c.command, validators: c.validators, source: before, sourceEnd: after, specSha256: sha(spec), binarySha256: binaryHash, binarySha256End: binaryHashEnd, context: context.map(({ path, sha256 }) => ({ path, sha256 })), stable: before.sha256 === after.sha256 && binaryHash === binaryHashEnd, results, freshLLMGate: 'pending; this runner does not replace bench/llm/gate.mjs' }
  const out = resolve(c.out)
  await mkdir(resolve(out, '..'), { recursive: true })
  await writeFile(out, JSON.stringify(result, null, 2) + '\n', { flag: 'wx', mode: 0o600 })
  return result
}
