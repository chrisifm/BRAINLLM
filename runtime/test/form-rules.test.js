// tel and file field types, and cross-field rules through the `validate` option.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { rule, check } from '../brain-schema.js'

const fake = (name, type, size) => ({ name, type, size })

test('tel accepts common written forms and rejects what cannot be dialled', () => {
  const r = rule('tel!')
  for (const ok of ['+52 55 5155 0000', '(555) 123-4567', '555.123.4567', '5551234', '+14155552671']) assert.equal(check(r, ok), '', ok)
  for (const bad of ['123456', 'call me', '+1 (415) 555-2671 ext', '1234567890123456', '++5551234', '555-123-456x', '+']) assert.notEqual(check(r, bad), '', bad)
  assert.equal(check(rule('tel'), ''), '', 'optional blank is fine')
  assert.equal(check(r, ''), 'Required')
  assert.notEqual(check(r, 5551234), '', 'a number is not a written phone')
})

test('file checks presence, size in bytes and accepted types', () => {
  const r = rule('file(image/*,.pdf)!<=1000')
  assert.equal(check(r, null), 'Required')
  assert.equal(check(r, fake('a.png', 'image/png', 1000)), '')
  assert.equal(check(r, fake('REPORT.PDF', '', 10)), '', 'extension is case-insensitive')
  assert.match(check(r, fake('a.png', 'image/png', 1001)), /1000/)
  assert.match(check(r, fake('a.exe', 'application/x-msdownload', 10)), /type/i)
  assert.notEqual(check(r, 'a.png'), '', 'a file name is not a file')
  assert.notEqual(check(r, { name: 'a.png' }), '', 'an object without a size is not a file')
  assert.equal(check(rule('file'), fake('any.bin', '', 0)), '')
  assert.equal(check(rule('file'), null), '')
  assert.notEqual(check(rule('file>=10'), fake('a', '', 9)), '')
})

test('a file list counts items and applies the accepted types to each', () => {
  const r = rule('[file(.pdf)]!<=2')
  assert.equal(check(r, [fake('a.pdf', '', 1)]), '')
  assert.notEqual(check(r, [fake('a.pdf', '', 1), fake('b.pdf', '', 1), fake('c.pdf', '', 1)]), '')
  assert.match(check(r, [fake('a.png', 'image/png', 1)]), /Item 1/)
})

test('invalid tel and file rules fail when the form is built', () => {
  for (const bad of ['file()', 'file(image)', 'file(.p df)', 'str(.pdf)', 'tel(.pdf)', 'file(.pdf)!!', 'file<=-1', 'file<=1.5']) assert.throws(() => rule(bad), /form:/, bad)
})
