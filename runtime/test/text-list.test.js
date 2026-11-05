import { test } from 'node:test'
import assert from 'node:assert/strict'
import { sibling } from './rt.js'
const { range } = await sibling('brain-list.js')
const { text } = await sibling('brain-text.js')

test('integer ranges are half-open, bounded and exact at safe integer endpoints', () => {
  assert.deepEqual(range(-2, 3), [-2, -1, 0, 1, 2])
  assert.deepEqual(range(3, 3), [])
  assert.deepEqual(range(3, 1), [])
  assert.equal(range(0, 10000).length, 10000)
  const end = Number.MAX_SAFE_INTEGER
  assert.deepEqual(range(end - 2, end), [end - 2, end - 1])
  for (const endpoints of [[0, 10001], [0, Infinity], [0.5, 2], ['0', 2], [0], [0, Number.MAX_SAFE_INTEGER + 1]]) assert.throws(() => range(...endpoints), RangeError)
})

test('text folding follows NFD, combining-mark removal and lowercase without locale state', () => {
  assert.equal(text.fold('CAF\u00c9'), 'cafe')
  assert.equal(text.fold('A\u0308'), 'a')
  assert.equal(text.fold('ASCII'), 'ascii')
  assert.equal(text.fold('\ud83d\ude00'), '\ud83d\ude00')
  assert.throws(() => text.fold(null), RangeError)
  assert.throws(() => text.fold('x'.repeat(4097)), RangeError)
})

const reference = (left, right) => {
  const a = Array.from(left), b = Array.from(right)
  const rows = Array.from({length:a.length + 1}, (_, i) => Array.from({length:b.length + 1}, (_, j) => i ? j ? 0 : i : j))
  for (let i = 1; i <= a.length; i++) for (let j = 1; j <= b.length; j++) {
    rows[i][j] = Math.min(rows[i - 1][j] + 1, rows[i][j - 1] + 1, rows[i - 1][j - 1] + (a[i - 1] === b[j - 1] ? 0 : 1))
    if (i > 1 && j > 1 && a[i - 1] === b[j - 2] && a[i - 2] === b[j - 1]) rows[i][j] = Math.min(rows[i][j], rows[i - 2][j - 2] + 1)
  }
  return rows[a.length][b.length]
}
test('bounded OSA distance agrees with an unbounded full-matrix oracle, including transpositions', () => {
  const words = ['']
  for (let length = 1; length <= 5; length++) for (let value = 0; value < 2 ** length; value++) words.push(value.toString(2).padStart(length, '0').replaceAll('0', 'a').replaceAll('1', 'b'))
  for (const left of words) for (const right of words) for (let max = 0; max <= 4; max++) assert.equal(text.distance(left, right, max), Math.min(reference(left, right), max + 1), `${left}/${right}/${max}`)
  assert.equal(text.distance('CA', 'ABC', 3), 3, 'OSA differs from unrestricted Damerau distance')
  assert.equal(text.distance('\ud83d\ude00a', 'a\ud83d\ude00', 1), 1)
  assert.equal(text.distance('x'.repeat(4096), 'x'.repeat(4095) + 'y', 1), 1)
  for (const max of [-1, 65, 0.5, Infinity, '2']) assert.throws(() => text.distance('a', 'b', max), RangeError)
  assert.throws(() => text.distance('a'.repeat(4097), ''), RangeError)
})

test('fuzzy matching folds inputs and follows per-length typo budgets while preserving candidates', () => {
  assert.deepEqual(text.fuzzy('cat', ['cat', 'Cat', 'cats', 'cut']), ['cat', 'Cat'])
  assert.deepEqual(text.fuzzy('book', ['BOOK', 'boook', 'bok', 'b00k']), ['BOOK', 'boook', 'bok'])
  assert.deepEqual(text.fuzzy('compiler', ['compiler', 'compielr', 'compilers', 'compi']), ['compiler', 'compielr', 'compilers'])
  assert.deepEqual(text.fuzzy('resume', ['r\u00e9sum\u00e9', 'resume', 'unrelated']), ['r\u00e9sum\u00e9', 'resume'])
  assert.throws(() => text.fuzzy('word', Array(10001).fill('word')), RangeError)
  assert.throws(() => text.fuzzy('word', [42]), RangeError)
  assert.throws(() => text.fuzzy('word', Array(1025).fill('x'.repeat(1024))), RangeError)
  const candidates = ['word']
  candidates[Symbol.iterator] = function* () { yield 'unrelated' }
  assert.deepEqual(text.fuzzy('word', candidates), ['word'], 'candidate indices bound work even with a replaced iterator')
})
