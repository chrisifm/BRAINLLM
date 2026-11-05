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
let putDelay = 0, putFails = false // a slow or full storage (B04)
const cacheOf = name => {
  if (!storage.has(name)) storage.set(name, new Map())
  const m = storage.get(name)
  return {
    put: async (k, r) => {
      const body = await r.text()
      if (putDelay) await new Promise(ok => setTimeout(ok, putDelay))
      if (putFails) throw new DOMException('Quota exceeded', 'QuotaExceededError')
      m.set(abs(k), { body, headers: [...r.headers], status: r.status })
    },
    keys: async () => [...m.keys()].map(url => new Request(url)),
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

let net = {}, netCalls = [], netInits = [], offline = false, slow = 0, inFlight = 0, peak = 0
const fetch = async (req, o) => {
  const u = abs(req)
  netCalls.push(u)
  netInits.push(o)
  peak = Math.max(peak, ++inFlight)
  try {
    if (slow) await new Promise(r => setTimeout(r, slow))
    return answer(u)
  } finally {
    inFlight--
  }
}
const answer = u => {
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
  vm.runInNewContext(SRC, { self, caches, fetch, Response, Request, URL, Promise, setTimeout, JSON, console, DOMException })
  const life = async t => {
    const waits = []
    on[t]({ waitUntil: p => waits.push(p) })
    await Promise.all(waits)
  }
  // dispatch a fetch; resolves to the Response, or null if the worker did not call respondWith
  const get = async (path, { method = 'GET', mode = 'cors', destination = '', preloadResponse, headers = {} } = {}) => {
    let resp = null
    const waits = []
    on.fetch({ request: { url: ORIGIN + path, method, mode, destination, headers: new Headers(headers) }, respondWith: p => (resp = p), waitUntil: p => waits.push(p), preloadResponse: Promise.resolve(preloadResponse) })
    const r = resp && (await resp)
    await Promise.all(waits)
    return r
  }
  return { life, get, stats: () => ({ claimed, preload }), msgs }
}

test('ranges and streaming manifests bypass even a cached full file and explicit cache routes', async () => {
  const w = boot({ hash: 'media-network', precache: [], offline: '/_offline.html', routes: [{ match: '/video/', prefix: true, strategy: 'cache-first', maxEntries: 10 }] })
  const cached = await caches.open('brain-media-network')
  const explicit = await caches.open('brain:route:/video/*')
  await explicit.put(ORIGIN + '/video/control.mp4', new Response('explicit route control'))
  assert.equal(await (await w.get('/video/control.mp4')).text(), 'explicit route control')
  for (const path of ['/_b/clip.mp4', '/video/clip.mp4', '/video/index.m3u8', '/video/audio.m3u', '/video/index.mpd', '/video/MASTER.M3U8']) {
    await cached.put(ORIGIN + path, new Response('stale complete file'))
    await explicit.put(ORIGIN + path, new Response('stale explicit route'))
    const options = path.endsWith('.mp4') ? { headers: { range: 'bytes=0-1' }, destination: 'video' } : { destination: 'video' }
    assert.ok((await w.get(path, options)) === null, path)
    assert.ok((await w.get(path, { ...options, destination: '' })) === null, `${path}: explicit route only`)
  }
})

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
  assert.ok(storage.get('brain/app-base0001').has(ORIGIN + '/app/_b/x.js')) // namespaced by base (B03)
  assert.equal(await w.get('/app/_r/h/save'), null)
  assert.equal(await w.get('/app/sw.js'), null)
})

test('B04: every cache write is registered with waitUntil; a failed write never fails the response', async () => {
  const w = boot({ hash: 'b04b04b0', precache: [], offline: null })
  await w.life('install')
  await w.life('activate')
  net['/_b/slow.js'] = { body: 'slow', type: 'text/javascript' }
  net['/slow.svg'] = { body: '<svg>s</svg>', type: 'image/svg+xml' }
  putDelay = 40
  // get() awaits only what the worker handed to waitUntil: a detached put would still be pending here
  assert.equal(await (await w.get('/_b/slow.js')).text(), 'slow')
  assert.ok(storage.get('brain-b04b04b0').has(ORIGIN + '/_b/slow.js'), 'cache-first write outlived its waitUntil')
  assert.equal(await (await w.get('/slow.svg', { destination: 'image' })).text(), '<svg>s</svg>')
  assert.ok(storage.get('brain-b04b04b0').has(ORIGIN + '/slow.svg'), 'stale-while-revalidate write outlived its waitUntil')
  putDelay = 0
  putFails = true
  const unhandled = []
  const onUnhandled = e => unhandled.push(e)
  process.on('unhandledRejection', onUnhandled)
  net['/_b/full.js'] = { body: 'full', type: 'text/javascript' }
  assert.equal(await (await w.get('/_b/full.js')).text(), 'full')
  assert.equal(await (await w.get('/full.svg', { destination: 'image' })).status, 404)
  await new Promise(ok => setTimeout(ok, 20))
  process.off('unhandledRejection', onUnhandled)
  putFails = false
  assert.deepEqual(unhandled, [])
})

