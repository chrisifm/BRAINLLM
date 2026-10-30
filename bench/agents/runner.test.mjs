import { test } from 'node:test'
import assert from 'node:assert/strict'
import { mkdtemp, mkdir, writeFile, readFile, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { config, fixture, metrics, execute, run, sourceHash } from './runner.mjs'

const base = { command: ['explicit-agent'], model: 'explicit-model', effort: 'explicit-effort', binary: '/explicit/binary', out: '/explicit/result', validators: [['explicit-validator']] }
test('requires explicit invocation, model, effort, validators and bounded configuration', () => {
  assert.throws(() => config({ ...base, command: 'agent --model paid' }), /JSON array/)
  for (const field of ['model', 'effort', 'binary', 'out']) assert.throws(() => config({ ...base, [field]: '' }), /required/)
  assert.throws(() => config({ ...base, timeoutMs: 3600001 }), /timeoutMs/)
  assert.throws(() => config({ ...base, validators: [] }), /validators/)
  assert.throws(() => fixture({ kind: 'create', prompt: 'x', files: { '../escape': 'x' } }), /Unsafe/)
})
test('does not fabricate token counts, turns or first-pass from exit status', () => {
  assert.deepEqual(metrics(null), { firstPass: null, turns: null, inputTokens: null, outputTokens: null, interventions: null, provenance: 'not-reported' })
  assert.equal(metrics({ firstPass: false, turns: 2 }).provenance, 'adapter-reported')
  assert.equal(metrics({ turns: 2 }).inputTokens, null)
  assert.throws(() => metrics({ turns: -1 }), /Invalid/)
})
test('bounds output and wall-clock time of owned commands', async () => {
  const limits = { timeoutMs: 500, maxOutputBytes: 1024 }
  const output = await execute([process.execPath, '-e', "process.stdout.write('x'.repeat(4096));setInterval(()=>{},1000)"], tmpdir(), '', limits)
  assert.equal(output.reason, 'output-limit')
  assert.ok(output.stdout.length <= 1024)
  const timeout = await execute([process.execPath, '-e', 'setInterval(()=>{},1000)'], tmpdir(), '', { ...limits, timeoutMs: 50 })
  assert.equal(timeout.reason, 'timeout')
  assert.ok(timeout.durationMs < 2000)
})
test('replay records stable source/binary/context hashes and locally verified output', async () => {
  const root = await mkdtemp(join(tmpdir(), 'brain-bench-test-'))
  try {
    for (const directory of ['crates', 'runtime', 'bench/agents']) await mkdir(join(root, directory), { recursive: true })
    for (const path of ['SPEC.md', 'Cargo.lock', 'Cargo.toml']) await writeFile(join(root, path), path)
    const original = await sourceHash(root)
    await writeFile(join(root, 'runtime/.brain_env'), 'PRIVATE=not-a-real-secret')
    await mkdir(join(root, 'runtime/.brain'), { recursive: true })
    await writeFile(join(root, 'runtime/.brain/cache'), 'private cache')
    await writeFile(join(root, 'runtime/private.key'), 'not a real key')
    assert.equal((await sourceHash(root)).sha256, original.sha256)
    const replay = join(root, 'replay.json')
    await writeFile(replay, JSON.stringify({ files: { 'result.txt': 'verified' }, report: {} }))
    const out = join(root, 'result.json')
    const result = await run({ ...base, binary: process.execPath, out, command: [process.execPath, resolve('bench/agents/replay-adapter.mjs'), replay], validators: [[process.execPath, '-e', "if(require('fs').readFileSync('result.txt','utf8')!=='verified')process.exit(1)"]] }, [{ kind: 'create', prompt: 'Replay only', files: {} }], root)
    assert.equal(result.results[0].passed, true)
    assert.equal(result.results[0].metrics.firstPass, null)
    assert.equal(result.stable, true)
    assert.match(result.binarySha256, /^[a-f0-9]{64}$/)
    assert.match(result.results[0].inputSha256, /^[a-f0-9]{64}$/)
    assert.equal(JSON.parse(await readFile(out)).runId, result.runId)
  } finally { await rm(root, { recursive: true, force: true }) }
})

test('bundled tasks cover every declared benchmark category with safe seeds', async () => {
  const tasks = JSON.parse(await readFile(new URL('./fixtures/tasks.json', import.meta.url), 'utf8'))
  assert.deepEqual(tasks.map(task => fixture(task).kind), ['create', 'edit', 'repair', 'atomic-conflict', 'CRUD'])
})
