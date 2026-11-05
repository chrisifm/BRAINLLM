// Pure bounded Unicode matching; no locale, network, DOM or persistent state.
const letters = value => {
  if (typeof value !== 'string' || value.length > 8192) throw new RangeError('text helpers require strings of at most 4096 Unicode characters')
  const result = Array.from(value)
  if (result.length > 4096) throw new RangeError('text helpers require strings of at most 4096 Unicode characters')
  return result
}
const fold = value => {
  letters(value)
  const result = value.normalize('NFD').replace(/[\u0300-\u036f]/g, '').toLowerCase()
  letters(result)
  return result
}
// Optimal string alignment: adjacent transpositions count as one edit; the band and row minimum bound work.
const distance = (left, right, max = 2) => {
  if (!Number.isInteger(max) || max < 0 || max > 64) throw new RangeError('text.distance max must be an integer from 0 to 64')
  const a = letters(left), b = letters(right), n = a.length, m = b.length, cap = max + 1
  if (Math.abs(n - m) > max) return cap
  if (!n) return Math.min(m, cap)
  if (!m) return Math.min(n, cap)
  let before = new Uint16Array(m + 1).fill(cap)
  let previous = new Uint16Array(m + 1).fill(cap)
  let current = new Uint16Array(m + 1).fill(cap)
  for (let j = 0; j <= Math.min(m, max); j++) previous[j] = j
  for (let i = 1; i <= n; i++) {
    const low = Math.max(1, i - max), high = Math.min(m, i + max)
    current[0] = Math.min(i, cap)
    if (low > 1) current[low - 1] = cap
    if (high < m) current[high + 1] = cap
    let minimum = current[0]
    for (let j = low; j <= high; j++) {
      let value = Math.min(previous[j] + 1, current[j - 1] + 1, previous[j - 1] + (a[i - 1] === b[j - 1] ? 0 : 1), cap)
      if (i > 1 && j > 1 && a[i - 1] === b[j - 2] && a[i - 2] === b[j - 1]) value = Math.min(value, before[j - 2] + 1)
      current[j] = value
      minimum = Math.min(minimum, value)
    }
    if (minimum > max) return cap
    const spare = before; before = previous; previous = current; current = spare
  }
  return Math.min(previous[m], cap)
}
const fuzzy = (word, list) => {
  const needle = fold(word), size = letters(needle).length
  if (!Array.isArray(list) || list.length > 10000) throw new RangeError('text.fuzzy requires at most 10000 candidate strings')
  const count = list.length, max = size < 4 ? 0 : size < 8 ? 1 : 2
  const found = []
  let units = 0
  for (let i = 0; i < count; i++) {
    const candidate = list[i]
    if (typeof candidate !== 'string' || candidate.length > 1048576 - units) throw new RangeError('text.fuzzy candidate text exceeds 1048576 UTF-16 code units or contains a non-string')
    units += candidate.length
    if (distance(needle, fold(candidate), max) <= max) found.push(candidate)
  }
  return found
}
export const text = Object.freeze({ fold, distance, fuzzy })
