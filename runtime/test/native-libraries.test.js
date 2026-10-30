import { test } from 'node:test'
import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { sibling } from './rt.js'
const { dateParse, dateAdd, dateDiff, dateRange, dateZone } = await sibling('brain-time.js')
const { hashText, shortHash } = await sibling('brain-hash.js')
test('native dates reject ambiguity, invalid calendar dates and overflow', () => {
  assert.equal(dateParse('2024-02-29'), 1709164800000)
  for (const input of ['2023-02-29', '2024-02-30', '10/12/2024', '2024-01-01T00:00', Infinity]) assert.throws(() => dateParse(input))
  assert.equal(dateParse('2024-01-01T01:00+01:00'), dateParse('2024-01-01'))
  assert.throws(() => dateAdd(8640000000000000, 1, 'day'))
})
test('UTC calendar addition clamps month and leap boundaries without mutating inputs', () => {
  const date = new Date('2024-01-31T12:00:00Z')
  assert.equal(new Date(dateAdd(date, 1, 'month')).toISOString(), '2024-02-29T12:00:00.000Z')
  assert.equal(new Date(dateAdd('2024-02-29', 1, 'year')).toISOString(), '2025-02-28T00:00:00.000Z')
  assert.equal(date.toISOString(), '2024-01-31T12:00:00.000Z')
  assert.equal(dateDiff('2024-03-11T04:00:00Z', '2024-03-10T05:00:00Z', 'hour'), 23)
  assert.throws(() => dateDiff(1, 0, 'month'))
})
test('ranges terminate with strict bounds and timezone projection tracks DST', () => {
  assert.equal(dateRange('2024-01-01', '2024-01-03').length, 3)
  assert.equal(dateRange('2024-01-03', '2024-01-01', { step: -1 }).length, 3)
  assert.throws(() => dateRange(0, 10, { unit: 'ms', limit: 10 }))
  assert.throws(() => dateRange(0, 10, { step: -1 }))
  assert.equal(dateZone('2024-03-10T06:00:00Z', 'America/New_York').offsetMinutes, -300)
  assert.equal(dateZone('2024-03-10T08:00:00Z', 'America/New_York').offsetMinutes, -240)
  assert.throws(() => dateZone(0, 'Invalid/Zone'))
  assert.equal(dateZone('0000-01-01T00:00:00Z', 'UTC').year, 0)
  assert.equal(dateZone('0000-01-01T00:00:00Z', 'UTC').offsetMinutes, 0)
})
test('Web Crypto hashes use published known vectors and explicit truncation', async () => {
  assert.equal(await hashText('abc'), 'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad')
  assert.equal(await hashText('abc', 'SHA-512'), 'ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f')
  assert.equal(await shortHash('abc', 8), 'ba7816bf')
  await assert.rejects(hashText('abc', 'MD5'))
  await assert.rejects(shortHash('abc', 0))
  await assert.rejects(hashText('😀'.repeat(2200000)))
})


test('UTC calendar operations stay valid at the epoch edges without intermediate overflow', () => {
  for (const instant of [-8640000000000000, 8640000000000000]) {
    for (const unit of ['month', 'year']) assert.equal(dateAdd(instant, 0, unit), instant)
  }
  assert.equal(dateAdd(8640000000000000, -1, 'month'), Date.parse('+275760-08-13T00:00:00.000Z'))
  assert.equal(dateAdd(-8640000000000000, 1, 'month'), Date.parse('-271821-05-20T00:00:00.000Z'))
  assert.equal(dateAdd(8640000000000000, -1, 'year'), Date.parse('+275759-09-13T00:00:00.000Z'))
  assert.throws(() => dateAdd(8640000000000000, 1, 'month'), /overflow/)
  assert.throws(() => dateAdd(-8640000000000000, -1, 'year'), /overflow/)
  assert.equal(new Date(dateAdd('0000-01-31', 1, 'month')).toISOString(), '0000-02-29T00:00:00.000Z')
})

test('singleton ranges validate options and month ranges use documented repeated clamping', () => {
  assert.throws(() => dateRange(0, 0, { unit: 'unknown' }), /supported unit/)
  assert.throws(() => dateRange(0, 0, { unit: 'month', step: 0.5 }), /integer calendar/)
  assert.deepEqual(dateRange('2024-01-31', '2024-03-31', { unit: 'month' }).map(value => new Date(value).toISOString()), ['2024-01-31T00:00:00.000Z', '2024-02-29T00:00:00.000Z', '2024-03-29T00:00:00.000Z'])
  assert.equal(dateRange(0, 9999, { unit: 'ms', limit: 10000 }).length, 10000)
  assert.throws(() => dateRange(0, 10000, { unit: 'ms', limit: 10000 }), /exceeds limit/)
  assert.throws(() => dateRange(0, 1, { unit: 'ms', step: 0.1 }), /does not advance/)
})

