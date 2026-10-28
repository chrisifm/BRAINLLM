// store(init, { persist: true }) → brain-persist.js (INTERNALS §35.4): localStorage below 4 KB restored before the first
// render, IndexedDB above, versioned by the shape of init, quota/private-mode errors → memory, reported once.
import { test } from 'node:test'
import assert from 'node:assert/strict'

const rt = await import('./rt.js')
const { store, ready } = await rt.sibling('brain-ch.js')
const wait = (ms = 150) => new Promise(r => setTimeout(r, ms))

// fake localStorage
let ls = new Map(), lsFail = null
globalThis.localStorage = {
  getItem: k => (ls.has(k) ? ls.get(k) : null),
  setItem: (k, v) => {
    if (lsFail) throw lsFail
    ls.set(k, String(v))
  },
  removeItem: k => ls.delete(k),
}
// fake IndexedDB (one database, one object store), with structured clone and async requests
const idb = new Map()
const rq = f => {
  const r = {}
  queueMicrotask(() => {
    try {
      r.result = f()
      r.onsuccess && r.onsuccess()
    } catch (e) {
      r.error = e
      r.onerror && r.onerror()
    }
  })
  return r
}
const kv = { get: k => rq(() => structuredClone(idb.get(k))), put: (v, k) => rq(() => void idb.set(k, structuredClone(v))), delete: k => rq(() => void idb.delete(k)) }
globalThis.indexedDB = {
  open: () => {
    const r = { result: { createObjectStore() {}, transaction: () => ({ objectStore: () => kv }) } }
    queueMicrotask(() => (r.onupgradeneeded && r.onupgradeneeded(), r.onsuccess()))
    return r
  },
}
const reports = []
globalThis.reportError = e => reports.push(e)

test('small state: saved to localStorage, restored synchronously on the next load (before the first render)', async () => {
  const a = store({ n: 0, tags: [] }, { persist: true }, 'p:small')
  a.n = 5
  a.tags = ['x']
  await wait()
  assert.deepEqual(JSON.parse(ls.get('brain:p:p:small')).d, { n: 5, tags: ['x'] })
  const b = store({ n: 0, tags: [] }, { persist: true }, 'p:small') // "reload"
  assert.equal(b.n, 5) // already there, no await
  assert.deepEqual(b.tags, ['x'])
})

test('changed shape: the old data is discarded instead of being applied', async () => {
  const c = store({ n: 0, tags: '' }, { persist: true }, 'p:small') // tags was a list, now a string
  assert.equal(c.n, 0)
  assert.equal(c.tags, '')
  assert.equal(ls.has('brain:p:p:small'), false)
})

test('large state: IndexedDB plus a marker; restored before ready() resolves; shrinking moves it back', async () => {
  const big = Array.from({ length: 400 }, (_, i) => ({ id: i, title: 'item ' + i }))
  const a = store({ items: [] }, { persist: true }, 'p:big')
  a.items = big
  await wait()
  assert.deepEqual(JSON.parse(ls.get('brain:p:p:big')).idb, 1)
  assert.equal(idb.get('brain:p:p:big').d.items.length, 400)
  const b = store({ items: [] }, { persist: true }, 'p:big')
  assert.equal(b.items.length, 0) // IndexedDB is async…
  await ready()
  assert.equal(b.items.length, 400) // …and done before ready() resolves (the page mounts after it)
  b.items = [{ id: 1 }]
  await wait()
  assert.equal(idb.has('brain:p:p:big'), false)
  assert.deepEqual(JSON.parse(ls.get('brain:p:p:big')).d, { items: [{ id: 1 }] })
})

test('quota error: the store keeps working in memory and the error is reported once', async () => {
  reports.length = 0
  lsFail = Object.assign(new Error('The quota has been exceeded.'), { name: 'QuotaExceededError' })
  const s = store({ n: 0 }, { persist: true }, 'p:quota')
  s.n = 1
  await wait()
  s.n = 2
  await wait()
  assert.equal(s.n, 2)
  assert.equal(reports.length, 1)
  assert.match(reports[0].message, /quota.*stays in memory/i)
  lsFail = null
})

test('private mode (storage access throws): no crash', () => {
  const real = globalThis.localStorage
  Object.defineProperty(globalThis, 'localStorage', { configurable: true, get: () => { throw new Error('SecurityError') } })
  const s = store({ n: 3 }, { persist: true }, 'p:private')
  assert.equal(s.n, 3)
  Object.defineProperty(globalThis, 'localStorage', { configurable: true, writable: true, value: real })
})
