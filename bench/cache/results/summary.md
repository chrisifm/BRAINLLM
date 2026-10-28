# Cache bench results (§35)

binary `6422654bbfa0` (brain 0.1.0) · 2026-10-13T23:03:55.700Z · Chrome Google Chrome 155.0.8059.40 · runs per app: 3 (medians)

Throttling: Network.emulateNetworkConditions latency 150 ms, down 1.60 Mbps, up 750 Kbps on the page and every service-worker target; CPU 4x; 412×915 mobile viewport

## Verdict

| item | result | evidence |
|---|---|---|
| 1. cold visit downloads precached chunks once | PASS | tasks 0 B duplicated · landing 0 B duplicated · my-project 0 B duplicated · fresh 0 B duplicated · data 0 B duplicated |
| 2. worker never stores /api/* (no-store) nor private responses | PASS | worker caches holding /api/items: [] · holding /me.json (private): [] |
| 3. offline:false → browser error page offline, no _offline built | PASS | /: error page · /about: error page · unvisited: error page · _offline in dist: [] |
| 4a. data:false: build emits <meta name="brain-data-cache" content="off"> | PASS | meta in dist/index.html: true |
| 4b. data:false: B1102 warning for api() cache: | PASS | `brain check --json` reports B1102: true · `brain build` prints it: true |
| 4c. data:false: layer really off | PASS | items after router back-nav 1513 ms (default 20 ms), next visit 1510 ms (default 21 ms), API 1500 ms · brain:data holds /api/items: false (default true) |
| 5. deleted chunk → exactly 1 document load, no loop, island alive | BLOCKED | precondition not met: the tab fetched A's island chunk /_b/m8p5dtjr.js before the deploy (200 (served from A); A's / lists it in <link rel="modulepreload">), so deleting it afterwards cannot be observed: the island resumed from memory (document loads 0, island adds a task: true) |
| 6. server.mjs: /sw.js no-cache, static HTML ETag = signature | PASS | /sw.js no-cache · ETag of / "6c7zi5at" vs signature 6c7zi5at |
| repeat visit: 0 B JS/CSS, LCP lower, visited routes offline | PASS | tasks 0 B, LCP 708→236 ms, offline 2/2 · landing 0 B, LCP 800→204 ms, offline 1/1 · my-project 0 B, LCP 1124→232 ms, offline 1/1 · fresh 0 B, LCP 876→264 ms, offline 6/6 · data 0 B, LCP 704→336 ms, offline 6/6 |
| prerender of a plain resource() does not freeze the error | PASS | <main data-b="bhng" class="home"><h1>Items</h1> <p class="loading">Loading…</p><!----> <a href="/about">About</a> <img src="/_b/8pkghzb1.svg |
| public/ paths inside JS are hashed | PASS | client-rendered img src /_b/8pkghzb1.svg · chunks with /logo.svg: [] |
| manifest icons point at files that exist | PASS | fresh app manifest icons: ["/_b/gdiv1rd4.svg","/_b/14bhc8bp.png","/_b/i3h3vxfy.png"] · missing: [] |

| app | first bytes / requests / LCP | second bytes / requests / LCP | 2nd JS/CSS bytes | offline routes OK | notes |
|---|---|---|---|---|---|
| tasks | 29.7 KB / 18 / 708 ms | 3.3 KB / 4 / 236 ms | 0 | 2/2 (/ ✓ /about ✓) | second visit: / 304, /manifest.webmanifest 304, /sw.js 304, /sw.js 304 |
| landing | 20.3 KB / 13 / 800 ms | 2.4 KB / 3 / 204 ms | 0 | 1/1 (/ ✓) | second visit: / 304, /manifest.webmanifest 304, /sw.js 304 |
| my-project | 20.3 KB / 12 / 1124 ms | 2.3 KB / 3 / 232 ms | 0 | 1/1 (/ ✓) | second visit: / 304, /manifest.webmanifest 304, /sw.js 304, /sw.js 304 |
| fresh | 50.8 KB / 31 / 876 ms | 3.6 KB / 4 / 264 ms | 0 | 6/6 (/ ✓ /403 ✓ /about ✓ /posts ✓ /posts/hello-world ✓ /posts/second-post ✓) | second visit: / 304, /sw.js 304, /manifest.webmanifest 304, /sw.js 304 |
| data | 55.6 KB / 35 / 704 ms | 3.8 KB / 3 / 336 ms | 0 | 6/6 (/ ✓ /403 ✓ /about ✓ /posts ✓ /posts/hello-world ✓ /posts/second-post ✓) | second visit: / 304, /manifest.webmanifest 304, /sw.js 304 |

## `brain run` registers no service worker

| check | result |
|---|---|
| fresh profile on `brain run`: registrations | 0 (PASS) |
| production worker on the same origin, then `brain run`: registrations before → after 1st load → after 2nd load | 1 → 0 → 0; brain-* caches after: [] (PASS) |

## Opt-outs (`brain.json` `cache.*`, app `data`)

| switch | expected | observed | result |
|---|---|---|---|
| `sw: false` | no registration snippet, kill worker, fresh visitor never registers, past visitor's worker removed | snippet absent · sw.js kill worker: true · fresh regs 0 · past visitor regs 1 → 0 → 0, caches after [] | PASS |
| `http: false` | no caching headers (no immutable, no Cache-Control); security headers may stay | _headers true · nginx true · immutable false · Cache-Control seen: none · public/ hashed: false | PASS |
| `offline: false` | browser error page offline (no cached pages, no offline page), no _offline built |  /: error page · /about: error page · unvisited: error page · _offline in dist: [] | PASS |
| `data: false` | resource() memory cache and Cache API data cache off | items after router back-nav: default 20 ms vs off 1513 ms · items on next visit (cache: '10m'): default 21 ms vs off 1510 ms (API takes 1500 ms) · brain:data has /api/items: default true vs off false · toolchain parses cache.data: true | PASS |
| (evidence) data:false changes the build | different output | sw.js manifest identical to the default build: false | PASS |
| (side finding) prerender of a plain `resource(api.get)` | loading state or data in the page | `<main data-b="bhng" class="home"><h1>Items</h1> <p class="loading">Loading…</p><!----> <a href="/about">About</a> <img src="/_b/8pkghzb1.svg" alt="Logo" width="…` | PASS |
| (side finding) hashed public/ URL in client-rendered markup | /_b/<hash>.svg | client render uses `/_b/8pkghzb1.svg`; chunks with /logo.svg: none | PASS |
| (side finding) worker caches API JSON | /api/* (no-store) never stored by the worker | build caches holding /api/items: default [] · data:false [] | PASS |

## Build signature (§35.7)

| case | result | evidence |
|---|---|---|
| 2. deleted chunk → one full reload, no loop, island alive | BLOCKED | precondition not met: the tab fetched A's island chunk /_b/m8p5dtjr.js before the deploy (200 (served from A); A's / lists it in <link rel="modulepreload">), so deleting it afterwards cannot be observed: the island resumed from memory (document loads 0, island adds a task: true) |
| 1. deploy A→B with an open tab | PASS | A `w3ho0n4f` → B `rjtf6c6s` · tab events after the deploy: same-document /about → document /about → load → same-document / · /about shows "About B" after 56 ms, / shows "My tasks B" · final page build `rjtf6c6s` · A-only files returned: 0 · responses carrying build A: 0 · caches: ["brain-w3ho0n4f","brain:meta","brain-rjtf6c6s"] |
| 3a. identical rebuild keeps the signature | PASS | printed `w8u1ftmu`, `w8u1ftmu`, `w8u1ftmu` (same dir twice, then a fresh copy) · files that differ: 0 / 0 |
| 3b. the signature is the same everywhere | PASS | meta `w8u1ftmu` · sw.js `w8u1ftmu` · _headers X-Brain-Build `w8u1ftmu` · nginx `w8u1ftmu` |
| 3c. one changed source byte changes it | PASS | .brain (visible text) → `ylairrak` (34 files) · .cssbrain (one hex digit) → `h0uni6wo` (30 files) · public/ referenced (favicon.svg) → `l9h32y6w` (34 files) · public/ unreferenced (notes.txt) → `fhr83bzj` (29 files) |
| 4. /_health = printed signature | PASS | served by node dist/server.mjs (PORT/HOST): printed `6c7zi5at`, /_health {"ok":true,"build":"6c7zi5at"}, X-Brain-Build on / and /about: `6c7zi5at`/`6c7zi5at` · `brain start`: exists |
| (side) server.mjs headers | PASS | /sw.js Cache-Control `no-cache` (§35.7: no-cache) · static HTML ETag "6c7zi5at" (§35.7: the signature) |
