// BRAINLLM schema mini-language (INTERNALS §14, §15): shared by form() (brain-form.js), api() validation
// (brain-data.js) and server.mjs, so a page that only fetches data does not load the form module.
// Per field: `str num bool email url date` + `!` required + `<=N >=N` (length for str and lists, value for num) + `[T]`
// list of T, or a nested schema object.
const RX_EMAIL = /^[^\s@]+@[^\s@]+\.[^\s@]+$/

// 'str!<=80' → { t: 'str', req: true, min, max, list: false }
export function rule(s) {
  if (typeof s === 'object') return { t: 'obj', s, req: false }
  const req = s.includes('!'), r = { t: '', req, list: false, min: null, max: null }
  let x = s.replace(/!/g, '')
  const m = /^(\[?)(\w+)\]?((?:[<>]=-?\d+(?:\.\d+)?)*)$/.exec(x)
  if (!m) throw Error(`form: invalid rule '${s}' (use str num bool email url date, ! required, <=N >=N, [T])`)
  r.list = !!m[1]
  r.t = m[2]
  for (const [, op, n] of m[3].matchAll(/([<>])=(-?\d+(?:\.\d+)?)/g)) r[op === '<' ? 'max' : 'min'] = +n
  return r
}

const empty = v => v == null || v === '' || (Array.isArray(v) && !v.length)
const plural = (n, w) => `${n} ${w}${n === 1 ? '' : 's'}`

// Message for one value, or '' when valid.
export function check(r, v) {
  if (r.list) {
    if (empty(v)) return r.req || r.min > 0 ? (r.req && !r.min ? 'Required' : `Choose at least ${r.min}`) : ''
    if (!Array.isArray(v)) return 'Must be a list'
    if (r.min != null && v.length < r.min) return `Choose at least ${r.min}`
    if (r.max != null && v.length > r.max) return `At most ${plural(r.max, 'item')}`
    const one = { ...r, list: false, req: true, min: null, max: null }
    for (let i = 0; i < v.length; i++) {
      const e = check(one, v[i])
      if (e) return `Item ${i + 1}: ${e}`
    }
    return ''
  }
  if (r.t === 'bool') return r.req && v !== true ? 'Required' : ''
  if (empty(v)) return r.req ? 'Required' : ''
  const s = String(v)
  switch (r.t) {
    case 'num': {
      const n = typeof v === 'number' ? v : s.trim() === '' ? NaN : Number(s)
      if (!Number.isFinite(n)) return 'Enter a number'
      if (r.min != null && n < r.min) return `Must be at least ${r.min}`
      if (r.max != null && n > r.max) return `Must be at most ${r.max}`
      return ''
    }
    case 'email':
      if (!RX_EMAIL.test(s)) return 'Enter a valid email'
      break
    case 'url':
      try {
        if (!/^https?:$/.test(new URL(s).protocol)) return 'Enter a valid URL'
      } catch {
        return 'Enter a valid URL'
      }
      break
    case 'date':
      if (Number.isNaN(Date.parse(s))) return 'Enter a valid date'
      return ''
    case 'str':
      break
    default:
      throw Error(`form: unknown type '${r.t}'`)
  }
  if (r.min != null && s.length < r.min) return `At least ${plural(r.min, 'character')}`
  if (r.max != null && s.length > r.max) return `At most ${plural(r.max, 'character')}`
  return ''
}
