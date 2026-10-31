// urlState(initial, { key, values }) keeps the URL inside an allowed set; watch(src, cb, { immediate: true }) runs once
// with the initial value.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { install } from '../shim/dom.js'
const doc = install()
const rt = await import('./rt.js')
const { urlState } = await rt.sibling('brain-url.js')
const { watch } = await rt.sibling('brain-env.js')
const events = new Map(), writes = []
globalThis.addEventListener = (key, f) => { if (!events.has(key)) events.set(key, new Set()); events.get(key).add(f) }
globalThis.removeEventListener = (key, f) => events.get(key)?.delete(f)
globalThis.reportError = () => {}
globalThis.history = { state: null, pushState(state, _, url) { writes.push(url); globalThis.location = new URL(url) }, replaceState(state, _, url) { globalThis.location = new URL(url) } }
const flush = () => new Promise(r => setTimeout(r, 0))
const pop = url => { globalThis.location = new URL(url); for (const f of events.get('popstate') || []) f() }
const values = ['overview', 'details', 'reviews']

test('an unknown query value falls back to the first allowed value without writing history', async () => {
  globalThis.location = new URL('https://example.test/shop?tab=bogus')
  let tab
  const stop = rt.mount(() => { tab = urlState('details', { key: 'tab', values }); return doc.createElement('div') }, doc.body)
  assert.equal(tab.v, 'overview')
  await flush()
  assert.equal(writes.length, 0)
  pop('https://example.test/shop?tab=reviews')
  assert.equal(tab.v, 'reviews')
  pop('https://example.test/shop?tab=nope')
  assert.equal(tab.v, 'overview')
  pop('https://example.test/shop')
  assert.equal(tab.v, 'details', 'no query value keeps the declared initial value')
  stop()
})

test('an allowed query value is kept and an initial value outside the set starts at the first allowed one', () => {
  globalThis.location = new URL('https://example.test/shop?tab=details')
  let tab, sort
  const stop = rt.mount(() => {
    tab = urlState('overview', { key: 'tab', values })
    sort = urlState('random', { key: 'sort', values: ['new', 'old'] })
    return doc.createElement('div')
  }, doc.body)
  assert.equal(tab.v, 'details')
  assert.equal(sort.v, 'new')
  stop()
})

test('server rendering applies the same allowed set', () => {
  globalThis.__brainRoute = { path: '/shop', params: {}, query: { tab: 'bogus' } }
  try { assert.equal(urlState('details', { key: 'tab', values }).v, 'overview') } finally { delete globalThis.__brainRoute }
})

test('watch runs immediately with the initial value when asked, and later on changes', async () => {
  const seen = []
  let n
  const stop = rt.mount(() => { n = rt.sig(1); watch(() => n.v, (v, old) => seen.push([v, old]), { immediate: true }); return doc.createElement('div') }, doc.body)
  assert.deepEqual(seen, [[1, undefined]])
  n.v = 2
  await flush()
  assert.deepEqual(seen, [[1, undefined], [2, 1]])
  const lazy = []
  const stop2 = rt.mount(() => { watch(() => n.v, v => lazy.push(v)); return doc.createElement('div') }, doc.body)
  assert.deepEqual(lazy, [])
  stop(); stop2()
})
