import test from 'node:test'
import assert from 'node:assert/strict'
import { existsSync, mkdtempSync, readdirSync, readFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join, relative, resolve } from 'node:path'
import { createHash } from 'node:crypto'
import { spawnSync } from 'node:child_process'

const binary = process.env.BRAIN_BIN ? resolve(process.env.BRAIN_BIN) : resolve('target/debug/brain')
if (process.env.BRAIN_BIN && !existsSync(binary)) throw Error('BRAIN_BIN must identify an existing compiler')
function run(args, cwd, deadline) {
  const remaining = deadline - Date.now()
  assert.ok(remaining > 0, 'compiler proof deadline')
  const result = spawnSync(binary, args, { cwd, encoding: 'utf8', detached: process.platform !== 'win32', timeout: Math.min(120000, remaining), killSignal: 'SIGKILL', maxBuffer: 4 * 1024 * 1024 })
  if ((result.error || result.signal) && result.pid && process.platform !== 'win32') {
    try { process.kill(-result.pid, 'SIGKILL') } catch {}
  }
  // Output can include private fixture paths: retain only status and structured budget diagnostics.
  let budget = ''
  try {
    const match = /^budget\.viewKB exceeded on ([A-Za-z0-9/_:*.-]{1,160}): (\d+) bytes/.exec(JSON.parse(result.stdout).error || '')
    if (match) budget = `, route=${match[1]}, bytes=${match[2]}`
  } catch {}
  assert.equal(result.status, 0, `compiler ${args[0]} failed (status ${result.status}, signal ${result.signal || 'none'}${budget})`)
  return args.includes('--json') ? JSON.parse(result.stdout) : null
}
function inventory(root, at = root, result = {}) {
  for (const entry of readdirSync(at, { withFileTypes: true })) {
    const file = join(at, entry.name)
    assert.ok(!entry.isSymbolicLink(), 'owned build output must not contain symlinks')
    if (entry.isDirectory()) inventory(root, file, result)
    else if (entry.isFile()) result[relative(root, file)] = createHash('sha256').update(readFileSync(file)).digest('hex')
  }
  return result
}

test('default30 builds in separate cold-cache processes emit identical files and shed unused scalar parsers', {
  skip: !existsSync(binary) && 'Build the actual compiler first or supply BRAIN_BIN; no mock is used', timeout: 360000
}, () => {
  const scratch = mkdtempSync(join(tmpdir(), 'brain-build-determinism-')), app = join(scratch, 'determinism-proof')
  const deadline = Date.now() + 300000
  try {
    run(['new', 'determinism-proof'], scratch, deadline)
    assert.equal(JSON.parse(readFileSync(join(app, 'brain.json'), 'utf8')).budget.viewKB, 30)
    const outputs = [], signatures = []
    for (let pass = 0; pass < 2; pass++) {
      // These directories belong only to this fixture; removing them cannot touch another build.
      rmSync(join(app, '.brain', 'cache'), { recursive: true, force: true })
      rmSync(join(app, '.brain', 'build'), { recursive: true, force: true })
      const report = run(['build', '--json'], app, deadline)
      assert.ok(report.buildCache.misses > 0, 'each process performs actual cold-cache work')
      const login = report.report.find(row => row.route === '/login')
      assert.ok(login, 'default scaffold includes the actual login route')
      assert.ok(login.jsGz + login.cssGz + login.htmlGz <= 30 * 1024)
      const ledger = report.ledger.find(row => row.route === '/login')
      const schema = ledger.assets.find(asset => asset.source === 'runtime/brain-schema.js')
      assert.ok(schema, 'text validation remains present')
      const code = readFileSync(join(app, 'dist', schema.path), 'utf8')
      assert.ok(!code.includes('unknown type') && !code.includes('invalid bounds'), 'unused general schema parser is absent')
      outputs.push(inventory(join(app, 'dist')))
      signatures.push(report.build)
    }
    assert.deepEqual(outputs[1], outputs[0], 'all text, compressed, server and immutable asset bytes match across processes')
    assert.equal(signatures[1], signatures[0], 'build signatures match')
  } finally { rmSync(scratch, { recursive: true, force: true }) }
})
