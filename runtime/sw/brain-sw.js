// BRAINLLM service worker (INTERNALS §35.2). `brain build` emits dist/sw.js as one line
// `self.BRAIN_BUILD = { hash, precache: [...], offline: '/_offline.html' | null }` followed by this file unchanged.
// Strategies: /_b/* cache-first (hashed, immutable) · navigations network-first with a 3 s timeout, then the cached
// page, then the offline page · other same-origin GETs (unhashed public/ files) stale-while-revalidate · /_r/, non-GET
// and other origins are never touched. Every build has one cache, `brain-<sig>`, holding its chunks, pages and public
// files (§35.7). A new worker takes over at once and keeps only the previous build's cache, and only for the chunks an
// open tab still asks for: navigations are answered from the current build alone, so nothing old is shown by mistake.
// After taking over it tells every tab `brain:build <sig>` (the router then does full loads). Caches that are not per
// build are named `brain:*`. (`sw: false` is a separate kill worker emitted by the build.)
// `cache.routes` (brain.json) adds declarative data routes: { match, prefix, strategy: cache-first | network-first |
// stale-while-revalidate | network-only, maxEntries, maxAge (s), timeout (ms) }, each in its own `brain:route:<match>`
// cache that survives builds, bounded by entries and age; personal responses are never stored. `cache.update:
// "prompt"` keeps a new worker waiting (while an older one controls tabs) until a page posts `brain:skip-waiting`.
const B = self.BRAIN_BUILD || { hash: 'dev', precache: [], offline: null }
const BASE = B.base || '/' // app served under a base path (§23 #148); precache/offline URLs already carry it
const BUILD = 'brain-' + B.hash, META = 'brain:meta'
const NAV_TIMEOUT = 3000
// responses marked personal or uncacheable are never stored
const storable = r => r.ok && !/no-store|private/i.test(r.headers.get('cache-control') || '')
const ROUTES = (B.routes || []).map(r => ({ ...r, cache: 'brain:route:' + r.match + (r.prefix ? '*' : '') }))
const route = p => ROUTES.find(r => (r.prefix ? p.startsWith(r.match) : p === r.match))
// identity-bound: an Authorization header, a response that varies by cookie/authorization/*, or one that sets a cookie
const personal = (req, r) =>
  !!req.headers?.has?.('authorization') || /(^|,)\s*(\*|cookie|authorization)\s*(,|$)/i.test(r.headers.get('vary') || '') || r.headers.has('set-cookie')
const OFFLINE =
  '<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">' +
  '<title>Offline</title><main style="font:16px/1.5 system-ui;max-width:32rem;margin:15vh auto;padding:0 16px">' +
  '<h1>You are offline</h1><p>This page has not been visited yet on this device. Reconnect and try again.</p></main></html>'

self.addEventListener('install', e => {
  e.waitUntil(
    (async () => {
      const c = await caches.open(BUILD)
      // one failing URL must not abort the install: cache what answers, the rest is cached on first use
      // default cache mode: the page just filled the HTTP cache with these immutable files, so this costs no bytes
      await Promise.allSettled(B.precache.map(u => fetch(u).then(r => r.ok && c.put(u, r))))
      // update: 'prompt' → an update waits for the page (appUpdate); the first install has nothing to replace
      if (B.update !== 'prompt' || !self.registration.active) await self.skipWaiting()
    })()
  )
})

self.addEventListener('activate', e => {
  e.waitUntil(
    (async () => {
      if (self.registration.navigationPreload) await self.registration.navigationPreload.enable()
      // generations: [previous, current]; every other build cache is deleted
      const meta = await caches.open(META)
      const r = await meta.match('/__brain_generations')
      const gens = r ? await r.json() : []
      const keep = [...gens.filter(g => g !== BUILD).slice(-1), BUILD]
      await meta.put('/__brain_generations', new Response(JSON.stringify(keep)))
      for (const k of await caches.keys())
        if ((k.startsWith('brain-') && !keep.includes(k)) || (k.startsWith('brain:route:') && !ROUTES.some(r => r.cache === k))) await caches.delete(k)
      await self.clients.claim()
      for (const c of await self.clients.matchAll({ type: 'window', includeUncontrolled: true })) c.postMessage('brain:build ' + B.hash)
    })()
  )
})

