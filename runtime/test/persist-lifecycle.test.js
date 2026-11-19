import { test } from 'node:test'
import assert from 'node:assert/strict'
import { sibling } from './rt.js'
const { _rt } = await sibling('brain.js')
const { store, ready } = await sibling('brain-ch.js')
const I = _rt(), values = new Map(), page = new EventTarget(), doc = new EventTarget()
globalThis.localStorage = { getItem: k => values.get(k) ?? null, setItem: (k, v) => values.set(k, v), removeItem: k => values.delete(k) }
globalThis.addEventListener = page.addEventListener.bind(page)
globalThis.removeEventListener = page.removeEventListener.bind(page)
globalThis.document = doc
const read = id => JSON.parse(values.get('brain:p:' + id)).d

test('small changes survive the effect flush without waiting for a debounce', async () => {
  let s, stop
  I.root(dispose => { stop = dispose; s = store({ n: 0 }, { persist: true }, 'immediate') })
  await ready()
  try {
    s.n = 7
    await Promise.resolve()
    assert.deepEqual(read('immediate'), { n: 7 })
  } finally { stop() }
})

test('pagehide saves pending changes before the effect queue runs', async () => {
  let s, stop
  I.root(dispose => { stop = dispose; s = store({ n: 0 }, { persist: true }, 'leaving') })
  await ready()
  try {
    s.n = 8
    page.dispatchEvent(new Event('pagehide'))
    assert.deepEqual(read('leaving'), { n: 8 })
  } finally { stop() }
})

test('hidden documents flush and disposed owners stop saving', async () => {
  let s, stop
  I.root(dispose => { stop = dispose; s = store({ n: 0 }, { persist: true }, 'hidden') })
  await ready()
  try {
    s.n = 9
    doc.visibilityState = 'hidden'
    doc.dispatchEvent(new Event('visibilitychange'))
    assert.deepEqual(read('hidden'), { n: 9 })
  } finally { stop() }
  s.n = 10
  page.dispatchEvent(new Event('pagehide'))
  await Promise.resolve()
  assert.deepEqual(read('hidden'), { n: 9 })
})

test('disposal during the dynamic import preserves the final small snapshot', async () => {
  I.root(dispose => {
    const s = store({ n: 0 }, { persist: true }, 'early-disposal')
    s.n = 11
    dispose()
  })
  await ready()
  assert.deepEqual(read('early-disposal'), { n: 11 })
})