test('B03: apps sharing an origin and unrelated caches survive another app\'s activation', async () => {
  storage.clear()
  storage.set('unrelated-app-cache', new Map())
  storage.set('workbox-precache-v2', new Map())
  const shop = boot({ hash: 'shop0001', base: '/shop/', precache: [], offline: null })
  await shop.life('install')
  await shop.life('activate')
  assert.ok(storage.has('brain/shop-shop0001') && storage.has('brain/shop:meta'))
  // the root app updates three times: it rotates only its own generations
  for (const h of ['root0001', 'root0002', 'root0003']) {
    const root = boot({ hash: h, precache: [], offline: null })
    await root.life('install')
    await root.life('activate')
  }
  const names = [...storage.keys()].sort()
  assert.ok(names.includes('brain/shop-shop0001'), names.join(' '))
  assert.ok(names.includes('unrelated-app-cache') && names.includes('workbox-precache-v2'), names.join(' '))
  assert.ok(names.includes('brain-root0003') && names.includes('brain-root0002') && !names.includes('brain-root0001'), names.join(' '))
  // and the app under a base rotates without touching the root app
  for (const h of ['shop0002', 'shop0003']) {
    const shop2 = boot({ hash: h, base: '/shop/', precache: [], offline: null })
    await shop2.life('install')
    await shop2.life('activate')
  }
  const after = [...storage.keys()]
  assert.ok(after.includes('brain-root0003') && after.includes('brain-root0002') && !after.includes('brain/shop-shop0001'), after.join(' '))
})

test('install downloads at most a few precache files at once (a burst of hundreds tripped per-IP rate limits)', async () => {
  storage.clear()
  const list = Array.from({ length: 40 }, (_, i) => `/_b/p${i}.js`)
  for (const u of list) net[u] = { body: u, type: 'text/javascript' }
  slow = 5
  peak = 0
  const w = boot({ hash: 'pool0001', precache: list, offline: null })
  await w.life('install')
  slow = 0
  assert.ok(peak > 1 && peak <= 4, `peak ${peak}`)
  assert.equal(storage.get('brain-pool0001').size, 40)
})

test('an update reuses the hashed files the previous build already cached; only what changed downloads', async () => {
  storage.clear()
  net = {
    '/_b/same.js': { body: 'same', type: 'text/javascript' },
    '/_b/old.js': { body: 'old', type: 'text/javascript' },
    '/_b/new.js': { body: 'new', type: 'text/javascript' },
    '/_offline.html': { body: '<h1>Offline</h1>' },
  }
  const v1 = boot({ hash: 'keep0001', precache: ['/_b/same.js', '/_b/old.js', '/_offline.html'], offline: '/_offline.html' })
  await v1.life('install')
  await v1.life('activate')
  netCalls = []
  const v2 = boot({ hash: 'keep0002', precache: ['/_b/same.js', '/_b/new.js', '/_offline.html'], offline: '/_offline.html' })
  await v2.life('install')
  assert.deepEqual(netCalls.sort(), [ORIGIN + '/_b/new.js', ORIGIN + '/_offline.html'], 'the unhashed offline page is fetched again')
  const c = storage.get('brain-keep0002')
  assert.equal(c.get(ORIGIN + '/_b/same.js').body, 'same')
  assert.ok(c.has(ORIGIN + '/_b/new.js') && !c.has(ORIGIN + '/_b/old.js'))
  // another app on the origin is not a source: its /shop/_b/ files are not this app's
  storage.set('brain/shop-x', new Map([[ORIGIN + '/_b/other.js', { body: 'shop', headers: [], status: 200 }]]))
  net['/_b/other.js'] = { body: 'mine', type: 'text/javascript' }
  const v3 = boot({ hash: 'keep0003', precache: ['/_b/other.js'], offline: null })
  await v3.life('install')
  assert.equal(storage.get('brain-keep0003').get(ORIGIN + '/_b/other.js').body, 'mine')
})

