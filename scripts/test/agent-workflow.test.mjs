import { test } from 'node:test'
import assert from 'node:assert/strict'
import { existsSync, mkdtempSync, mkdirSync, readFileSync, writeFileSync, rmSync, unlinkSync, symlinkSync, lstatSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { spawn, spawnSync } from 'node:child_process'
import { createInterface } from 'node:readline'

const binary = process.env.BRAIN_BIN ? resolve(process.env.BRAIN_BIN) : resolve('target/debug/brain')
if (process.env.BRAIN_BIN && !existsSync(binary)) throw new Error('BRAIN_BIN does not identify an existing binary')
const requiresBinary = { skip: !existsSync(binary) && 'Build the actual brain CLI first, or set BRAIN_BIN; no mock/source-only substitute is used.', timeout: 180000 }
function app() {
  const root = mkdtempSync(join(tmpdir(), 'brain-agent-workflow-'))
  mkdirSync(join(root, 'comp')); mkdirSync(join(root, 'views'))
  writeFileSync(join(root, 'brain.json'), JSON.stringify({ name: 'AgentWorkflow' }))
  writeFileSync(join(root, 'tokens.cssbrain'), ':root { --ink: #111; }\n')
  for (const name of ['card', 'row', 'panel']) writeFileSync(join(root, 'comp', `${name}.brain`), `<p>${name}</p>\n`)
  writeFileSync(join(root, 'notes.md'), 'Original notes\n')
  return root
}
function cli(root, args, expected = 0) {
  const result = spawnSync(binary, args, { cwd: root, encoding: 'utf8', timeout: 45000, maxBuffer: 2 * 1024 * 1024, env: { ...process.env, NO_COLOR: '1' } })
  assert.ifError(result.error)
  assert.equal(result.status, expected, `CLI ${args[0]} exited unexpectedly: ${result.stderr.slice(0, 2000)} ${result.stdout.slice(0, 2000)}`)
  return JSON.parse(result.stdout)
}
function toolData(result) {
  assert.notEqual(result?.isError, true, result?.content?.find(v => v.type === 'text')?.text || 'MCP tool failed')
  if (result?.structuredContent) return result.structuredContent
  return JSON.parse(result.content.filter(v => v.type === 'text').map(v => v.text).join('\n'))
}
function failure(result, message) {
  assert.equal(result.isError, true, 'The actual MCP tool must reject this operation')
  const text = result.content.filter(v => v.type === 'text').map(v => v.text).join('\n')
  assert.match(text, message)
}
async function until(promise, milliseconds, label) {
  let timer
  try { return await Promise.race([promise, new Promise((_, reject) => { timer = setTimeout(() => reject(new Error(label)), milliseconds) })]) }
  finally { clearTimeout(timer) }
}
class Mcp {
  constructor(root) {
    this.child = spawn(binary, ['mcp', '--profile', 'full'], { cwd: root, stdio: ['pipe', 'pipe', 'pipe'], env: { ...process.env, NO_COLOR: '1' } })
    this.pending = new Map(); this.next = 0; this.bytes = 0; this.stderr = ''
    this.lines = createInterface({ input: this.child.stdout })
    this.lines.on('line', line => {
      this.bytes += Buffer.byteLength(line)
      if (this.bytes > 8 * 1024 * 1024 || Buffer.byteLength(line) > 1024 * 1024) { this.abort(new Error('MCP response exceeded the test output budget')); return }
      let message
      try { message = JSON.parse(line) } catch { this.abort(new Error('MCP emitted a non-JSON protocol line')); return }
      const waiter = this.pending.get(message.id)
      if (waiter) { this.pending.delete(message.id); message.error ? waiter.reject(new Error(message.error.message)) : waiter.resolve(message.result) }
    })
    this.child.stdin.on('error', error => this.rejectAll(error))
    this.child.stderr.on('data', bytes => { this.stderr = (this.stderr + bytes.toString()).slice(-8192) })
    this.exited = new Promise(resolveExit => {
      this.child.once('exit', (code, signal) => {
        this.rejectAll(new Error(`MCP exited (${code ?? signal})`)); resolveExit({ code, signal })
      })
      this.child.once('error', error => { this.rejectAll(error); resolveExit({ error }) })
    })
  }
  rejectAll(error) { for (const p of this.pending.values()) p.reject(error); this.pending.clear() }
  abort(error) { this.rejectAll(error); this.child.kill('SIGTERM') }
  async request(method, params) {
    if (this.pending.size >= 4) throw new Error('MCP pending request bound exceeded')
    const id = ++this.next
    const response = new Promise((resolveResponse, reject) => { this.pending.set(id, { resolve: resolveResponse, reject }) })
    this.child.stdin.write(`${JSON.stringify({ jsonrpc: '2.0', id, method, params })}\n`)
    try { return await until(response, 45000, `MCP ${method} timed out`) }
    catch (error) { this.pending.delete(id); throw error }
  }
  async initialize() {
    const result = await this.request('initialize', { protocolVersion: '2025-11-25', capabilities: {}, clientInfo: { name: 'brain-agent-workflow-test', version: '1' } })
    assert.equal(result.protocolVersion, '2025-11-25')
    this.child.stdin.write(`${JSON.stringify({ jsonrpc: '2.0', method: 'notifications/initialized' })}\n`)
  }
  call(name, args) { return this.request('tools/call', { name, arguments: args }) }
  async close() {
    this.child.stdin.end()
    let exit, forced = false
    try { exit = await until(this.exited, 3000, 'MCP did not close after EOF') }
    catch {
      forced = true; this.child.kill('SIGTERM')
      try { exit = await until(this.exited, 1500, 'MCP did not stop after SIGTERM') }
      catch { this.child.kill('SIGKILL'); exit = await until(this.exited, 1500, 'MCP did not exit after SIGKILL') }
    } finally { this.lines.close(); this.rejectAll(new Error('MCP test completed')) }
    assert.equal(forced, false, 'MCP required forced termination instead of closing on EOF')
    assert.equal(exit.code, 0, 'MCP must exit successfully after the protocol session')
  }
}

test('actual recipe CLI accepts JSON flags before/after list and recipe names without writing previews', requiresBinary, () => {
  const root = app()
  try {
    const before = cli(root, ['recipe', '--json', 'list']), after = cli(root, ['recipe', 'list', '--json'])
    assert.deepEqual(before, after)
    assert.ok(before.recipes.some(r => r.name === 'auth-ui'))
    const first = cli(root, ['recipe', '--json', 'auth-ui']), second = cli(root, ['recipe', 'auth-ui', '--json'])
    assert.deepEqual(first, second)
    assert.equal(first.previewOnly, true); assert.equal(first.dryRun, true)
    assert.ok(first.operations.length > 0)
    for (const op of first.operations) { assert.equal(op.expectedRevision, null); assert.equal(existsSync(join(root, op.path)), false) }
  } finally { rmSync(root, { recursive: true, force: true }) }
})

test('actual graph/context flags bound output and preflight exits 1 for a compiler blocker', requiresBinary, () => {
  const root = app()
  try {
    const one = cli(root, ['graph', '--cursor', '0', '--json', '--limit', '1'])
    assert.equal(one.cursor, 0); assert.equal(one.nodes.length, 1); assert.ok(one.totals.nodes > 1); assert.equal(one.nextCursor, 1)
    const two = cli(root, ['graph', '--json', '--limit', '1', '--cursor', String(one.nextCursor)])
    assert.equal(two.cursor, 1); assert.equal(two.nodes.length, 1); assert.notEqual(one.nodes[0].id, two.nodes[0].id)
    const context = cli(root, ['context', '--budget', '512', '--json', 'card'])
    assert.equal(context.budget.requested, 512); assert.equal(context.budget.maximumBytes, 512)
    assert.ok(Buffer.byteLength(JSON.stringify(context)) <= 512)
    assert.equal(context.runtimeProof, false)
    const clean = cli(root, ['preflight', '--json'])
    assert.equal(clean.firstBlockingDiagnostic, null)
    writeFileSync(join(root, 'comp', 'broken.brain'), '<p>{missingValue}</p>\n')
    const blocked = cli(root, ['preflight', '--json'], 1)
    assert.equal(blocked.firstBlockingDiagnostic.severity, 'error')
    assert.match(blocked.firstBlockingDiagnostic.code, /^B\d{4}$/)
    assert.equal(blocked.firstBlockingDiagnostic.file, 'comp/broken.brain')
    assert.ok(blocked.firstBlockingDiagnostic.line >= 1)
  } finally { rmSync(root, { recursive: true, force: true }) }
})

test('one persistent actual MCP session enforces revisions, file leases and symlink boundaries', requiresBinary, async () => {
  const root = app(), outside = mkdtempSync(join(tmpdir(), 'brain-agent-outside-'))
  const outsideFile = join(outside, 'private.md'), notes = join(root, 'notes.md')
  writeFileSync(outsideFile, 'Outside fixture must remain unchanged\n')
  const mcp = new Mcp(root)
  try {
    await mcp.initialize()
    const tools = await mcp.request('tools/list', {})
    for (const name of ['brain_read', 'brain_write', 'brain_remove', 'brain_undo', 'brain_changes', 'brain_recipe', 'brain_intelligence']) assert.ok(tools.tools.some(t => t.name === name), `actual tool missing: ${name}`)
    const read = toolData(await mcp.call('brain_read', { path: 'notes.md' }))
    assert.match(read.revision, /^[0-9a-f]{64}$/)
    assert.equal(read.source, 'Original notes\n')
    toolData(await mcp.call('brain_write', { path: 'notes.md', content: 'First change\n', expectedRevision: read.revision }))
    failure(await mcp.call('brain_write', { path: 'notes.md', content: 'Stale overwrite\n', expectedRevision: read.revision }), /revision conflict/i)
    assert.equal(readFileSync(notes, 'utf8'), 'First change\n')
    toolData(await mcp.call('brain_write', { path: 'notes.md', content: 'Optional revision still supported\n' }))
    const reserved = toolData(await mcp.call('brain_changes', { operation: 'lease', request: { owner: 'workflow-agent', paths: ['notes.md'], ttlSeconds: 30 } }))
    assert.match(reserved.token, /^[0-9a-f]{64}$/)
    failure(await mcp.call('brain_write', { path: 'notes.md', content: 'Lease bypass\n' }), /active file leases/i)
    assert.equal(readFileSync(notes, 'utf8'), 'Optional revision still supported\n')
    toolData(await mcp.call('brain_changes', { operation: 'lease', request: { action: 'release', owner: 'workflow-agent', token: reserved.token, paths: ['notes.md'] } }))
    toolData(await mcp.call('brain_write', { path: 'notes.md', content: 'After release\n' }))
    assert.equal(readFileSync(notes, 'utf8'), 'After release\n')
    failure(await mcp.call('brain_read', { path: outsideFile }), /relative|outside|path/i)
    if (process.platform !== 'win32') {
      unlinkSync(notes); symlinkSync(outsideFile, notes)
      failure(await mcp.call('brain_read', { path: 'notes.md' }), /symlink/i)
      failure(await mcp.call('brain_remove', { path: 'notes.md', confirm: true }), /symlink/i)
      failure(await mcp.call('brain_undo', { steps: 1, confirm: true }), /symlink/i)
      assert.equal(lstatSync(notes).isSymbolicLink(), true)
      assert.equal(readFileSync(outsideFile, 'utf8'), 'Outside fixture must remain unchanged\n')
    }
  } finally {
    try { await mcp.close() }
    finally { rmSync(root, { recursive: true, force: true }); rmSync(outside, { recursive: true, force: true }) }
  }
})
