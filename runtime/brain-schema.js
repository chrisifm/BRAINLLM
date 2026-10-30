// BRAINLLM schema mini-language (INTERNALS §14, §15): shared by form() (brain-form.js), api() validation
// (brain-data.js) and server.mjs, so a page that only fetches data does not load the form module.
// Per field: `str num bool email url date` + `!` required + `<=N >=N` (length for str and lists, value for num) + `[T]`
// list of T, or a nested schema object.
const RX_EMAIL = /^[^\s@]+@[^\s@]+\.[^\s@]+$/
const G = globalThis

// Validation messages (§14, §37): the app's dictionary wins when it has the key (`"form.required": "Obligatorio"` in
// i18n/<locale>.json; plural objects allowed for keys with `{n}`), else the English text. The table is read directly
// (globalThis.__brainDict, filled by brain-i18n.js `add`) so a page that only validates does not load brain-i18n.js.
// Keys: form.required form.list form.min_items form.max_items form.item form.number form.min form.max form.email
// form.url form.date form.text form.boolean form.min_chars form.max_chars.
export function msg(key, en, vars) {
  const D = G.__brainDict
  if (D) {
    const h = G.document && G.document.documentElement
    const loc = G.__brainLocale || (h && h.getAttribute('lang')) || 'en'
    const def = (h && h.getAttribute('data-default-lang')) || 'en'
    let m = (D[loc] && D[loc][key]) ?? (D[def] && D[def][key])
    if (m != null) {
      if (typeof m === 'object') {
        const n = vars && vars.n
        m = m['=' + n] ?? m[new Intl.PluralRules(loc).select(+n || 0)] ?? m.other ?? en
      }
      return vars ? String(m).replace(/\{(\w+)\}/g, (s, k) => (k in vars ? String(vars[k]) : s)) : String(m)
    }
  }
  return en
}

