import { test } from 'node:test'
import assert from 'node:assert/strict'
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, rmSync, symlinkSync, existsSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { plan } from '../migrate-plan.mjs'
import { inspect, audit } from '../privacy-audit.mjs'
import { run, git } from '../lib/jobs.mjs'
import { main, policy } from '../evolve.mjs'

const fixture = t => { const p = mkdtempSync(join(tmpdir(), 'brain-tools-')); t.after(() => rmSync(p, { recursive: true, force: true })); return p }
test('migration inventories common frameworks without running scripts or following symlinks', t => {
  const root = fixture(t)
  for (const framework of ['react', '@angular/core', 'vue', 'svelte']) {
    writeFileSync(join(root, 'package.json'), JSON.stringify({ dependencies: { [framework]: '1' }, scripts: { test: 'exit 99' } }))
    mkdirSync(join(root, 'src/pages'), { recursive: true })
    writeFileSync(join(root, 'src/pages/home.tsx'), 'private source remains unread')
    assert.notEqual(plan(root).frameworks[0], 'unknown')
    assert.deepEqual(plan(root).routeCandidates, ['src/pages/home.tsx'])
  }
  symlinkSync('/nonexistent', join(root, 'outside'))
  assert.deepEqual(plan(root).skippedSymlinks, ['outside'])
})
test('privacy audit redacts findings and catches deleted historical credentials', t => {
  const root = fixture(t)
  git(root, 'init', '-q')
  git(root, 'config', 'user.name', 'Test')
  git(root, 'config', 'user.email', 'test@example.com')
  const secret = 'ghp_' + 'A'.repeat(36)
  writeFileSync(join(root, 'old.txt'), secret)
  git(root, 'add', '.'); git(root, 'commit', '-qm', 'fixture')
  git(root, 'rm', 'old.txt'); git(root, 'commit', '-qm', 'remove')
  assert.equal(audit(root).ok, true)
  const report = audit(root, { history: true })
  assert.equal(report.ok, false)
  assert.ok(!JSON.stringify(report).includes(secret))
  assert.equal(inspect('.env', Buffer.from('anything'))[0].rule, 'private-file')
})
test('jobs cap output, expire stalled groups and leave no owned child running', async t => {
  const root = fixture(t), log = join(root, 'out.log')
  const result = await run(process.execPath, ['-e', 'process.stdout.write("x".repeat(20000))'], { log, maxBytes: 1000 })
  assert.equal(readFileSync(log).length, 1000)
  assert.equal(result.truncated, true)
  await assert.rejects(run(process.execPath, ['-e', 'setInterval(()=>{}, 10)'], { log, timeout: 50 }), /deadline/)
})
test('evolution policy requires regression tests and prevents changing its own gates', () => {
  assert.throws(() => policy(['crates/brain-cli/src/main.rs']), /regression test/)
  assert.throws(() => policy(['scripts/verify.sh', 'runtime/test/a.test.js']), /gates/)
  assert.equal(policy(['crates/brain-check/src/lib.rs', 'crates/brain-check/tests/rules.rs']).needsLlmGate, true)
})
test('proposal lifecycle isolates edits, refuses dirty starting trees and unvalidated activation', async t => {
  const root = fixture(t)
  git(root, 'init', '-q'); git(root, 'config', 'user.name', 'Test'); git(root, 'config', 'user.email', 'test@example.com')
  writeFileSync(join(root, '.gitignore'), '.brain/\n')
  writeFileSync(join(root, 'original'), 'v1')
  git(root, 'add', '.'); git(root, 'commit', '-qm', 'fixture')
  const p = await main(['start', 'test-fix'], root)
  writeFileSync(join(p.worktree, 'original'), 'v2')
  assert.equal(readFileSync(join(root, 'original'), 'utf8'), 'v1')
  await assert.rejects(main(['activate', 'test-fix'], root), /must pass finish/)
  await assert.rejects(main(['discard', 'test-fix'], root), /Git operation failed/)
  writeFileSync(join(p.worktree, 'original'), 'v1')
  await main(['discard', 'test-fix'], root)
  assert.equal(existsSync(p.worktree), false)
  writeFileSync(join(root, 'original'), 'v3')
  await assert.rejects(main(['start', 'dirty'], root), /Working tree changed/)
})
