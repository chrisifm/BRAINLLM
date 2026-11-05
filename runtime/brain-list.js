// Pure bounded integer iteration, outside the runtime core.
export function range(from, to) {
  if (!Number.isSafeInteger(from) || !Number.isSafeInteger(to)) throw new RangeError('range requires two safe integer endpoints')
  const length = Math.max(0, to - from)
  if (length > 10000) throw new RangeError('range cannot emit more than 10000 integers')
  const values = new Array(length)
  for (let i = 0; i < length; i++) values[i] = from + i
  return values
}
