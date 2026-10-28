// api() + resource() (INTERNALS §15, §31, §35.3): client, validation, memory SWR, Cache API opt-in, races, timeout.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { install } from '../shim/dom.js'

install()
globalThis.location = new URL('https://app.test/')
const rt = await import('./rt.js')
const { sig, fx } = rt
const { api, resource, http } = await rt.sibling('brain-data.js')
const tick = (ms = 0) => new Promise(r => setTimeout(r, ms))

// fake server
let db = { 1: { id: 1, name: 'Ada' }, 2: { id: 2, name: 'Linus' } }, calls = [], delay = 0, fail = false
globalThis.fetch = async (u, init) => {
  calls.push({ u, init })
  await new Promise((ok, no) => {
    const t = setTimeout(ok, delay)
    init.signal && init.signal.addEventListener('abort', () => (clearTimeout(t), no(init.signal.reason)))
  })
  if (fail) throw new TypeError('fetch failed')
  const url = new URL(u, 'https://app.test'), m = url.pathname.match(/^\/users\/(\d+)$/)
  const res = (status, body) => ({ ok: status < 400, status, text: async () => body })
  if (init.method === 'GET' && url.pathname === '/users') return res(200, JSON.stringify(Object.values(db)))
  if (init.method === 'GET' && m) return db[m[1]] ? res(200, JSON.stringify(db[m[1]])) : res(404, '{"error":"Not found"}')
  if (init.method === 'POST' && url.pathname === '/users') {
    const b = JSON.parse(init.body)
    if (b.name === 'taken') return res(422, '{"error":"Invalid input","errors":{"name":"Taken"}}')
    const id = Object.keys(db).length + 1
    db[id] = { id, ...b }
    return res(200, JSON.stringify(db[id]))
  }
  if (url.pathname === '/evil') return res(200, '{"name":"x","__proto__":{"admin":true},"nested":{"constructor":{"p":1}}}')
  if (url.pathname === '/bad') return res(200, '{"id":"seven"}')
  return res(404, '')
}
// fake Cache API
const store = new Map()
globalThis.caches = { delete: async () => (store.clear(), true), open: async () => ({
  put: async (u, r) => store.set(u, { body: await r.text(), h: r.headers }),
  match: async u => store.has(u) && { headers: store.get(u).h, text: async () => store.get(u).body },
  keys: async () => [...store.keys()].map(url => ({ url })),
  delete: async key => store.delete(typeof key === 'string' ? key : key.url),
}) }

const User = { id: 'num!', name: 'str!' }
const users = api('/users', { get: { out: [User] }, post: { in: { name: 'str!' }, out: User } })
const one = api('/users/:id', { get: { out: User } })

test('api: path params, query, JSON body, input validation, server field errors, prototype keys dropped', async () => {
  calls = []
  assert.deepEqual(await one.get({ id: 2, fields: 'all' }), { id: 2, name: 'Linus' })
  assert.equal(calls[0].u, '/users/2?fields=all')
  assert.equal(calls[0].init.credentials, 'same-origin')
  const created = await users.post({ name: 'Grace' })
  assert.deepEqual(created, { id: 3, name: 'Grace' })
  assert.equal(calls[1].init.body, '{"name":"Grace"}')
  await assert.rejects(users.post({}), e => /invalid input: name: Required/.test(e.message)) // never sent
  assert.equal(calls.length, 2)
  await assert.rejects(users.post({ name: 'taken' }), e => e.status === 422 && e.errors.name === 'Taken')
  const evil = await api('/evil', { get: {} }).get()
  assert.equal({}.admin, undefined)
  assert.equal(Object.getPrototypeOf(evil), Object.prototype)
  assert.equal(Object.hasOwn(evil.nested, 'constructor'), false)
})

test('api: a response that breaks its schema is reported with the diff and rejects', async () => {
  const seen = []
  globalThis.reportError = e => seen.push(e)
  await assert.rejects(api('/bad', { get: { out: User } }).get(), /does not match its schema: id: Enter a number/)
  assert.equal(seen.length, 1)
  delete globalThis.reportError
})

// a stand-in for the compiled {#await x} block: records what each promise of the node resolves to
const watch = node => {
  const seen = []
  fx(() => {
    const p = node.v
    Promise.resolve(p).then(v => seen.push(v), e => seen.push('error: ' + e.message))
  })
  return seen
}

test('resource: memory stale-while-revalidate per URL (instant second visit), replaced only when data changed', async () => {
  delay = 20
  const a = watch(resource(users.get))
  await tick(40)
  assert.equal(a.length, 1)
  const b = watch(resource(users.get)) // back/forward: cached at once
  await tick(0)
  assert.equal(b.length, 1)
  assert.equal(b[0].length, 3)
  await tick(40)
  assert.equal(b.length, 1) // network said the same: no re-render
  db[4] = { id: 4, name: 'Barbara' }
  const c = watch(resource(users.get))
  await tick(0)
  assert.equal(c[0].length, 3) // stale first…
  await tick(40)
  assert.equal(c[1].length, 4) // …then the fresh list
  delay = 0
})

