// runtime/sw/brain-sw.js: declarative data routes (`cache.routes`) and the user-controlled update flow
// (`cache.update: "prompt"`), in a simulated service worker scope. No wall-clock assertions: ages use a fake clock.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import vm from 'node:vm'

const SRC = readFileSync(new URL('../sw/brain-sw.js', import.meta.url), 'utf8')
const ORIGIN = 'https://shop.test'
const abs = u => new URL(typeof u === 'string' ? u : u.url, ORIGIN).href

function world() {
  const storage = new Map()
  const cacheOf = name => {
    if (!storage.has(name)) storage.set(name, new Map())
    const m = storage.get(name)
    return {
      put: async (k, r) => void (m.delete(abs(k)), m.set(abs(k), { body: await r.text(), headers: [...r.headers], status: r.status })),
      match: async k => {
        const e = m.get(abs(k))
        return e && new Response(e.body, { status: e.status, headers: e.headers })
      },
      keys: async () => [...m.keys()].map(url => ({ url })),
      delete: async k => m.delete(abs(k)),
    }
  }
  const caches = {
    open: async n => cacheOf(n),
    keys: async () => [...storage.keys()],
    delete: async n => storage.delete(n),
    match: async (k, o = {}) => {
      for (const n of o.cacheName ? [o.cacheName] : [...storage.keys()]) {
        if (!storage.has(n)) continue
        const r = await cacheOf(n).match(k)
        if (r) return r
      }
    },
  }
  const W = { net: {}, calls: [], offline: false, hang: false, now: 1_000_000, storage, caches }
  W.fetch = async req => {
    const u = abs(req)
    W.calls.push(new URL(u).pathname + new URL(u).search)
    if (W.hang) return new Promise(() => {})
    if (W.offline) throw new TypeError('Failed to fetch')
    const e = W.net[new URL(u).pathname]
    if (!e) return new Response('nf', { status: 404 })
    return new Response(e.body, { status: e.status || 200, headers: { 'content-type': 'application/json', ...(e.headers || {}) } })
  }
  return W
}

function boot(W, build, { active = null } = {}) {
  const on = {}
  let skipped = 0
  const timers = []
  const self = {
    BRAIN_BUILD: build,
    location: new URL(ORIGIN + '/sw.js'),
    registration: { active, navigationPreload: null },
    clients: { claim: async () => {}, matchAll: async () => [] },
    skipWaiting: async () => void skipped++,
    addEventListener: (t, f) => (on[t] = f),
  }
  // timers run only when the test says so (deterministic network-first timeout)
  const setTimeout = (f, ms) => (timers.push({ f, ms }), timers.length)
  const DateShim = { now: () => W.now }
  vm.runInNewContext(SRC, { self, caches: W.caches, fetch: W.fetch, Response, Request, Headers, URL, Promise, setTimeout, Date: DateShim, JSON, console, String, Number, Math })
  const life = async t => {
    const waits = []
    on[t]({ waitUntil: p => waits.push(p) })
    await Promise.all(waits)
  }
  const get = async (path, { headers = {}, tick = false } = {}) => {
    let resp = null
    const waits = []
    on.fetch({ request: { url: ORIGIN + path, method: 'GET', mode: 'cors', destination: '', headers: new Headers(headers) }, respondWith: p => (resp = p), waitUntil: p => waits.push(p) })
    if (tick) {
      await new Promise(r => setImmediate(r))
      for (const t of timers.splice(0)) t.f()
    }
    const r = resp && (await resp)
    if (!tick) await Promise.all(waits) // a hung network never settles its background copy
    return r
  }
  return { on, life, get, skipped: () => skipped, timers }
}

const routes = [
  { match: '/api/products/', prefix: true, strategy: 'stale-while-revalidate', maxEntries: 2, maxAge: 60 },
  { match: '/api/cart', prefix: false, strategy: 'network-first', maxEntries: 10, maxAge: 0, timeout: 3000 },
  { match: '/api/catalog', prefix: false, strategy: 'cache-first', maxEntries: 10, maxAge: 60 },
  { match: '/api/session', prefix: false, strategy: 'network-only', maxEntries: 0, maxAge: 0 },
]
const ROUTE = m => 'brain:route:' + m

