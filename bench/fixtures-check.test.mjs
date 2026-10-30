import test from 'node:test'
import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { mkdtempSync, readFileSync, readdirSync, writeFileSync, rmSync } from 'node:fs'
import { join } from 'node:path'
import { tmpdir } from 'node:os'
import { fileURLToPath } from 'node:url'

const root = fileURLToPath(new URL('..', import.meta.url))
const warning = { file: 'views/dashboard.brain', code: 'B0912', line: 6, col: 3, level: 'warning', msg: 'Imported JavaScript has no execution checkpoints.', fix: 'Audit the import boundary or isolate heavy work in a worker.' }
const invalid = readdirSync(join(root, 'examples/invalid'), { recursive: true })
  .filter(path => path.endsWith('.expected.json')).sort().flatMap(path => {
    const expected = JSON.parse(readFileSync(join(root, 'examples/invalid', path), 'utf8'))
    return expected.map(diag => ({ ...diag, level: diag.level ?? 'error', msg: diag.msg || 'Expected diagnostic', fix: diag.fix || 'Expected correction' }))
  })
assert.ok(invalid.length, 'The fake CLI replays the real invalid fixture expectations')

function run(valid, broken = invalid) {
  const scratch = mkdtempSync(join(tmpdir(), 'brain-fixture-gate-'))
  try {
    const payload = join(scratch, 'diagnostics.json'), binary = join(scratch, 'brain.mjs')
    writeFileSync(payload, JSON.stringify({ valid, invalid: broken }))
    writeFileSync(binary, `#!${process.execPath}\nimport {readFileSync} from 'node:fs';const data=JSON.parse(readFileSync(process.env.FIXTURE_GATE_DATA,'utf8'));process.stdout.write(JSON.stringify(process.argv.at(-1)==='invalid'?data.invalid:data.valid))\n`, { mode: 0o700 })
    const result = spawnSync(process.execPath, [join(root, 'bench/fixtures-check.mjs'), binary], {
      cwd: root, env: { PATH: process.env.PATH, FIXTURE_GATE_DATA: payload }, encoding: 'utf8', timeout: 5000, maxBuffer: 256 * 1024,
    })
    assert.equal(result.error, undefined, String(result.error))
    return result
  } finally { rmSync(scratch, { recursive: true }) }
}

test('accepts the one exact documented foreign JavaScript warning', () => {
  const result = run([warning])
  assert.equal(result.status, 0, result.stdout + result.stderr)
  assert.match(result.stdout, /valid examples: OK 1 diagnostics/)
})

test('rejects missing, additional or relocated warnings and wrong severity/code', () => {
  for (const diagnostics of [[], [warning, warning], [warning, { ...warning, code: 'B2005' }],
    ...Object.entries({ file: 'views/other.brain', code: 'B2101', line: 7, col: 4, level: 'error' }).map(([field, value]) => [{ ...warning, [field]: value }])]) {
    const result = run(diagnostics)
    assert.equal(result.status, 1, JSON.stringify(diagnostics) + result.stdout + result.stderr)
  }
})

test('requires actionable nonempty message and fix for the accepted warning', () => {
  for (const field of ['msg', 'fix']) for (const value of ['', '   ', null]) {
    assert.equal(run([{ ...warning, [field]: value }]).status, 1)
  }
})

test('the new valid-warning oracle does not relax invalid fixture comparisons', () => {
  assert.equal(run([warning], invalid.slice(1)).status, 1)
  assert.equal(run([warning], invalid.map((diag, i) => i === 0 ? { ...diag, col: diag.col + 1 } : diag)).status, 1)
})