// 'str!<=80' → { t: 'str', req: true, min, max, list: false }
const TYPES = new Set(['str', 'num', 'bool', 'email', 'url', 'date'])
function scalarRule(s) {
  if (typeof s !== 'string' || s.length > 256) throw Error('form: invalid rule (use str num bool email url date, ! required, <=N >=N, [T])')
  const m = /^(?:([a-z]+)(!)?|\[([a-z]+)(!)?\](!)?)((?:[<>]=-?\d+(?:\.\d+)?)*)$/.exec(s)
  if (!m || m[4] && m[5]) throw Error(`form: invalid rule '${s}' (use str num bool email url date, ! required, <=N >=N, [T])`)
  const r = { t: m[1] || m[3], req: !!(m[2] || m[4] || m[5]), list: !!m[3], min: null, max: null }
  if (!TYPES.has(r.t)) throw Error(`form: unknown type '${r.t}'`)
  for (const [, op, n] of m[6].matchAll(/([<>])=(-?\d+(?:\.\d+)?)/g)) {
    const key = op === '<' ? 'max' : 'min', value = Number(n)
    if (r[key] != null || !Number.isFinite(value) || (!r.list && r.t === 'bool') || ((r.list || r.t !== 'num') && (!Number.isSafeInteger(value) || value < 0))) throw Error(`form: invalid bounds in rule '${s}'`)
    r[key] = value
  }
  if (r.min != null && r.max != null && r.min > r.max) throw Error(`form: invalid bounds in rule '${s}'`)
  return r
}
function schemaRules(schema, seen, budget, depth) {
  if (!schema || Array.isArray(schema) || typeof schema !== 'object' || ![null, Object.prototype].includes(Object.getPrototypeOf(schema)) || seen.has(schema) || depth > 16) throw Error('form: invalid nested schema')
  seen.add(schema)
  for (const key of Object.keys(schema)) {
    const field = Object.getOwnPropertyDescriptor(schema, key)
    if (++budget.n > 4096 || ['__proto__', 'prototype', 'constructor'].includes(key) || !field || !('value' in field)) throw Error('form: invalid schema field')
    if (field.value && typeof field.value === 'object') schemaRules(field.value, seen, budget, depth + 1)
    else scalarRule(field.value)
  }
  seen.delete(schema)
}
export function rule(s) {
  if (s && typeof s === 'object') {
    schemaRules(s, new Set(), { n: 0 }, 0)
    return { t: 'obj', s, req: false }
  }
  return scalarRule(s)
}
const empty = v => v == null || typeof v === 'string' && v.trim() === '' || Array.isArray(v) && !v.length
const NUMBER = /^[+-]?(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][+-]?\d+)?$/
// Check calendar fields directly; Date.parse may normalize impossible days or accept locale-specific formats.
function calendarDate(s) {
  const m = /^(\d{4})-(\d{2})-(\d{2})(?:T([01]\d|2[0-3]):([0-5]\d)(?::([0-5]\d)(?:\.\d{1,9})?)?(?:Z|[+-](?:[01]\d|2[0-3]):[0-5]\d)?)?$/.exec(s)
  if (!m) return false
  const year = +m[1], month = +m[2], day = +m[3]
  const days = [31, year % 4 === 0 && (year % 100 !== 0 || year % 400 === 0) ? 29 : 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
  return year > 0 && month > 0 && month <= 12 && day > 0 && day <= days[month - 1]
}

const plural = (n, w) => `${n} ${w}${n === 1 ? '' : 's'}`

// Compiler-only validators for proven unbounded scalars. Separate entry points let
// text-only forms omit numeric syntax and boolean validation from their dependency graph.
export function checkText(r, value) {
  if (Array.isArray(value)) return msg('form.text', 'Enter text')
  if (empty(value)) return r.req ? msg('form.required', 'Required') : ''
  if (r.t === 'email' && (typeof value !== 'string' || !RX_EMAIL.test(value))) return msg('form.email', 'Enter a valid email')
  return typeof value === 'string' ? '' : msg('form.text', 'Enter text')
}
export function checkScalar(r, value) {
  if (r.t !== 'bool' && r.t !== 'num') return checkText(r, value)
  if (Array.isArray(value)) return msg(r.t === 'num' ? 'form.number' : 'form.boolean', r.t === 'num' ? 'Enter a number' : 'Choose true or false')
  if (empty(value)) return r.req ? msg('form.required', 'Required') : ''
  if (r.t === 'bool') return typeof value !== 'boolean' ? msg('form.boolean', 'Choose true or false') : r.req && !value ? msg('form.required', 'Required') : ''
  return Number.isFinite(typeof value === 'number' ? value : typeof value === 'string' && NUMBER.test(value.trim()) ? Number(value) : NaN) ? '' : msg('form.number', 'Enter a number')
}

// Message for one value, or '' when valid.
export function check(r, v) {
  if (r.list) {
    if (v != null && !Array.isArray(v)) return msg('form.list', 'Must be a list')
    if (empty(v)) return r.req || r.min > 0 ? (r.req && !r.min ? msg('form.required', 'Required') : msg('form.min_items', `Choose at least ${r.min}`, { n: r.min })) : ''
    if (!Array.isArray(v)) return msg('form.list', 'Must be a list')
    if (r.min != null && v.length < r.min) return msg('form.min_items', `Choose at least ${r.min}`, { n: r.min })
    if (r.max != null && v.length > r.max) return msg('form.max_items', `At most ${plural(r.max, 'item')}`, { n: r.max })
    const one = { ...r, list: false, req: true, min: null, max: null }
    for (let i = 0; i < v.length; i++) {
      const e = r.t === 'bool' ? typeof v[i] === 'boolean' ? '' : msg('form.boolean', 'Choose true or false') : check(one, v[i])
      if (e) return msg('form.item', `Item ${i + 1}: ${e}`, { n: i + 1, error: e })
    }
    return ''
  }
  if (Array.isArray(v)) return msg(r.t === 'num' ? 'form.number' : r.t === 'bool' ? 'form.boolean' : 'form.text', r.t === 'num' ? 'Enter a number' : r.t === 'bool' ? 'Choose true or false' : 'Enter text')
  if (empty(v)) return r.req ? msg('form.required', 'Required') : ''
  if (r.t === 'bool') return typeof v !== 'boolean' ? msg('form.boolean', 'Choose true or false') : r.req && !v ? msg('form.required', 'Required') : ''
  const s = typeof v === 'string' ? v : ''
  switch (r.t) {
    case 'num': {
      const n = typeof v === 'number' ? v : typeof v === 'string' && NUMBER.test(s.trim()) ? Number(s) : NaN
      if (!Number.isFinite(n)) return msg('form.number', 'Enter a number')
      if (r.min != null && n < r.min) return msg('form.min', `Must be at least ${r.min}`, { n: r.min })
      if (r.max != null && n > r.max) return msg('form.max', `Must be at most ${r.max}`, { n: r.max })
      return ''
    }
    case 'email':
      if (typeof v !== 'string' || !RX_EMAIL.test(s)) return msg('form.email', 'Enter a valid email')
      break
    case 'url':
      if (typeof v !== 'string') return msg('form.url', 'Enter a valid URL')
      try {
        if (!/^https?:$/.test(new URL(s).protocol)) return msg('form.url', 'Enter a valid URL')
      } catch {
        return msg('form.url', 'Enter a valid URL')
      }
      break
    case 'date':
      if (typeof v !== 'string' || !calendarDate(s)) return msg('form.date', 'Enter a valid date')
      break
    case 'str':
      if (typeof v !== 'string') return msg('form.text', 'Enter text')
      break
    default:
      throw Error(`form: unknown type '${r.t}'`)
  }
  if (r.min != null && s.length < r.min) return msg('form.min_chars', `At least ${plural(r.min, 'character')}`, { n: r.min })
  if (r.max != null && s.length > r.max) return msg('form.max_chars', `At most ${plural(r.max, 'character')}`, { n: r.max })
  return ''
}
