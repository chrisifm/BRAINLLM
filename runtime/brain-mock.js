// BRAINLLM fake data (brain run --mock, INTERNALS §15, #159). Development only: brain-data never imports it, so no page
// in production carries it. `fake(schema, seed)` → data that matches an api() schema (form()'s mini-language, nested
// objects, [Schema] lists), deterministic: the same schema and seed give the same JSON on any machine. Field names steer
// the values (email, name, title, city, url, phone, price, date, id…) so screens look real; bounds and required hold.
// `mocker(seed)` is what `http({ mock })` takes: the same seed and URL give the same answer; a write answers its input
// merged over a fake `out`. TOOLCHAIN #266 wires it: `http({ mock: mocker(seed) })`.
import { rule } from './brain-schema.js'

const hashOf = s => {
  let h = 2166136261
  for (let i = 0; i < s.length; i++) h = Math.imul(h ^ s.charCodeAt(i), 16777619)
  return h >>> 0
}
const prng = seed => {
  let a = hashOf(seed)
  return () => {
    a = (a + 0x6d2b79f5) | 0
    let t = Math.imul(a ^ (a >>> 15), 1 | a)
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296
  }
}
const FIRST = ['Ada', 'Grace', 'Linus', 'Margaret', 'Alan', 'Barbara', 'Ken', 'Radia', 'Dennis', 'Frances', 'Tim', 'Katherine']
const LAST = ['Lovelace', 'Hopper', 'Torvalds', 'Hamilton', 'Turing', 'Liskov', 'Thompson', 'Perlman', 'Ritchie', 'Allen', 'Berners-Lee', 'Johnson']
const WORDS = ['quiet', 'bright', 'river', 'signal', 'garden', 'orbit', 'paper', 'stone', 'harbor', 'window', 'field', 'summer', 'north', 'cedar', 'lantern', 'meadow']
const CITIES = ['Lisbon', 'Kyoto', 'Montreal', 'Oslo', 'Puebla', 'Cape Town', 'Melbourne', 'Lima', 'Seoul', 'Vienna']
function fakeField(r, key, rnd, i, who) {
  const k = String(key || '').toLowerCase(), pick = a => a[Math.floor(rnd() * a.length)]
  const words = n => Array.from({ length: n }, () => pick(WORDS)).join(' ')
  const cap = w => w.charAt(0).toUpperCase() + w.slice(1)
  if (r.t === 'bool') return rnd() < 0.5
  if (r.t === 'num') {
    const lo = r.min ?? (/(^|_)id$|count|qty|quantity|age/.test(k) ? 1 : 0), hi = r.max ?? (/age/.test(k) ? 90 : /price|amount|total|cost/.test(k) ? 500 : /(^|_)id$/.test(k) ? 10000 : 100)
    if (/(^|_)id$/.test(k) && i != null) return i + 1 // ids of a list are unique and stable
    const v = lo + rnd() * (hi - lo)
    return /price|amount|total|cost|rate/.test(k) ? Math.round(v * 100) / 100 : Math.round(v)
  }
  if (r.t === 'date') return new Date(Date.UTC(2026, 0, 1) + Math.floor(rnd() * 365) * 864e5).toISOString().slice(0, 10)
  const { first, last } = who || { first: pick(FIRST), last: pick(LAST) } // one person per object: name and email agree
  let v
  if (r.t === 'email' || /email/.test(k)) v = `${first}.${last}`.toLowerCase().replace(/[^a-z.]/g, '') + '@example.com'
  else if (r.t === 'url' || /url|link|website|href/.test(k)) v = `https://example.com/${pick(WORDS)}`
  else if (/(^|_)id$|slug/.test(k)) v = (i != null ? `${pick(WORDS)}-${i + 1}` : `${pick(WORDS)}-${Math.floor(rnd() * 1000)}`)
  else if (/^name$|full_?name|author|user/.test(k)) v = `${first} ${last}`
  else if (/first/.test(k)) v = first
  else if (/last|surname/.test(k)) v = last
  else if (/city|town/.test(k)) v = pick(CITIES)
  else if (/phone|tel/.test(k)) v = `+1 555 01${String(Math.floor(rnd() * 100)).padStart(2, '0')}`
  else if (/title|subject|headline/.test(k)) v = cap(words(3))
  else if (/body|text|content|description|summary|bio|note/.test(k)) v = cap(words(12)) + '.'
  else if (/color|colour/.test(k)) v = '#' + Math.floor(rnd() * 0xffffff).toString(16).padStart(6, '0')
  else if (/status|state/.test(k)) v = pick(['active', 'pending', 'done'])
  else v = cap(words(2))
  if (r.max != null && v.length > r.max) v = v.slice(0, r.max).trim()
  if (r.min != null && v.length < r.min) v = v.padEnd(r.min, 'x')
  return v
}
function gen(s, key, rnd, i, who) {
  if (Array.isArray(s)) return Array.from({ length: 3 + Math.floor(rnd() * 4) }, (_, j) => gen(s[0], key, rnd, j))
  if (s && typeof s === 'object') {
    const o = {}, me = { first: FIRST[Math.floor(rnd() * FIRST.length)], last: LAST[Math.floor(rnd() * LAST.length)] }
    for (const k of Object.keys(s)) o[k] = gen(s[k], k, rnd, i, me)
    return o
  }
  let r
  try {
    r = rule(s)
  } catch {
    return null // a type name the compiler could not resolve: nothing to imitate
  }
  if (r.t === 'obj') return gen(r.s, key, rnd, i)
  if (r.list) return Array.from({ length: Math.max(r.min || 0, 3) }, (_, j) => fakeField(r, key, rnd, j, who))
  return fakeField(r, key, rnd, i, who)
}
export const fake = (schema, seed = 'brain') => gen(schema, '', prng(String(seed)), null)

export const mocker = (seed = 'brain') => (method, url, def, body) => {
  const out = def && def.out ? fake(def.out, seed + ' ' + method + ' ' + url) : null
  return method === 'GET' || !out || typeof out !== 'object' || Array.isArray(out) ? out : { ...out, ...body }
}
