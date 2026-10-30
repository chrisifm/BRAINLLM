import { test } from 'node:test'
import assert from 'node:assert/strict'
import { rule, check } from '../brain-schema.js'
import { callForm } from '../server/server.mjs'
const c = (schema, value) => check(rule(schema), value)
test('rules reject malformed lists, unknown types and incoherent bounds at creation', () => {
  for (const source of ['[str', 'str]', '[str]]', '[[str]]', 'str!!', '[str!]!', 'str<=2>=3', 'str>=-1', 'str<=1.5', '[num]<=-1', '[str]<=1.5', 'bool<=1', 'num>=1>=2', 'str<=1<=2', 'num<=99999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999']) assert.throws(() => rule(source), /invalid/)
  assert.throws(() => rule('unknown'), /unknown type/)
  for (const value of [null, 1, true, [], new Date(), { bad: 'unknown' }]) assert.throws(() => rule(value), /invalid|unknown/)
  const cycle = {}; cycle.child = cycle; assert.throws(() => rule(cycle), /invalid nested/)
  assert.throws(() => rule(JSON.parse('{"__proto__":"str"}')), /invalid schema/)
  let reads = 0; const getter = { get value() { reads++; return 'str' } }; assert.throws(() => rule(getter)); assert.equal(reads, 0)
  assert.equal(rule('[str!]<=4').req, true)
  assert.equal(rule('[str]!<=4').req, true)
  assert.equal(rule('num>=-1.5<=2.5').min, -1.5)
})
test('blank required fields and scalar types are checked without object coercion', () => {
  for (const source of ['str!', 'email!', 'url!', 'date!', 'num!']) assert.equal(c(source, ' \t\n'), 'Required')
  assert.equal(c('str', '   '), '')
  let coerced = 0; const object = { toString() { coerced++; return '12' }, valueOf() { coerced++; return 12 } }
  for (const source of ['str', 'num', 'email', 'url', 'date', 'bool']) for (const value of [object, [], {}, 123n, Symbol('x')]) assert.ok(c(source, value), source)
  assert.equal(coerced, 0)
  for (const value of [12, false, true]) assert.ok(c('str', value))
  for (const value of ['false', 'true', 0, 1, {}]) assert.equal(c('bool', value), 'Choose true or false')
  assert.equal(c('bool', false), ''); assert.equal(c('bool!', false), 'Required'); assert.equal(c('bool!', true), '')
  for (const value of ['', '   ', 'text', 1, {}]) assert.equal(c('[str]', value), 'Must be a list')
})
test('numeric HTML strings stay valid while nondecimal coercions are rejected', () => {
  for (const value of ['0', '-1.5', ' 2.5 ', '.5', '1e3', 0, 2.5]) assert.equal(c('num', value), '', String(value))
  for (const value of ['0x10', '0b10', 'Infinity', '1_000', '2px', true, [], [1], NaN, Infinity]) assert.equal(c('num', value), 'Enter a number')
})
test('dates check leap years, month lengths and clock fields without Date.parse normalization', () => {
  for (const value of ['2024-02-29', '2000-02-29', '2026-12-31', '2026-10-30T12:30', '2026-10-30T12:30:59.123Z', '2026-10-30T12:30:00-06:00']) assert.equal(c('date!', value), '', value)
  for (const value of ['2026-02-30', '2025-02-29', '1900-02-29', '2026-04-31', '2026-00-01', '2026-13-01', '2026-01-00', '0000-01-01', '2026-10-30T24:00', '2026-10-30T12:60', '2026-10-30T12:30:60Z', '10/30/2026', '2026', 1793318400000, new Date()]) assert.equal(c('date', value), 'Enter a valid date', String(value))
})
test('server form validation rejects blank numbers, impossible dates and disguised bool/hex before side effects', async () => {
  let calls = 0, received
  const module = { $forms: { save: { name: 'str!', age: 'num!', date: 'date!', active: 'bool' } }, save: (_, value) => { calls++; received = value } }
  const ctx = { url: new URL('https://example.test/form') }, rules = { rule, check }
  for (const fields of [{ name: ' ', age: '12', date: '2026-01-01' }, { name: 'Ada', age: ' ', date: '2026-01-01' }, { name: 'Ada', age: '0x10', date: '2026-01-01' }, { name: 'Ada', age: '12', date: '2026-02-30' }, { name: 'Ada', age: '12', date: '2026-01-01', active: 'arbitrary' }]) {
    const raw = new URLSearchParams({ _brain: 'h/save', ...fields }).toString()
    const result = await callForm({ module, raw, ctx, rules }); assert.equal(result.status, 422); assert.equal(calls, 0)
  }
  const raw = new URLSearchParams({ _brain: 'h/save', name: 'Ada', age: '12.5', date: '2024-02-29', active: 'on' }).toString()
  assert.equal((await callForm({ module, raw, ctx, rules })).status, 303)
  assert.equal(calls, 1); assert.deepEqual(received, { name: 'Ada', age: 12.5, date: '2024-02-29', active: true })
})
