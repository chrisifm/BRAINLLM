import { test } from 'node:test'
import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { generate, render, status, verifyPinned, PIN } from '../gen-baseline.mjs'

const support = { chrome: '100', chrome_android: '100', edge: '100', firefox: '100', firefox_android: '100', safari: '16', safari_ios: '16' }
const baseline = (level = 'low', date = '2025-01-01') => ({ baseline: level, baseline_low_date: date, ...(level === 'high' ? { baseline_high_date: '2026-01-01' } : {}), support })
const feature = (keys, st = baseline(), overrides = {}) => ({ kind: 'feature', compat_features: keys, status: { ...st, by_compat_key: overrides } })
const globals = new Set(['Array', 'DataView', 'Set', 'Map', 'Object', 'Promise', 'Math', 'JSON', 'URL', 'Date'])
const fixture = {
  features: {
    'array-at': feature(['javascript.builtins.Array.at']),
    'array-from': feature(['javascript.builtins.Array.from']),
    'set-union': feature(['javascript.builtins.Set.union']),
    'float16': feature(['javascript.builtins.DataView.getFloat16']),
    'url-parse': feature(['api.URL.parse_static']),
    'object-legacy': feature(['javascript.builtins.Object.defineGetter', 'javascript.builtins.Object.proto']),
    'css-color': feature(['css.properties.color'], baseline('high', '2015-01-01')),
    'css-color-mix': feature(['css.types.color.color-mix']),
    'css-subgrid': feature(['css.properties.grid-template-columns.subgrid']),
    'custom-element-method': feature(['api.Element.moveBefore']),
    'unmapped': feature([]),
    'moved': { kind: 'moved', redirect_target: 'array-at' },
  },
}
const provenance = { version: 'test-fixture', published: '2025-01-01T00:00:00Z', dataSha256: 'fixture-sha256', integrity: 'fixture-integrity' }

test('builtins preserve receiver ownership and distinguish static versus prototype properties', () => {
  const { tables } = generate(fixture, globals)
  assert.ok(tables.paths.has('Array.prototype.at'))
  assert.ok(tables.paths.has('Array.from'))
  assert.ok(tables.paths.has('Set.prototype.union'))
  assert.ok(tables.paths.has('DataView.prototype.getFloat16'))
  assert.ok(tables.paths.has('URL.parse'))
  for (const wrong of ['Array.at', 'Set.union', 'DataView.getFloat16']) assert.equal(tables.paths.has(wrong), false, wrong)
  assert.equal(tables.members.get('Set.union').extra, 'Set')
  assert.equal(tables.members.has('union'), false, 'an unknown receiver is never matched by a leaf name')
  assert.equal(tables.members.get('Element.moveBefore').extra, 'Element')
  assert.equal(tables.paths.has('moveBefore'), false)
})

test('normalized BCD aliases map to real legacy property names', () => {
  const { tables } = generate(fixture, globals)
  assert.ok(tables.paths.has('Object.prototype.__defineGetter__'))
  assert.ok(tables.paths.has('Object.prototype.__proto__'))
  assert.equal(tables.paths.has('Object.defineGetter'), false)
})

test('CSS values and unmappable features retain raw evidence and explicit unknown classification', () => {
  const { tables } = generate(fixture, globals)
  assert.ok(tables.props.has('color'), 'old widely available features are retained, without a year cutoff')
  for (const key of ['css.types.color.color-mix', 'css.properties.grid-template-columns.subgrid']) {
    assert.ok(tables.raw.has(key))
    assert.ok(tables.unclassified.has(key))
  }
  assert.ok(tables.unclassified.has('feature:unmapped'))
  assert.equal(tables.props.has('color-mix'), false)
  assert.equal(tables.props.has('subgrid'), false)
})

test('headline fallback, key-specific overrides and upper-bound dates are preserved', () => {
  const data = { features: { mixed: feature(['api.URL.parse_static', 'javascript.builtins.Array.at'], baseline('high', '≤2018-10-02'), { 'api.URL.parse_static': baseline('low', '2025-04-01') }) } }
  const { tables } = generate(data, globals)
  assert.equal(tables.paths.get('URL.parse').status.widely, false)
  assert.equal(tables.members.get('Array.at').status.year, 2018)
  assert.equal(tables.members.get('Array.at').status.lowDate, '≤2018-10-02')
  assert.equal(status({ baseline: false, support: { chrome: '120' } }).mask, 1)
})

test('colliding evidence stays conservative and retains every source feature', () => {
  const data = { features: { older: feature(['css.properties.color'], baseline('high', '2015-01-01')), newer: feature(['css.properties.color'], baseline('low', '2025-05-01')) } }
  const record = generate(data, globals).tables.props.get('color')
  assert.equal(record.status.widely, false)
  assert.equal(record.status.year, 2025)
  assert.deepEqual(record.features, ['newer', 'older'])
})

test('generation is deterministic under source object order and carries provenance without synthetic polyfills', () => {
  const reversed = { features: Object.fromEntries(Object.entries(fixture.features).reverse()) }
  const a = render(fixture, globals, provenance), b = render(reversed, new Set([...globals].reverse()), provenance)
  assert.equal(a, b)
  assert.ok(a.includes('fixture-sha256'))
  assert.ok(a.includes('GLOBALS_SHA256'))
  assert.ok(a.includes('neither diagnostics, lowering nor polyfills'))
  assert.ok(!a.includes('NaN'))
})

test('import has no network or generation side effects and unverified data is rejected', () => {
  const url = new URL('../gen-baseline.mjs', import.meta.url).href
  const result = spawnSync(process.execPath, ['--input-type=module', '-e', `globalThis.fetch=()=>{throw Error('unexpected network')}; await import(${JSON.stringify(url)});`], { encoding: 'utf8', timeout: 5000, maxBuffer: 16384 })
  assert.equal(result.status, 0, result.stderr)
  assert.equal(result.stdout, '')
  assert.throws(() => verifyPinned(Buffer.from('{}')), /SHA-256/)
  assert.match(PIN.dataSha256, /^[a-f0-9]{64}$/)
  assert.match(PIN.integrity, /^sha512-/)
  assert.throws(() => generate({ features: [] }), /object/)
  assert.throws(() => status({ baseline: 'maybe', support }), /status/)
})