self.addEventListener('message', e => {
  if (e.data === 'brain:skip-waiting') self.skipWaiting()
})

self.addEventListener('fetch', e => {
  const req = e.request
  if (req.method !== 'GET') return
  const url = new URL(req.url)
  if (url.origin !== self.location.origin || url.pathname.startsWith(BASE + '_r/') || url.pathname === BASE + 'sw.js') return
  // offline: false (B.offline null) → navigations go straight to the network: no page cache, no offline page
  if (req.mode === 'navigate') return B.offline ? e.respondWith(navigate(e)) : undefined
  if (url.pathname.startsWith(BASE + '_b/')) return e.respondWith(hashed(req))
  const rt = route(url.pathname)
  if (rt) return e.respondWith(rt.strategy === 'network-only' ? fetch(req) : data(e, rt))
  // only files the page itself loads (img, style, script, font, manifest…); fetch()/XHR data is never touched (§35:
  // data is not cached by default)
  if (req.destination) e.respondWith(revalidate(e))
})

// cache-first: any generation may hold it (an old tab asks for old chunks); misses are stored in the current build
async function hashed(req) {
  const hit = await caches.match(req)
  if (hit) return hit
  const r = await fetch(req)
  if (storable(r)) (await caches.open(BUILD)).put(req, r.clone())
  return r
}

async function navigate(e) {
  const net = (async () => {
    const r = (await e.preloadResponse) || (await fetch(e.request))
    if (storable(r) && (r.headers.get('content-type') || '').includes('text/html')) {
      const copy = r.clone()
      e.waitUntil(caches.open(BUILD).then(c => c.put(e.request, copy)))
    }
    return r
  })()
  e.waitUntil(net.catch(() => {}))
  const timeout = new Promise(ok => setTimeout(() => ok(null), NAV_TIMEOUT))
  const first = await Promise.race([net, timeout]).catch(() => null)
  if (first) return first
  const cached = await caches.match(e.request, { cacheName: BUILD }) // never a page of the previous build
  if (cached) return cached
  // nothing cached: a slow network is still better than the offline page
  try {
    return await net
  } catch {
    const off = B.offline && (await caches.match(B.offline, { cacheName: BUILD }))
    return off || new Response(OFFLINE, { status: 503, headers: { 'content-type': 'text/html; charset=utf-8' } })
  }
}

// stale-while-revalidate for unhashed same-origin files
async function revalidate(e) {
  const c = await caches.open(BUILD)
  const hit = await c.match(e.request)
  const net = fetch(e.request).then(r => (storable(r) && c.put(e.request, r.clone()), r))
  e.waitUntil(net.catch(() => {}))
  return hit || net
}

// declared data route: entries carry their storage time (x-brain-at); older than maxAge are never served
async function data(e, rt) {
  const req = e.request, c = await caches.open(rt.cache)
  const fresh = async () => {
    const h = await c.match(req)
    return h && (!rt.maxAge || Date.now() - Number(h.headers.get('x-brain-at') || 0) <= rt.maxAge * 1000) ? h : null
  }
  const save = async r => {
    if (!storable(r) || r.status !== 200 || personal(req, r)) return
    const h = new Headers(r.headers)
    h.set('x-brain-at', String(Date.now()))
    await c.put(req, new Response(await r.blob(), { status: 200, statusText: r.statusText, headers: h }))
    const keys = await c.keys() // insertion order: the oldest first
    for (const k of keys.slice(0, Math.max(0, keys.length - rt.maxEntries))) await c.delete(k)
  }
  const net = () => {
    const p = fetch(req).then(r => (e.waitUntil(save(r.clone()).catch(() => {})), r))
    e.waitUntil(p.catch(() => {}))
    return p
  }
  if (rt.strategy === 'cache-first') return (await fresh()) || net()
  if (rt.strategy === 'network-first') {
    const n = net()
    const timeout = new Promise(ok => setTimeout(() => ok(null), rt.timeout || NAV_TIMEOUT))
    const first = await Promise.race([n, timeout]).catch(() => null)
    return first || (await fresh()) || n
  }
  const hit = await fresh() // stale-while-revalidate
  const n = net()
  return hit || n
}