test('routes: stale-while-revalidate answers from cache and refreshes in the background', async () => {
  const W = world()
  const w = boot(W, { hash: 'r1', precache: [], offline: null, routes })
  await w.life('install')
  await w.life('activate')
  W.net['/api/products/1'] = { body: '{"v":1}' }
  assert.equal(await (await w.get('/api/products/1')).text(), '{"v":1}')
  W.net['/api/products/1'] = { body: '{"v":2}' }
  assert.equal(await (await w.get('/api/products/1')).text(), '{"v":1}')
  assert.equal(await (await w.get('/api/products/1')).text(), '{"v":2}')
})

test('routes: maxEntries evicts the oldest entries and maxAge expires stale ones', async () => {
  const W = world()
  const w = boot(W, { hash: 'r2', precache: [], offline: null, routes })
  for (const id of [1, 2, 3]) {
    W.net['/api/products/' + id] = { body: String(id) }
    await w.get('/api/products/' + id)
  }
  const kept = [...W.storage.get(ROUTE('/api/products/*')).keys()].map(u => new URL(u).pathname)
  assert.deepEqual(kept, ['/api/products/2', '/api/products/3'])
  W.offline = true
  W.now += 61_000 // older than maxAge 60 s: never served
  await assert.rejects(w.get('/api/products/3'))
})

test('routes: network-first falls back to the cache offline and after its timeout', async () => {
  const W = world()
  const w = boot(W, { hash: 'r3', precache: [], offline: null, routes })
  W.net['/api/cart'] = { body: '{"items":1}' }
  assert.equal(await (await w.get('/api/cart')).text(), '{"items":1}')
  W.offline = true
  assert.equal(await (await w.get('/api/cart')).text(), '{"items":1}')
  W.offline = false
  W.hang = true
  assert.equal(await (await w.get('/api/cart', { tick: true })).text(), '{"items":1}')
  assert.equal(w.timers.length, 0)
})

test('routes: cache-first serves the stored copy without the network', async () => {
  const W = world()
  const w = boot(W, { hash: 'r4', precache: [], offline: null, routes })
  W.net['/api/catalog'] = { body: '["a"]' }
  await w.get('/api/catalog')
  W.calls = []
  assert.equal(await (await w.get('/api/catalog')).text(), '["a"]')
  assert.deepEqual(W.calls, [])
})

test('routes: network-only and private responses are never stored', async () => {
  const W = world()
  const w = boot(W, { hash: 'r5', precache: [], offline: null, routes })
  W.net['/api/session'] = { body: '{"user":1}' }
  assert.equal(await (await w.get('/api/session')).text(), '{"user":1}')
  assert.ok(!W.storage.has(ROUTE('/api/session')))
  W.net['/api/products/9'] = { body: 'mine', headers: { vary: 'Accept, Cookie' } }
  await w.get('/api/products/9')
  W.net['/api/products/8'] = { body: 'auth' }
  await w.get('/api/products/8', { headers: { authorization: 'Bearer x' } })
  W.net['/api/products/7'] = { body: 'p', headers: { 'cache-control': 'private, max-age=60' } }
  await w.get('/api/products/7')
  W.net['/api/products/6'] = { body: 'partial', status: 206 }
  await w.get('/api/products/6')
  assert.equal((W.storage.get(ROUTE('/api/products/*')) || new Map()).size, 0)
  assert.equal(await w.get('/api/other'), null) // no route: data stays untouched
})

test('routes: activate deletes route caches that are no longer declared', async () => {
  const W = world()
  W.storage.set(ROUTE('/api/old*'), new Map())
  const w = boot(W, { hash: 'r6', precache: [], offline: null, routes })
  await w.life('activate')
  assert.ok(!W.storage.has(ROUTE('/api/old*')))
})

test('update "prompt": the first install activates, a later one waits for brain:skip-waiting', async () => {
  const W = world()
  const first = boot(W, { hash: 'u1', precache: [], offline: null, update: 'prompt' })
  await first.life('install')
  assert.equal(first.skipped(), 1)
  const next = boot(W, { hash: 'u2', precache: [], offline: null, update: 'prompt' }, { active: {} })
  await next.life('install')
  assert.equal(next.skipped(), 0)
  next.on.message({ data: 'something else' })
  assert.equal(next.skipped(), 0)
  next.on.message({ data: 'brain:skip-waiting' })
  assert.equal(next.skipped(), 1)
  const auto = boot(W, { hash: 'u3', precache: [], offline: null }, { active: {} })
  await auto.life('install')
  assert.equal(auto.skipped(), 1)
})
