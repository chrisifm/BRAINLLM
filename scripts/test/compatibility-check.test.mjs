import { test } from 'node:test'
import assert from 'node:assert/strict'
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { spawnSync } from 'node:child_process'
import { compareManifests, snapshotRepository, validateManifest } from '../compatibility-check.mjs'

function manifest(version = '1.2.3') {
  return { manifestVersion: 1, frameworkVersion: version, surfaces: { commands: { check: { signature: 'brain check [path]' } }, diagnostics: { B0001: { severity: 'error' } }, primitives: { sig: { signature: 'sig(value)' } }, outputs: { check: { schemaVersion: 1, outputSchema: { type: 'object', properties: { code: { type: 'string' } }, required: ['code'] } } } }, security: { failClosed: true } }
}
const copy = value => structuredClone(value)
const script = resolve('scripts/compatibility-check.mjs')

test('unchanged contract and patch metadata produce no manufactured baseline or diff', () => {
  const before = manifest(), after = copy(before)
  after.frameworkVersion = '1.2.4'; after.surfaces.commands.check.description = 'Updated prose'
  const result = compareManifests(before, after)
  assert.equal(result.policyPassed, true); assert.equal(result.compatible, true); assert.deepEqual(result.changes, [])
})
test('removed commands, diagnostics, primitives and output fields require major', () => {
  for (const surface of ['commands', 'diagnostics', 'primitives', 'outputs']) {
    const before = manifest(), after = copy(before)
    after.frameworkVersion = '1.3.0'; delete after.surfaces[surface][Object.keys(after.surfaces[surface])[0]]
    assert.equal(compareManifests(before, after).policyPassed, false, surface)
    after.frameworkVersion = '2.0.0'
    const result = compareManifests(before, after)
    assert.equal(result.policyPassed, true); assert.equal(result.compatible, false)
  }
})
test('rename remains breaking and a mechanical fix cannot waive the version policy', () => {
  const before = manifest(), after = copy(before)
  after.surfaces.primitives.signal = { signature: 'sig(value)', renamedFrom: 'sig', mechanicalFix: 'brain upgrade --rename sig=signal' }
  delete after.surfaces.primitives.sig
  after.frameworkVersion = '1.3.0'
  const result = compareManifests(before, after)
  assert.equal(result.policyPassed, false)
  assert.ok(result.changes.some(c => c.message.includes('renamed to signal')))
  assert.ok(result.warnings.some(w => w.includes('upgrade note')))
})
test('old alias preserved with deprecation is minor and emits migration warning', () => {
  const before = manifest(), after = copy(before)
  after.frameworkVersion = '1.3.0'
  after.surfaces.primitives.sig.deprecated = true
  after.surfaces.primitives.sig.replacement = 'signal'
  after.surfaces.primitives.signal = { signature: 'signal(value)' }
  const result = compareManifests(before, after)
  assert.equal(result.policyPassed, true); assert.equal(result.compatible, true)
  assert.ok(result.warnings.some(w => w.includes('deprecated')))
  after.frameworkVersion = '1.2.4'
  assert.equal(compareManifests(before, after).policyPassed, false)
})
test('pre-1.0 breaking changes require a minor bump; patch is insufficient', () => {
  const before = manifest('0.3.1'), after = copy(before)
  after.surfaces.primitives.sig.signature = 'sig(options, value)'
  after.frameworkVersion = '0.3.2'
  assert.equal(compareManifests(before, after).policyPassed, false)
  after.frameworkVersion = '0.4.0'
  assert.equal(compareManifests(before, after).policyPassed, true)
})
test('security invariants cannot be removed or disabled even by a major release', () => {
  for (const enabled of [false, undefined]) {
    const before = manifest(), after = copy(before); after.frameworkVersion = '2.0.0'
    if (enabled === undefined) delete after.security.failClosed
    else after.security.failClosed = enabled
    const result = compareManifests(before, after)
    assert.equal(result.policyPassed, false); assert.ok(result.errors.some(e => e.includes('cannot be weakened')))
  }
})
test('directional schema checks catch required input, removed output and wire version', () => {
  const before = manifest(), after = copy(before)
  before.surfaces.commands.check.inputSchema = { type: 'object', properties: { path: { type: 'string' } } }
  after.surfaces.commands.check.inputSchema = { type: 'object', properties: { path: { type: 'string' } }, required: ['path'] }
  after.surfaces.outputs.check.outputSchema.required = []
  after.surfaces.outputs.check.schemaVersion = 2
  after.frameworkVersion = '1.3.0'
  const result = compareManifests(before, after)
  assert.equal(result.policyPassed, false)
  assert.ok(result.changes.some(c => c.path.endsWith('inputSchema.required')))
  assert.ok(result.changes.some(c => c.message === 'output no longer guaranteed'))
  assert.ok(result.changes.some(c => c.path.endsWith('.schemaVersion')))
})
test('accepted input enum widening is additive; possible output enum widening is breaking', () => {
  const before = manifest(), after = copy(before)
  before.surfaces.commands.check.inputSchema = { enum: ['a'] }; after.surfaces.commands.check.inputSchema = { enum: ['a', 'b'] }
  after.frameworkVersion = '1.3.0'
  assert.equal(compareManifests(before, after).policyPassed, true)
  before.surfaces.outputs.check.outputSchema = { enum: ['a'] }; after.surfaces.outputs.check.outputSchema = { enum: ['a', 'b'] }
  assert.equal(compareManifests(before, after).policyPassed, false)
})
test('unknown schema constraints fail conservatively rather than masking incompatible changes', () => {
  const before = manifest(), after = copy(before)
  after.frameworkVersion = '1.3.0'; after.surfaces.outputs.check.outputSchema.pattern = '^x$'
  assert.equal(compareManifests(before, after).policyPassed, false)
})
test('invalid manifest versions, incomplete surfaces, malformed versions and downgrades fail', () => {
  assert.throws(() => validateManifest({}), /manifestVersion/)
  const incomplete = manifest(); delete incomplete.surfaces.primitives
  assert.throws(() => validateManifest(incomplete), /primitives/)
  for (const v of ['1.2', 'v1.2.3', '1.02.3', '1.2.3-01', '99999999999999999.0.0']) {
    const invalid = manifest(v); assert.throws(() => validateManifest(invalid), /semantic version/)
  }
  assert.equal(compareManifests(manifest('1.3.0'), manifest('1.2.9')).policyPassed, false)
  assert.equal(compareManifests(manifest('1.2.3'), manifest('1.2.3-beta.1')).policyPassed, false)
})
test('current repository snapshot is deterministic source inventory with explicit limitations', () => {
  const one = snapshotRepository(resolve('.')), two = snapshotRepository(resolve('.'))
  assert.deepEqual(one, two)
  assert.equal(one.provenance.baseline, false)
  assert.ok(one.provenance.limitations.includes('Not a released baseline'))
  assert.ok(one.surfaces.commands.check)
  assert.ok(one.surfaces.primitives.sig)
  assert.ok(one.surfaces.diagnostics.B0119)
  assert.ok(one.surfaces.outputs.diagnostics.fields.includes('code'))
  assert.ok(one.surfaces.outputs.diagnostics.fields.includes('endCol'))
})
test('CLI requires an external baseline, refuses overwrite, and uses documented exit codes', () => {
  const dir = mkdtempSync(join(tmpdir(), 'brain-compatibility-'))
  const call = args => spawnSync(process.execPath, [script, ...args], { encoding: 'utf8', timeout: 20000, maxBuffer: 4 * 1024 * 1024 })
  try {
    assert.equal(call(['compare', '--json']).status, 2)
    const snapshot = join(dir, 'inventory.json')
    assert.equal(call(['snapshot', '--output', snapshot]).status, 0)
    assert.equal(JSON.parse(readFileSync(snapshot, 'utf8')).provenance.baseline, false)
    assert.equal(call(['snapshot', '--output', snapshot]).status, 2)
    const base = join(dir, 'released.json'), next = join(dir, 'next.json')
    writeFileSync(base, JSON.stringify(manifest()))
    const current = manifest('1.2.4'); delete current.surfaces.commands.check
    writeFileSync(next, JSON.stringify(current))
    const report = call(['compare', '--baseline', base, '--current', next, '--json'])
    assert.equal(report.status, 1)
    assert.equal(JSON.parse(report.stdout).policyPassed, false)
    assert.equal(call(['compare', '--baseline', base, '--current', base]).status, 0)
    assert.equal(call(['compare', '--baseline', base, '--root', '.', '--current', next]).status, 2)
  } finally { rmSync(dir, { recursive: true, force: true }) }
})