test('resource: a write drops the cached GETs; reactive params refetch and the stale request is aborted', async () => {
  await users.post({ name: 'Alan' })
  calls = []
  const d = watch(resource(users.get))
  await tick(5) // the request goes through the queue: a few microtasks more than one timer
  assert.equal(d.length, 1)
  assert.equal(d[0].length, 5) // no stale copy: it was invalidated
  delay = 30
  const id = sig(1)
  const r = watch(resource(() => one.get({ id: id.v })))
  await tick(5)
  id.v = 2
  await tick(60)
  const aborted = calls.filter(c => c.u === '/users/1')[0]
  assert.equal(aborted.init.signal.aborted, true)
  assert.deepEqual(r.at(-1), { id: 2, name: 'Linus' })
  assert.ok(!r.some(v => v && v.id === 1)) // the old answer never landed
  delay = 0
})

test('resource: network error keeps the cached answer; no cache → error branch', async () => {
  fail = true
  http({ retry: 0 }) // retries are covered by the #168 tests
  const e = watch(resource(users.get))
  await tick(5)
  assert.equal(e.length, 1)
  assert.equal(e[0].length, 5)
  const f = watch(resource(() => one.get({ id: 99 })))
  await tick(5)
  assert.match(f[0], /^error: Connection lost\./) // ApiError kind 'network'
  await tick(30) // let every request started while offline settle: a later test must not join one in flight (dedupe)
  fail = false
  http({ retry: 2 })
})

test("cache: '10m' keeps the GET in the Cache API for the next visit (served first, then revalidated)", async () => {
  const posts = api('/users/:id', { get: { out: User, cache: '10m' } })
  await posts.get({ id: 1 })
  await tick(5) // the Cache API write is fire-and-forget
  assert.ok(store.has('https://app.test/users/1'))
  // next visit: memory is gone, Cache API is not
  const fresh = await import(rt.base + 'brain-data.js?visit2')
  delay = 30
  db[1] = { id: 1, name: 'Ada L.' }
  const p = fresh.api('/users/:id', { get: { out: User, cache: '10m' } })
  const g = watch(fresh.resource(() => p.get({ id: 1 })))
  await tick(10)
  assert.deepEqual(g[0], { id: 1, name: 'Ada' }) // from the Cache API before the network answers
  await tick(50)
  assert.deepEqual(g.at(-1), { id: 1, name: 'Ada L.' })
  delay = 0
  const noCache = api('/users/:id', { get: { out: User } })
  store.clear()
  await noCache.get({ id: 2 })
  await tick(5)
  assert.equal(store.size, 0) // never persisted without the opt-in
})

test("QA #124: cache.data:false (meta brain-data-cache=off) → no memory SWR and no Cache API", async () => {
  const meta = document.createElement('meta')
  meta.setAttribute('name', 'brain-data-cache')
  meta.setAttribute('content', 'off')
  document.body.appendChild(meta)
  store.clear()
  const p = api('/users/:id', { get: { out: User, cache: '10m' } })
  await p.get({ id: 1 })
  await tick(5)
  assert.equal(store.size, 0)
  const again = p.get({ id: 1 })
  assert.equal(again.$stale, undefined)
  assert.equal(again.$stored, undefined)
  await again
  meta.remove()
})

test('persistent cache bounds entries, rejects corrupt timestamps and invalidates after writes', async () => {
  store.clear()
  const fresh = await import(rt.base + 'brain-data.js?cache-bounds')
  const endpoint = fresh.api('/users/:id', { get: { cache: '10m' } })
  for (let page = 0; page < 130; page++) await endpoint.get({ id: 1, page })
  await tick(10)
  assert.equal(store.size, 128)
  assert.equal(store.has('https://app.test/users/1?page=0'), false)
  store.set('https://app.test/users/1?probe=invalid', { body: '{"private":"stale"}', h: new Headers({ 'x-brain-t': 'invalid' }) })
  const call = endpoint.get({ id: 1, probe: 'invalid' })
  assert.equal(await call.$stored, undefined)
  await call
  await fresh.api('/users', { post: {} }).post({ name: 'Cache test' })
  await tick(10)
  assert.equal(store.size, 0)
})

test('#216 api() paths are app paths: under a base path the base is prepended', async () => {
  const m = document.createElement('meta')
  m.setAttribute('name', 'brain-base')
  m.setAttribute('content', '/app/')
  document.body.appendChild(m)
  calls = []
  await one.get({ id: 2 }).catch(() => {}) // the fake server only knows app paths: we check the URL asked for
  assert.equal(calls[0].u, '/app/users/2')
  m.remove()
})
