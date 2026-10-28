// BRAINLLM languages (INTERNALS §37): our own small standard, no library. Static `t('key')` calls are resolved by the
// compiler; this module serves what is left at runtime (plurals, dynamic keys, islands) plus formatting and the switcher.
// Dictionaries arrive per view and locale as chunks that call `add(locale, { key: text | { one, other, … } })` before
// the page mounts; only the current locale is ever loaded (switching language is a full load of the other URL).
const G = globalThis
// locale → { key: text | plural object }. One table for the whole process: the server and the prerender may load this
// module twice (their own copy and the build's), and a dictionary added through either must be seen by both (BUILD)
const D = (G.__brainDict ||= {})
const RTL = /^(ar|he|fa|ur|ps|sd|ug|yi|dv)(-|$)/i
let warned = null

export function add(loc, dict) {
  Object.assign((D[loc] ||= {}), dict)
}
// The page's language: the server sets __brainLocale while it renders; in the browser it is <html lang>.
export function locale() {
  return G.__brainLocale || (G.document && document.documentElement && document.documentElement.getAttribute('lang')) || 'en'
}
const def = () => (G.document && document.documentElement && document.documentElement.getAttribute('data-default-lang')) || 'en'
export const dirOf = loc => (RTL.test(loc) ? 'rtl' : 'ltr')

// t('posts.count', { n: 3 }) → '3 posts'. A plural object is chosen by Intl.PluralRules on `n` (or `count`); `=0`-style
// exact keys win. `{var}` placeholders: numbers are formatted for the locale. A missing key falls back to the default
// locale, then to the key itself (reported once in dev).
export function t(key, vars) {
  const loc = locale()
  let m = (D[loc] && D[loc][key]) ?? (D[def()] && D[def()][key])
  if (m == null) {
    if (G.__brainDev && (warned ||= new Set()) && !warned.has(key)) warned.add(key), console.warn(`i18n: missing key "${key}" for ${loc}`)
    return key
  }
  if (typeof m === 'object') {
    const n = vars && (vars.n ?? vars.count)
    m = m['=' + n] ?? m[pr(loc).select(+n || 0)] ?? m.other ?? ''
    if (vars && vars.n == null) vars = { ...vars, n } // `{n}` works with either name
  }
  return vars ? m.replace(/\{(\w+)\}/g, (s, k) => (k in vars ? (typeof vars[k] === 'number' ? fmt.number(vars[k]) : String(vars[k])) : s)) : m
}

const cache = new Map()
const memo = (k, f) => (cache.has(k) ? cache.get(k) : (cache.set(k, f()), cache.get(k)))
const pr = loc => memo('p' + loc, () => new Intl.PluralRules(loc))
const nf = (loc, o) => memo('n' + loc + JSON.stringify(o), () => new Intl.NumberFormat(loc, o))
const df = (loc, o) => memo('d' + loc + JSON.stringify(o), () => new Intl.DateTimeFormat(loc, o))
const rf = loc => memo('r' + loc, () => new Intl.RelativeTimeFormat(loc, { numeric: 'auto' }))
const UNITS = [['year', 31536e6], ['month', 2592e6], ['week', 6048e5], ['day', 864e5], ['hour', 36e5], ['minute', 6e4], ['second', 1e3]]

// Formatting for the current locale, over Intl (cached formatters).
export const fmt = {
  number: (n, o) => nf(locale(), o).format(n),
  money: (n, currency = 'USD', o) => nf(locale(), { style: 'currency', currency, ...o }).format(n),
  date: (d, o = { dateStyle: 'medium' }) => df(locale(), o).format(new Date(d)),
  // "3 days ago", "in 2 hours", "yesterday": the largest unit that fits
  relative(d, now = Date.now()) {
    const ms = new Date(d) - now
    for (const [u, size] of UNITS) if (Math.abs(ms) >= size || u === 'second') return rf(locale()).format(Math.round(ms / size), u)
  },
}

// The same page in every language of the site, from the <link rel="alternate" hreflang> tags the build emits; keeps the
// current query and hash; each language named in itself ("Deutsch", "Nederlands"). One language → [] (switcher hidden).
// While the server renders (SSR or prerender) the links come from globalThis.__brainAlternates ([{ locale, href }], set
// per route from the manifest), so the switcher is in the HTML before any JS runs (§38).
export function alternates() {
  const d = G.document, cur = locale(), out = []
  const links = G.__brainAlternates || (d && d.head ? [...d.head.querySelectorAll('link[rel=alternate][hreflang]')].map(l => ({ locale: l.getAttribute('hreflang'), href: l.getAttribute('href') })) : [])
  const here = G.location && G.location.href
  for (const l of links) {
    if (l.locale === 'x-default') continue
    let href = l.href
    if (here) {
      const u = new URL(l.href, here)
      u.search = G.location.search
      u.hash = G.location.hash
      href = u.pathname + u.search + u.hash
    }
    out.push({ locale: l.locale, href, name: nameOf(l.locale), current: l.locale === cur })
  }
  return out.length > 1 ? out : []
}
const nameOf = loc => {
  try {
    const n = new Intl.DisplayNames([loc], { type: 'language' }).of(loc)
    return n.charAt(0).toLocaleUpperCase(loc) + n.slice(1)
  } catch {
    return loc
  }
}

// One-time suggestion: the browser prefers another language this site has (never an automatic redirect, §37).
// Returns { locale, href, name } once per browser, or null; `dismiss()` remembers the answer.
const SEEN = 'brain:lang-suggested'
export function suggest() {
  try {
    if (G.localStorage && localStorage.getItem(SEEN)) return null
  } catch {}
  const alts = alternates(), cur = locale()
  if (!alts.length || !G.navigator) return null
  for (const want of navigator.languages || [navigator.language]) {
    const base = String(want).toLowerCase().split('-')[0]
    if (cur.toLowerCase().split('-')[0] === base) return null // already in a language the user reads
    const a = alts.find(x => x.locale.toLowerCase() === String(want).toLowerCase() || x.locale.toLowerCase().split('-')[0] === base)
    if (a) return { locale: a.locale, href: a.href, name: a.name }
  }
  return null
}
export function dismiss() {
  try {
    G.localStorage && localStorage.setItem(SEEN, '1')
  } catch {}
}