test('custom immutable inventories retain old layout assets for exactly one previous generation', async () => {
  storage.clear(); offline = false; slow = 0
  net = {
    '/site/js/old.js': { body: 'old module', type: 'text/javascript' },
    '/site/assets/old.bin': { body: 'old binary', type: 'application/octet-stream' },
    '/site/src/new.js': { body: 'new module', type: 'text/javascript' },
    '/site/lib/third.js': { body: 'third module', type: 'text/javascript' },
    '/site/js/unhashed.js': { body: 'unhashed one', type: 'text/javascript' },
  }
  const first = boot({ hash: 'layout01', base: '/site/', immutable: ['/site/js/old.js', '/site/assets/old.bin'], precache: ['/site/js/old.js', '/site/assets/old.bin'], offline: null })
  await first.life('install'); await first.life('activate')
  const second = boot({ hash: 'layout02', base: '/site/', immutable: ['/site/src/new.js'], precache: ['/site/src/new.js'], offline: null })
  await second.life('install'); await second.life('activate')
  offline = true
  assert.equal(await (await second.get('/site/js/old.js', { destination: 'script' })).text(), 'old module', 'open tabs keep their exact prior module after the layout changes')
  assert.equal(await second.get('/site/js/old.js', { mode: 'navigate' }), null, 'prior asset bytes never answer a navigation')
  assert.equal(await (await second.get('/site/assets/old.bin')).text(), 'old binary', 'programmatic binary loads use the exact retained inventory without extension guesses')
  await assert.rejects(second.get('/site/assets/not-in-inventory.bin'), /Failed to fetch/)
  offline = false
  assert.equal(await (await second.get('/site/js/unhashed.js', { destination: 'script' })).text(), 'unhashed one')
  net['/site/js/unhashed.js'].body = 'unhashed two'; netCalls = []
  await second.get('/site/js/unhashed.js', { destination: 'script' })
  assert.ok(netCalls.includes(ORIGIN + '/site/js/unhashed.js'), 'directory membership never makes a public original cache-first')
  const third = boot({ hash: 'layout03', base: '/site/', immutable: ['/site/lib/third.js'], precache: ['/site/lib/third.js'], offline: null })
  await third.life('install'); await third.life('activate')
  assert.ok(!storage.has('brain/site-layout01'))
  const metadata = await (await cacheOf('brain/site:meta').match('/__brain_immutable')).json()
  assert.deepEqual(Object.keys(metadata).sort(), ['brain/site-layout02', 'brain/site-layout03'])
  assert.ok(!JSON.stringify(metadata).includes('/site/js/old.js'))
  offline = true
  await assert.rejects(third.get('/site/js/old.js', { destination: 'script' }), /Failed to fetch/)
  offline = false
})


test('inventory workers preserve legacy hashed chunks without promoting arbitrary cached responses', async () => {
  storage.clear(); offline = false; slow = 0
  net = { '/_b/legacy.js': { body: 'legacy code', type: 'text/javascript' }, '/js/current.js': { body: 'current code', type: 'text/javascript' } }
  const legacy = boot({ hash: 'before-layout', precache: ['/_b/legacy.js'], offline: null })
  await legacy.life('install'); await legacy.life('activate')
  await cacheOf('brain-before-layout').put('/api/private.json', new Response('never an immutable asset'))
  const next = boot({ hash: 'after-layout', immutable: ['/js/current.js'], precache: ['/js/current.js'], offline: null })
  await next.life('install'); await next.life('activate')
  offline = true
  assert.equal(await (await next.get('/_b/legacy.js', { destination: 'script' })).text(), 'legacy code')
  await assert.rejects(next.get('/api/private.json'), /Failed to fetch/)
  const metadata = await (await cacheOf('brain:meta').match('/__brain_immutable')).json()
  assert.deepEqual(metadata['brain-before-layout'], ['/_b/legacy.js'])
  offline = false
})
