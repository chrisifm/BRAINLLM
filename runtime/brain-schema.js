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
// form.url form.date form.min_chars form.max_chars.
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
    if (empty(v)) return r.req || r.min > 0 ? (r.req && !r.min ? msg('form.required', 'Required') : msg('form.min_items', `Choose at least ${r.min}`, { n: r.min })) : ''
    if (!Array.isArray(v)) return msg('form.list', 'Must be a list')
    if (r.min != null && v.length < r.min) return msg('form.min_items', `Choose at least ${r.min}`, { n: r.min })
    if (r.max != null && v.length > r.max) return msg('form.max_items', `At most ${plural(r.max, 'item')}`, { n: r.max })
    const one = { ...r, list: false, req: true, min: null, max: null }
    for (let i = 0; i < v.length; i++) {
      const e = check(one, v[i])
      if (e) return msg('form.item', `Item ${i + 1}: ${e}`, { n: i + 1, error: e })
    }
    return ''
  }
  if (r.t === 'bool') return r.req && v !== true ? msg('form.required', 'Required') : ''
  if (empty(v)) return r.req ? msg('form.required', 'Required') : ''
  const s = String(v)
  switch (r.t) {
    case 'num': {
      const n = typeof v === 'number' ? v : s.trim() === '' ? NaN : Number(s)
      if (!Number.isFinite(n)) return msg('form.number', 'Enter a number')
      if (r.min != null && n < r.min) return msg('form.min', `Must be at least ${r.min}`, { n: r.min })
      if (r.max != null && n > r.max) return msg('form.max', `Must be at most ${r.max}`, { n: r.max })
      return ''
    }
    case 'email':
      if (!RX_EMAIL.test(s)) return msg('form.email', 'Enter a valid email')
      break
    case 'url':
      try {
        if (!/^https?:$/.test(new URL(s).protocol)) return msg('form.url', 'Enter a valid URL')
      } catch {
        return msg('form.url', 'Enter a valid URL')
      }
      break
    case 'date':
      if (Number.isNaN(Date.parse(s))) return msg('form.date', 'Enter a valid date')
      return ''
    case 'str':
      break
    default:
      throw Error(`form: unknown type '${r.t}'`)
  }
  if (r.min != null && s.length < r.min) return msg('form.min_chars', `At least ${plural(r.min, 'character')}`, { n: r.min })
  if (r.max != null && s.length > r.max) return msg('form.max_chars', `At most ${plural(r.max, 'character')}`, { n: r.max })
  return ''
}
