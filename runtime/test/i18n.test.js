// brain-i18n.js (INTERNALS §37): t() with plurals and placeholders, fallback, fmt over Intl, alternates, suggestion.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { install } from '../shim/dom.js'

const doc = install()
const rt = await import('./rt.js')
const { add, t, locale, fmt, dirOf, alternates, suggest, dismiss } = await rt.sibling('brain-i18n.js')
const lang = (l, def = 'en') => (doc.documentElement.setAttribute('lang', l), doc.documentElement.setAttribute('data-default-lang', def))

add('en', { 'nav.home': 'Home', 'hi': 'Hello {name}', 'posts.count': { '=0': 'No posts', one: '{n} post', other: '{n} posts' }, 'only.en': 'English only' })
add('nl', { 'nav.home': 'Startpagina', 'hi': 'Hallo {name}', 'posts.count': { one: '{n} bericht', other: '{n} berichten' } })
add('ar', { 'posts.count': { zero: 'صفر', one: 'واحد', two: 'اثنان', few: '{n} قليل', many: '{n} كثير', other: '{n} آخر' } })

test('t(): current locale, {vars}, plurals by Intl.PluralRules with exact =N, numbers formatted for the locale', () => {
  lang('en')
  assert.equal(locale(), 'en')
  assert.equal(t('nav.home'), 'Home')
  assert.equal(t('hi', { name: 'Ada' }), 'Hello Ada')
  assert.equal(t('posts.count', { n: 0 }), 'No posts')
  assert.equal(t('posts.count', { n: 1 }), '1 post')
  assert.equal(t('posts.count', { n: 1234 }), '1,234 posts')
  lang('nl')
  assert.equal(t('nav.home'), 'Startpagina')
  assert.equal(t('posts.count', { count: 1 }), '1 bericht')
  assert.equal(t('posts.count', { n: 1234 }), '1.234 berichten') // grouped the Dutch way
  lang('ar')
  assert.equal(t('posts.count', { n: 2 }), 'اثنان')
  assert.equal(t('posts.count', { n: 5 }), fmt.number(5) + ' قليل') // digits as Intl formats them for ar
})

test('t(): a key missing in the locale falls back to the default locale, then to the key (warned once in dev)', () => {
  lang('nl', 'en')
  assert.equal(t('only.en'), 'English only')
  const warns = []
  const w = console.warn
  console.warn = m => warns.push(m)
  globalThis.__brainDev = true
  assert.equal(t('nope.key'), 'nope.key')
  t('nope.key')
  console.warn = w
  delete globalThis.__brainDev
  assert.equal(warns.length, 1)
  globalThis.__brainLocale = 'en' // what server.mjs sets while it renders a page
  assert.equal(t('nav.home'), 'Home')
  delete globalThis.__brainLocale
})

test('fmt: number, money, date and relative time for the current locale; dir', () => {
  lang('en')
  assert.equal(fmt.number(1234.5), '1,234.5')
  assert.equal(fmt.money(9.5, 'USD'), '$9.50')
  assert.equal(fmt.date('2026-10-13T12:00:00Z', { dateStyle: 'long', timeZone: 'UTC' }), 'October 13, 2026')
  const now = Date.parse('2026-10-13T12:00:00Z')
  assert.equal(fmt.relative(now - 864e5, now), 'yesterday')
  assert.equal(fmt.relative(now + 2 * 36e5, now), 'in 2 hours')
  lang('nl')
  assert.equal(fmt.money(9.5, 'EUR'), '€\u00a09,50') // Intl puts a no-break space
  assert.equal(fmt.relative(now - 3 * 864e5, now), '3 dagen geleden')
  assert.equal(dirOf('ar'), 'rtl')
  assert.equal(dirOf('nl-BE'), 'ltr')
})

test('alternates(): one link per locale (x-default skipped), current query and hash kept, names in their own language', () => {
  lang('en')
  globalThis.location = new URL('https://site.test/blog/post?ref=x#c2')
  doc.head.innerHTML =
    '<link rel="alternate" hreflang="en" href="https://site.test/blog/post"><link rel="alternate" hreflang="nl" href="https://site.test/nl/blog/post">' +
    '<link rel="alternate" hreflang="de" href="https://site.test/de/blog/post"><link rel="alternate" hreflang="x-default" href="https://site.test/blog/post">'
  const a = alternates()
  assert.deepEqual(a.map(x => [x.locale, x.href, x.name, x.current]), [
    ['en', '/blog/post?ref=x#c2', 'English', true],
    ['nl', '/nl/blog/post?ref=x#c2', 'Nederlands', false],
    ['de', '/de/blog/post?ref=x#c2', 'Deutsch', false],
  ])
  doc.head.innerHTML = '<link rel="alternate" hreflang="en" href="https://site.test/blog/post">'
  assert.deepEqual(alternates(), []) // one language: the switcher hides
})

test('suggest(): once, only for a language the site has and the page is not already in', () => {
  lang('en')
  globalThis.location = new URL('https://site.test/')
  doc.head.innerHTML = '<link rel="alternate" hreflang="en" href="/"><link rel="alternate" hreflang="nl" href="/nl/">'
  const ls = new Map()
  globalThis.localStorage = { getItem: k => ls.get(k) ?? null, setItem: (k, v) => ls.set(k, v) }
  const nav0 = Object.getOwnPropertyDescriptor(globalThis, 'navigator')
  Object.defineProperty(globalThis, 'navigator', { configurable: true, value: { languages: ['nl-BE', 'en'] } })
  assert.deepEqual(suggest(), { locale: 'nl', href: '/nl/', name: 'Nederlands' })
  dismiss()
  assert.equal(suggest(), null) // only once
  ls.clear()
  Object.defineProperty(globalThis, 'navigator', { configurable: true, value: { languages: ['en-GB'] } })
  assert.equal(suggest(), null) // already English
  Object.defineProperty(globalThis, 'navigator', { configurable: true, value: { languages: ['fr'] } })
  assert.equal(suggest(), null) // the site has no French
  if (nav0) Object.defineProperty(globalThis, 'navigator', nav0)
})

test('alternates() while the server renders: from __brainAlternates (no document head, no location)', () => {
  const loc0 = globalThis.location
  delete globalThis.location
  globalThis.__brainLocale = 'nl'
  globalThis.__brainAlternates = [{ locale: 'en', href: '/about' }, { locale: 'nl', href: '/nl/over' }]
  assert.deepEqual(alternates().map(x => [x.locale, x.href, x.current]), [['en', '/about', false], ['nl', '/nl/over', true]])
  delete globalThis.__brainAlternates
  delete globalThis.__brainLocale
  globalThis.location = loc0
})
