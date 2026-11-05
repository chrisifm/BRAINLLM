// BRAINLLM service worker (INTERNALS §35.2). `brain build` emits dist/sw.js as one line
// `self.BRAIN_BUILD = { hash, precache: [...], offline: '/_offline.html' | null }` followed by this file unchanged.
// Strategies: /_b/* cache-first (hashed, immutable) · navigations network-first with a 3 s timeout, then the cached
// page, then the offline page · other same-origin GETs (unhashed public/ files) stale-while-revalidate · /_r/, non-GET
// and other origins are never touched. Every build has one cache, `brain-<sig>`, holding its chunks, pages and public
// files (§35.7). A new worker takes over at once and keeps only the previous build's cache, and only for the chunks an
// open tab still asks for: navigations are answered from the current build alone, so nothing old is shown by mistake.
// After taking over it tells every tab `brain:build <sig>` (the router then does full loads). Caches that are not per
// build are named `brain:*`. (`sw: false` is a separate kill worker emitted by the build.) An app under a base path
// prefixes every name with it (`brain/app-<sig>`, `brain/app:meta`): apps sharing an origin never touch each other's
// caches, and an app at the root keeps the historical names.
// `cache.routes` (brain.json) adds declarative data routes: { match, prefix, strategy: cache-first | network-first |
// stale-while-revalidate | network-only, maxEntries, maxAge (s), timeout (ms) }, each in its own `brain:route:<match>`
// cache that survives builds, bounded by entries and age; personal responses are never stored. `cache.update:
// "prompt"` keeps a new worker waiting (while an older one controls tabs) until a page posts `brain:skip-waiting`.
const B = self.BRAIN_BUILD || { hash: 'dev', precache: [], offline: null }
const IMMUTABLE = Array.isArray(B.immutable) ? new Set(B.immutable) : null
const isHashed = path => IMMUTABLE ? IMMUTABLE.has(path) : path.startsWith(BASE + '_b/')
const BASE = B.base || '/' // app served under a base path (§23 #148); precache/offline URLs already carry it
const NS = 'brain' + BASE.replace(/\/$/, '') // `brain` at the root, `brain/app` under /app/
const BUILD = NS + '-' + B.hash, META = NS + ':meta'
let priorImmutable
const previousAssets = () => priorImmutable ||= caches.open(META).then(cache => cache.match('/__brain_immutable')).then(async response => {
  const manifests = response ? await response.json() : {}
  return new Set(Object.values(manifests).filter(Array.isArray).flat().filter(path => typeof path === 'string'))
}).catch(() => new Set())
const NAV_TIMEOUT = 3000
// precache downloads at once: hundreds in one burst tripped per-IP proxy rate limits (429) and competed with the page
const PRECACHE_PARALLEL = 4
// responses marked personal or uncacheable are never stored
const storable = r => r.ok && !/no-store|private/i.test(r.headers.get('cache-control') || '')
const ROUTES = (B.routes || []).map(r => ({ ...r, cache: NS + ':route:' + r.match + (r.prefix ? '*' : '') }))
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
      // a _b/ file is content-hashed: the same URL in an earlier build of this app holds the same bytes, so an
      // update downloads only what changed
      const olds = (await caches.keys()).filter(k => k.startsWith(NS + '-') && k !== BUILD)
      const kept = async u => {
        if (!isHashed(u)) return null
        for (const k of olds) {
          const r = await (await caches.open(k)).match(u)
          if (r) return r
        }
        return null
      }
      // default cache mode: the page may have just filled the HTTP cache with these immutable files
      const one = async u => {
        const r = (await kept(u)) || (await fetch(u))
        if (r.ok) await c.put(u, r)
      }
      // one failing URL must not abort the install: cache what answers, the rest is cached on first use
      const queue = [...B.precache]
      const lane = async () => {
        while (queue.length) await one(queue.shift()).catch(() => {})
      }
      await Promise.all(Array.from({ length: Math.min(PRECACHE_PARALLEL, queue.length) }, lane))
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
      // Keep exact custom asset identities for the same two generations as their bytes, including old open tabs.
      const stored = await meta.match('/__brain_immutable')
      if (IMMUTABLE || stored) {
        const decoded = stored ? await stored.json().catch(() => ({})) : {}
        const old = decoded && typeof decoded === 'object' ? decoded : {}
        const manifests = {}
        for (const generation of keep) {
          if (generation === BUILD) manifests[generation] = [...(IMMUTABLE || [])]
          else if (Array.isArray(old[generation])) manifests[generation] = old[generation]
          else if (generation.startsWith(NS + '-')) {
            // A pre-inventory worker treated only its historical namespace as immutable; infer no other cached URL.
            const entries = await (await caches.open(generation)).keys()
            manifests[generation] = entries.map(entry => new URL(entry.url, self.location.origin)).filter(url => url.origin === self.location.origin && url.pathname.startsWith(BASE + '_b/')).map(url => url.pathname)
          }
        }
        await meta.put('/__brain_immutable', new Response(JSON.stringify(manifests)))
        priorImmutable = Promise.resolve(new Set(Object.values(manifests).flat()))
      }
      for (const k of await caches.keys())
        if ((k.startsWith(NS + '-') && !keep.includes(k)) || (k.startsWith(NS + ':route:') && !ROUTES.some(r => r.cache === k))) await caches.delete(k)
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
  // A cached whole response cannot satisfy a byte range; live manifests must never lag one revalidation behind.
  if (req.headers?.has?.('range') || /\.(m3u8?|mpd)$/i.test(url.pathname)) return
  // offline: false (B.offline null) → navigations go straight to the network: no page cache, no offline page
  if (req.mode === 'navigate') return B.offline ? e.respondWith(navigate(e)) : undefined
  if (isHashed(url.pathname)) return e.respondWith(hashed(e))
  const rt = route(url.pathname)
  // A custom layout can change between builds; only exact retained inventory entries use old immutable bytes.
  if (IMMUTABLE) return e.respondWith((async () => {
    if ((await previousAssets()).has(url.pathname)) return hashed(e)
    if (rt) return rt.strategy === 'network-only' ? fetch(req) : data(e, rt)
    return req.destination ? revalidate(e) : fetch(req)
  })())
  if (rt) return e.respondWith(rt.strategy === 'network-only' ? fetch(req) : data(e, rt))
  // only files the page itself loads (img, style, script, font, manifest…); fetch()/XHR data is never touched (§35:
  // data is not cached by default)
  if (req.destination) e.respondWith(revalidate(e))
})

// cache-first: any generation may hold it (an old tab asks for old chunks); misses are stored in the current build.
// Every cache write is handed to waitUntil, so the worker stays alive until it lands; a failed write never fails the response.
async function hashed(e) {
  const req = e.request
  const hit = await caches.match(req)
  if (hit) return hit
  const r = await fetch(req)
  if (storable(r)) {
    const copy = r.clone()
    e.waitUntil(caches.open(BUILD).then(c => c.put(req, copy)).catch(() => {}))
  }
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
  const net = fetch(e.request).then(r => (storable(r) && e.waitUntil(c.put(e.request, r.clone()).catch(() => {})), r))
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
