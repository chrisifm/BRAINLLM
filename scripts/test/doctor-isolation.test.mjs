// Doctor validates current source in private temporary output instead of replacing a possibly live dist/.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { mkdtempSync, mkdirSync, readFileSync, writeFileSync, readdirSync, rmSync } from 'node:fs'
import { join, relative } from 'node:path'
import { tmpdir } from 'node:os'
const brain = process.env.BRAIN_BIN ?? new URL('../../target/debug/brain', import.meta.url).pathname
const cli = (args, cwd) => spawnSync(brain, args, { cwd, encoding: 'utf8', timeout: 90000, maxBuffer: 4 * 1024 * 1024 })
function snapshot(root, at = root, out = []) {
  for (const entry of readdirSync(at, { withFileTypes: true })) {
    const path = join(at, entry.name)
    if (entry.isDirectory()) snapshot(root, path, out)
    else out.push([relative(root, path), createHash('sha256').update(readFileSync(path)).digest('hex')])
  }
  return out.sort((a, b) => a[0].localeCompare(b[0]))
}
test('ordinary and static doctor validate fresh isolated output and preserve live artifacts on success and build failure', { timeout: 240000 }, () => {
  const root = mkdtempSync(join(tmpdir(), 'brain-doctor-isolation-'))
  try {
    for (const mode of ['static', 'ordinary']) {
      const created = cli(['new', mode, ...(mode === 'static' ? ['--static'] : []), '--json'], root)
      assert.equal(created.status, 0, created.stdout + created.stderr)
      const app = join(root, mode), dist = join(app, 'dist')
      const built = cli(['build', '--site', 'https://example.test', '--json'], app)
      assert.equal(built.status, 0, built.stdout + built.stderr)
      writeFileSync(join(dist, 'preserve-deployment.bin'), Buffer.from([0, 1, 2, 3, 255]))
      const gracePath = join(app, '.brain/stale-chunks.json'), grace = '{"_b/retained123.js":1700000000}'
      mkdirSync(join(app, '.brain'), { recursive: true })
      writeFileSync(gracePath, grace)
      const before = snapshot(dist)
      const source = join(app, 'views/home.brain')
      const original = readFileSync(source, 'utf8')
      writeFileSync(source, original.replace('</main>', '<p>This current source is compiled privately for validation before a deliberate deployment.</p></main>'))
      const healthy = cli(['doctor', '--json'], app), report = JSON.parse(healthy.stdout)
      assert.equal(healthy.status, 0, healthy.stdout + healthy.stderr)
      assert.equal(report.build.outputScope, 'isolated-doctor-output')
      assert.equal(report.build.outputRetained, false)
      if (mode === 'static') assert.equal(report.staticArtifact.ok, true)
      assert.deepEqual(snapshot(dist), before)
      assert.equal(readFileSync(gracePath, 'utf8'), grace, 'doctor must not reset deployment chunk ages')
      writeFileSync(source, original.replace('</main>', '<a href="/missing-current-source/">Missing page</a></main>'))
      const rejected = cli(['doctor', '--json'], app), failure = JSON.parse(rejected.stdout)
      assert.notEqual(rejected.status, 0)
      assert.ok(failure.failed.includes('build'), rejected.stdout)
      assert.match(failure.build.error, /B1819/)
      assert.deepEqual(snapshot(dist), before)
      assert.equal(readFileSync(gracePath, 'utf8'), grace, 'failed validation preserves deployment chunk ages too')
    }
  } finally { rmSync(root, { recursive: true, force: true }) }
})
