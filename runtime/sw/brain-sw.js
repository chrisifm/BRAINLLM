// BRAINLLM service worker (INTERNALS §35.2). `brain build` emits dist/sw.js as one line
// `self.BRAIN_BUILD = { hash, precache: [...], offline: '/_offline.html' | null }` followed by this file unchanged.
// Strategies: /_b/* cache-first (hashed, immutable) · navigations network-first with a 3 s timeout, then the cached
// page, then the offline page · other same-origin GETs (unhashed public/ files) stale-while-revalidate · /_r/, non-GET
// and other origins are never touched. Every build has one cache, `brain-<sig>`, holding its chunks, pages and public
// files (§35.7). A new worker takes over at once and keeps only the previous build's cache, and only for the chunks an
// open tab still asks for: navigations are answered from the current build alone, so nothing old is shown by mistake.
// After taking over it tells every tab `brain:build <sig>` (the router then does full loads). Caches that are not per
// build are named `brain:*`. (`sw: false` is a separate kill worker emitted by the build.)
const B = self.BRAIN_BUILD || { hash: 'dev', precache: [], offline: null }
const BASE = B.base || '/' // app served under a base path (§23 #148); precache/offline URLs already carry it
const BUILD = 'brain-' + B.hash, META = 'brain:meta'
const NAV_TIMEOUT = 3000
// responses marked personal or uncacheable are never stored
const storable = r => r.ok && !/no-store|private/i.test(r.headers.get('cache-control') || '')
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
      await self.skipWaiting()
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
      for (const k of await caches.keys()) if (k.startsWith('brain-') && !keep.includes(k)) await caches.delete(k)
      await self.clients.claim()
      for (const c of await self.clients.matchAll({ type: 'window', includeUncontrolled: true })) c.postMessage('brain:build ' + B.hash)
    })()
  )
})

self.addEventListener('fetch', e => {
  const req = e.request
  if (req.method !== 'GET') return
  const url = new URL(req.url)
  if (url.origin !== self.location.origin || url.pathname.startsWith(BASE + '_r/') || url.pathname === BASE + 'sw.js') return
  // offline: false (B.offline null) → navigations go straight to the network: no page cache, no offline page
  if (req.mode === 'navigate') return B.offline ? e.respondWith(navigate(e)) : undefined
  if (url.pathname.startsWith(BASE + '_b/')) return e.respondWith(hashed(req))
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
