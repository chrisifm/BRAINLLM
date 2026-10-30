import { test } from 'node:test'
import assert from 'node:assert/strict'
import { install } from '../shim/dom.js'
install()
const { sibling } = await import('./rt.js')
const { form } = await sibling('brain-form.js')
test('invalid submits never run remote checks, cross-field hooks or submit side effects', async () => {
  let remote = 0, cross = 0, submitted = 0, coerced = 0
  const valid = { name: 'Ada', age: '12.5', day: '2024-02-29', active: false }
  const f = form({ name: 'str!', age: 'num!', day: 'date!', active: 'bool' }, { init: valid, check: { name: () => { remote++; return '' } }, validate: () => { cross++; return {} }, submit: () => { submitted++ } })
  for (const [key, value] of [['name', '  '], ['name', 12], ['age', []], ['age', { valueOf() { coerced++; return 12 } }], ['age', '0x10'], ['day', '2026-02-30'], ['active', 'true']]) {
    f.reset(valid); f.v[key] = value
    assert.equal(f.ok, false)
    await f.submit(); await f.submit()
    assert.ok(f.err[key]); assert.equal(remote, 0); assert.equal(cross, 0); assert.equal(submitted, 0); assert.equal(f.busy, false)
  }
  assert.equal(coerced, 0)
  f.reset(valid); await f.submit()
  assert.equal(remote, 1); assert.equal(cross, 1); assert.equal(submitted, 1)
})
test('optional blank numeric values and numeric lists match native server conversion', async () => {
  let values
  const f = form({ age: 'num', scores: '[num]', choices: '[bool]' }, { init: { age: ' ', scores: ['1', '2.5'], choices: [true, false] }, submit: value => { values = value } })
  assert.equal(f.ok, true); await f.submit()
  assert.deepEqual(values, { age: null, scores: [1, 2.5], choices: [true, false] })
})
test('form construction rejects an invalid nested schema before hooks can run', () => {
  let called = 0
  assert.throws(() => form({ child: { bad: 'unsupported' } }, { submit: () => { called++ } }), /unknown type/)
  assert.throws(() => form(null), /invalid/)
  assert.equal(called, 0)
})
