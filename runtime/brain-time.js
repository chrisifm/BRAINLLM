// Pure native date operations. Calendar arithmetic is UTC; timezone projection never changes the instant.
const MS = { ms: 1, second: 1000, minute: 60000, hour: 3600000, day: 86400000, week: 604800000 }
let zones
const monthDays = (y, m) => m === 1 ? (y % 4 === 0 && (y % 100 !== 0 || y % 400 === 0) ? 29 : 28) : [31, 0, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31][m]
export function dateParse(value) {
  if (value instanceof Date) value = value.getTime()
  if (typeof value === 'number') {
    const instant = new Date(value).getTime()
    if (!Number.isFinite(value) || !Number.isFinite(instant)) throw RangeError('dateParse: invalid epoch milliseconds')
    return instant
  }
  if (typeof value !== 'string') throw TypeError('dateParse: use epoch milliseconds, Date or an ISO date')
  const match = /^(\d{4})-(\d{2})-(\d{2})(?:T\d{2}:\d{2}(?::\d{2}(?:\.\d{1,3})?)?(?:Z|[+-]\d{2}:\d{2}))?$/.exec(value)
  if (!match) throw RangeError('dateParse: datetime strings require an explicit timezone')
  const y = +match[1], m = +match[2], d = +match[3]
  if (m < 1 || m > 12 || d < 1 || d > monthDays(y, m - 1)) throw RangeError('dateParse: invalid calendar date')
  const result = Date.parse(value)
  if (!Number.isFinite(result)) throw RangeError('dateParse: invalid ISO date')
  return result
}
export function dateAdd(value, amount, unit = 'day') {
  const start = dateParse(value)
  if (!Number.isFinite(amount)) throw RangeError('dateAdd: amount must be finite')
  if (Object.hasOwn(MS, unit)) return dateParse(start + amount * MS[unit])
  if (!['month', 'year'].includes(unit) || !Number.isSafeInteger(amount)) throw RangeError('dateAdd: use a supported unit and integer calendar amount')
  const date = new Date(start), day = date.getUTCDate()
  const months = date.getUTCFullYear() * 12 + date.getUTCMonth() + (unit === 'month' ? amount : 0)
  const year = Math.floor(months / 12) + (unit === 'year' ? amount : 0), month = months - Math.floor(months / 12) * 12
  // Set the complete calendar date once: day 1 or the end of a month can be outside Date's valid range.
  date.setUTCFullYear(year, month, Math.min(day, monthDays(year, month)))
  if (!Number.isFinite(date.getTime())) throw RangeError('dateAdd: calendar overflow')
  return dateParse(date)
}
export function dateDiff(left, right, unit = 'day') {
  if (!Object.hasOwn(MS, unit)) throw RangeError('dateDiff: elapsed units are ms, second, minute, hour, day or week')
  return (dateParse(left) - dateParse(right)) / MS[unit]
}
export function dateRange(start, end, options = {}) {
  let cursor = dateParse(start)
  const stop = dateParse(end), step = options.step ?? 1, unit = options.unit ?? 'day', limit = options.limit ?? 1000
  if (!Number.isInteger(limit) || limit < 1 || limit > 10000 || !Number.isFinite(step) || step === 0) throw RangeError('dateRange: nonzero step and limit 1..10000 required')
  if (!Object.hasOwn(MS, unit) && !(['month', 'year'].includes(unit) && Number.isSafeInteger(step))) throw RangeError('dateRange: supported unit and integer calendar step required')
  if ((stop - cursor) * step < 0) throw RangeError('dateRange: step must point toward the end')
  const out = []
  while (step > 0 ? cursor <= stop : cursor >= stop) {
    if (out.length === limit) throw RangeError('dateRange: result exceeds limit')
    out.push(cursor)
    if (cursor === stop) break
    const next = dateAdd(cursor, step, unit)
    if (step > 0 ? next <= cursor : next >= cursor) throw RangeError('dateRange: step does not advance')
    cursor = next
  }
  return out
}
export function dateZone(value, timeZone) {
  const instant = dateParse(value)
  if (typeof timeZone !== 'string' || !timeZone || timeZone.length > 128 || /^[+-]/.test(timeZone)) throw TypeError('dateZone: explicit IANA timezone required')
  zones ||= new Map()
  let formatter = zones.get(timeZone)
  if (!formatter) {
    formatter = new Intl.DateTimeFormat('en-US-u-ca-gregory-nu-latn', { timeZone, era: 'short', year: 'numeric', month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit', second: '2-digit', hourCycle: 'h23' })
    if (zones.size === 16) zones.delete(zones.keys().next().value)
    zones.set(timeZone, formatter)
  }
  const parts = {}
  for (const part of formatter.formatToParts(instant)) if (part.type !== 'literal') parts[part.type] = part.type === 'era' ? part.value : Number(part.value)
  if (parts.era === 'BC') parts.year = 1 - parts.year
  // Gregorian calendars repeat every 400 years. Project both calendar dates into the same safe cycle,
  // preserving the difference even when the local wall clock lies beyond Date's global epoch bounds.
  const reference = new Date(instant), cycle = Math.floor(reference.getUTCFullYear() / 400) * 400
  reference.setUTCFullYear(reference.getUTCFullYear() - cycle)
  reference.setUTCMilliseconds(0)
  const local = new Date(0)
  local.setUTCFullYear(parts.year - cycle, parts.month - 1, parts.day)
  local.setUTCHours(parts.hour, parts.minute, parts.second, 0)
  return { timeZone, instant, year: parts.year, month: parts.month, day: parts.day, hour: parts.hour, minute: parts.minute, second: parts.second, offsetMinutes: (local.getTime() - reference.getTime()) / 60000 }
}