test('timezone projections retain finite offsets at epoch bounds, negative milliseconds and DST folds', () => {
  assert.equal(dateZone(8640000000000000, 'Asia/Tokyo').offsetMinutes, 540)
  const earliest = dateZone(-8640000000000000, 'America/New_York')
  assert.ok(Number.isFinite(earliest.offsetMinutes))
  assert.equal(earliest.day, 19)
  assert.equal(dateZone(-1, 'UTC').offsetMinutes, 0)
  assert.equal(dateZone('2024-11-03T05:30:00Z', 'America/New_York').hour, 1)
  assert.equal(dateZone('2024-11-03T06:30:00Z', 'America/New_York').hour, 1)
  assert.equal(dateZone('2024-11-03T05:30:00Z', 'America/New_York').offsetMinutes, -240)
  assert.equal(dateZone('2024-11-03T06:30:00Z', 'America/New_York').offsetMinutes, -300)
  assert.equal(dateZone('2024-01-01', 'Asia/Kathmandu').offsetMinutes, 345)
  assert.throws(() => dateZone(0, '+01:00'), /IANA/)
})

test('timezone formatter cache remains bounded across hits, invalid identifiers and evictions', () => {
  const Original = Intl.DateTimeFormat, seen = []
  Intl.DateTimeFormat = function (...args) { seen.push(args[1].timeZone); return new Original(...args) }
  try {
    dateZone(0, 'Etc/GMT+12')
    const before = seen.length
    dateZone(0, 'Etc/GMT+12')
    assert.equal(seen.length, before, 'a cache hit reuses the existing formatter')
    for (let offset = -12; offset <= 12; offset++) dateZone(0, `Etc/GMT${offset < 0 ? offset : '+' + offset}`)
    const after = seen.length
    dateZone(0, 'Etc/GMT+12')
    assert.equal(seen.length, after, 'a recently inserted key remains cached')
    for (let offset = -3; offset <= 12; offset++) dateZone(0, `Etc/GMT${offset < 0 ? offset : '+' + offset}`)
    assert.equal(seen.length, after, 'the last 16 inserted zones remain cached')
    dateZone(0, 'Etc/GMT-4')
    assert.equal(seen.length, after + 1, 'the seventeenth most recent insertion was evicted')
    assert.throws(() => dateZone(0, 'Invalid/Zone'))
    dateZone(0, 'Etc/GMT+12')
    assert.equal(seen.length, after + 2, 'invalid timezone construction does not evict a successful entry')
  } finally { Intl.DateTimeFormat = Original }
})

test('Web Crypto hashes UTF-8 correctly and reports unavailable secure-context capabilities', async () => {
  for (const text of ['', 'caf\u00e9 \u{1f600}', '\ud800']) for (const algorithm of ['SHA-256', 'SHA-384', 'SHA-512']) {
    assert.equal(await hashText(text, algorithm), createHash(algorithm.toLowerCase().replace('-', '')).update(text, 'utf8').digest('hex'))
  }
  assert.equal((await shortHash('caf\u00e9 \u{1f600}', 64)).length, 64)
  assert.notEqual(await hashText('\u00e9'), await hashText('e\u0301'), 'UTF-8 bytes are hashed without implicit Unicode normalization')
  const boundary = '\u00e9'.repeat(4194304)
  assert.equal(await hashText(boundary), createHash('sha256').update(boundary, 'utf8').digest('hex'))
  await assert.rejects(hashText(boundary + '\u00e9'), /UTF-8 input exceeds/)
  await assert.rejects(shortHash('value', 7), /8..64/)
  await assert.rejects(shortHash('value', 65), /8..64/)
  await assert.rejects(hashText(123), /text input/)
  const descriptor = Object.getOwnPropertyDescriptor(globalThis, 'crypto')
  try {
    Object.defineProperty(globalThis, 'crypto', { configurable: true, value: {} })
    await assert.rejects(hashText('value'), /secure browser context/)
  } finally { if (descriptor) Object.defineProperty(globalThis, 'crypto', descriptor); else delete globalThis.crypto }
})
