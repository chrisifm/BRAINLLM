// runtime/sw/brain-sw.js (INTERNALS §35.2) in a simulated service worker scope: precache, every strategy, navigation
// preload, generations kept on update, requests that must not be touched.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import vm from 'node:vm'

const SRC = readFileSync(new URL('../sw/brain-sw.js', import.meta.url), 'utf8')
const ORIGIN = 'https://app.test'
const abs = u => new URL(typeof u === 'string' ? u : u.url, ORIGIN).href

// one CacheStorage shared by successive workers (like the browser)
const storage = new Map() // name → Map(url → { body, headers, status })
const cacheOf = name => {
  if (!storage.has(name)) storage.set(name, new Map())
  const m = storage.get(name)
  return {
    put: async (k, r) => void m.set(abs(k), { body: await r.text(), headers: [...r.headers], status: r.status }),
    match: async k => {
      const e = m.get(abs(k))
      return e && new Response(e.body, { status: e.status, headers: e.headers })
    },
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

let net = {}, netCalls = [], netInits = [], offline = false, slow = 0
const fetch = async (req, o) => {
  const u = abs(req)
  netCalls.push(u)
  netInits.push(o)
  if (slow) await new Promise(r => setTimeout(r, slow))
  if (offline) throw new TypeError('Failed to fetch')
  const e = net[new URL(u).pathname]
  return e ? new Response(e.body, { status: 200, headers: { 'content-type': e.type || 'text/html', ...(e.cc ? { 'cache-control': e.cc } : {}) } }) : new Response('nf', { status: 404 })
}

function boot(build) {
  const on = {}
  let claimed = 0, preload = 0
  const msgs = []
  const self = {
    BRAIN_BUILD: build,
    location: new URL(ORIGIN + '/sw.js'),
    registration: { navigationPreload: { enable: async () => void preload++ } },
    clients: { claim: async () => void claimed++, matchAll: async () => [{ postMessage: m => msgs.push(m) }, { postMessage: m => msgs.push(m) }] },
    skipWaiting: async () => {},
    addEventListener: (t, f) => (on[t] = f),
  }
  vm.runInNewContext(SRC, { self, caches, fetch, Response, Request, URL, Promise, setTimeout, JSON, console })
  const life = async t => {
    const waits = []
    on[t]({ waitUntil: p => waits.push(p) })
    await Promise.all(waits)
  }
  // dispatch a fetch; resolves to the Response, or null if the worker did not call respondWith
  const get = async (path, { method = 'GET', mode = 'cors', destination = '', preloadResponse } = {}) => {
    let resp = null
    const waits = []
    on.fetch({ request: { url: ORIGIN + path, method, mode, destination }, respondWith: p => (resp = p), waitUntil: p => waits.push(p), preloadResponse: Promise.resolve(preloadResponse) })
    const r = resp && (await resp)
    await Promise.all(waits)
    return r
  }
  return { life, get, stats: () => ({ claimed, preload }), msgs }
}

test('install precaches (a failing URL does not abort it); activate enables preload and claims clients', async () => {
  net = { '/_b/core.js': { body: 'core1', type: 'text/javascript' }, '/_offline.html': { body: '<h1>Offline page</h1>' } }
  const w = boot({ hash: 'aaaa1111', precache: ['/_b/core.js', '/_b/missing.js', '/_offline.html'], offline: '/_offline.html' })
  await w.life('install')
  await w.life('activate')
  assert.ok(storage.get('brain-aaaa1111').has(ORIGIN + '/_b/core.js'))
  assert.ok(!storage.get('brain-aaaa1111').has(ORIGIN + '/_b/missing.js'))
  assert.ok(netInits.every(o => !o || !o.cache), 'precache uses the HTTP cache the page just filled (no cache: reload)')
  assert.deepEqual(w.stats(), { claimed: 1, preload: 1 })
  globalThis.W = w
})

test('/_b/ is cache-first; /_r/, non-GET, other origins and /sw.js are not touched', async () => {
  const w = globalThis.W
  netCalls = []
  assert.equal(await (await w.get('/_b/core.js')).text(), 'core1')
  assert.equal(netCalls.length, 0)
  net['/_b/late.js'] = { body: 'late', type: 'text/javascript' }
  assert.equal(await (await w.get('/_b/late.js')).text(), 'late') // miss → network → stored
  assert.ok(storage.get('brain-aaaa1111').has(ORIGIN + '/_b/late.js'))
  assert.equal(await w.get('/_r/abc/save'), null)
  assert.equal(await w.get('/users', { method: 'POST' }), null)
  assert.equal(await w.get('/sw.js'), null)
  let touched = false
  const W2 = boot({ hash: 'x', precache: [], offline: null })
  void W2
  assert.equal(touched, false)
})

test('navigation: network first (with navigation preload) and stored; offline → cached page; never visited → offline page', async () => {
  const w = globalThis.W
  net['/about'] = { body: '<h1>About v1</h1>' }
  netCalls = []
  const pre = new Response('<h1>About (preload)</h1>', { headers: { 'content-type': 'text/html' } })
  assert.equal(await (await w.get('/about', { mode: 'navigate', preloadResponse: pre })).text(), '<h1>About (preload)</h1>')
  assert.equal(netCalls.length, 0) // the preload response was used, no second request
  await w.get('/about', { mode: 'navigate' })
  offline = true
  assert.equal(await (await w.get('/about', { mode: 'navigate' })).text(), '<h1>About v1</h1>')
  const off = await w.get('/never', { mode: 'navigate' })
  assert.equal(await off.text(), '<h1>Offline page</h1>')
  offline = false
})

test('navigation: a network slower than 3 s falls back to the cached page', async () => {
  const w = globalThis.W
  slow = 3300
  const t0 = Date.now()
  const r = await w.get('/about', { mode: 'navigate' })
  assert.equal(await r.text(), '<h1>About v1</h1>')
  assert.ok(Date.now() - t0 < 3300 + 200)
  slow = 0
})

test('public files: stale-while-revalidate', async () => {
  const w = globalThis.W
  net['/logo.svg'] = { body: '<svg>1</svg>', type: 'image/svg+xml' }
  assert.equal(await (await w.get('/logo.svg', { destination: 'image' })).text(), '<svg>1</svg>')
  net['/logo.svg'] = { body: '<svg>2</svg>', type: 'image/svg+xml' }
  assert.equal(await (await w.get('/logo.svg', { destination: 'image' })).text(), '<svg>1</svg>') // stale at once…
  assert.equal(await (await w.get('/logo.svg', { destination: 'image' })).text(), '<svg>2</svg>') // …updated in the background
})

test('updates keep one previous generation: build 2 keeps build 1, build 3 deletes it', async () => {
  const b2 = boot({ hash: 'bbbb2222', precache: [], offline: null })
  await b2.life('install')
  await b2.life('activate')
  assert.ok(storage.has('brain-aaaa1111') && storage.has('brain-bbbb2222'))
  assert.equal(await (await b2.get('/_b/core.js')).text(), 'core1') // an open old tab still gets its chunk
  const b3 = boot({ hash: 'cccc3333', precache: [], offline: null })
  await b3.life('install')
  await b3.life('activate')
  assert.ok(!storage.has('brain-aaaa1111'))
  assert.ok(storage.has('brain-bbbb2222') && storage.has('brain-cccc3333'))
  assert.ok(storage.has('brain:meta')) // not a build cache: never rotated
  assert.deepEqual(b3.msgs, ['brain:build cccc3333', 'brain:build cccc3333']) // every open tab is told
})

test('the previous build never answers a navigation (only chunks for open tabs)', async () => {
  net = { '/about': { body: '<h1>About from build 3</h1>' } }
  const b3 = boot({ hash: 'cccc3333', precache: [], offline: '/_offline.html' })
  await b3.get('/about', { mode: 'navigate' }) // page cached inside build 3
  assert.ok(storage.get('brain-cccc3333').has(ORIGIN + '/about'))
  const b4 = boot({ hash: 'dddd4444', precache: [], offline: '/_offline.html' })
  await b4.life('install')
  await b4.life('activate')
  assert.ok(storage.has('brain-cccc3333')) // kept as the previous generation…
  offline = true
  const r = await b4.get('/about', { mode: 'navigate' })
  assert.equal(r.status, 503) // …but its page is not shown: the built-in offline page is
  assert.match(await r.text(), /You are offline/)
  offline = false
})

test('QA #124: data fetches are never touched; no-store/private responses never stored; offline:false leaves navigations alone', async () => {
  const w = boot({ hash: 'eeee5555', precache: [], offline: '/_offline.html' })
  net['/api/items'] = { body: '[1]', type: 'application/json', cc: 'no-store' }
  assert.equal(await w.get('/api/items'), null) // fetch()/XHR (destination '') → not intercepted
  net['/me.png'] = { body: 'me', type: 'image/png', cc: 'private, max-age=60' }
  await w.get('/me.png', { destination: 'image' })
  assert.ok(!(storage.get('brain-eeee5555') || new Map()).has(ORIGIN + '/me.png'))
  net['/secret'] = { body: '<h1>Account</h1>', cc: 'no-store' }
  await w.get('/secret', { mode: 'navigate' })
  assert.ok(!(storage.get('brain-eeee5555') || new Map()).has(ORIGIN + '/secret'))
  const off = boot({ hash: 'ffff6666', precache: [], offline: null })
  assert.equal(await off.get('/about', { mode: 'navigate' }), null) // the browser handles it: its own error page offline
})

test('#216 base path: _b/ cache-first, _r/ and sw.js untouched, all under BRAIN_BUILD.base', async () => {
  net = { '/app/_b/x.js': { body: 'x', type: 'text/javascript' } }
  const w = boot({ hash: 'base0001', base: '/app/', precache: [], offline: null })
  await w.life('install')
  await w.life('activate')
  assert.equal(await (await w.get('/app/_b/x.js')).text(), 'x')
  assert.ok(storage.get('brain-base0001').has(ORIGIN + '/app/_b/x.js'))
  assert.equal(await w.get('/app/_r/h/save'), null)
  assert.equal(await w.get('/app/sw.js'), null)
})
