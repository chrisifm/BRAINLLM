# Cache bench — the repeat visit (QA #124, INTERNALS §35)

Measures what §35 promises with no code from the author: the second visit downloads no unchanged JS or CSS and
paints sooner, visited routes open offline, `brain run` never leaves a service worker behind, and every
`brain.json` `cache.*` switch really turns its layer off. The build signature of §35.7 (#130: deploy under an
open tab, deleted chunk, identical rebuild, `/_health`) has its own section.

```sh
cargo build -p brain-cli
node bench/cache/run.mjs                 # 3 runs per app, then `brain run`, the opt-outs and the build signature (~6 min)
node bench/cache/run.mjs --runs 1 --apps tasks --skip-dev --skip-optouts --skip-signature
node bench/cache/run.mjs --report-only    # rewrite summary.md from summary.json
```

Writes `results/summary.json` and `results/summary.md`; per-run raw data (every request the server saw, the
page's request ledger, LCP, cache contents) goes to `results/raw/` (ignored by git).

## Method

- **Binary**: `target/debug/brain` is copied to the scratch dir first (other sessions rebuild `target/`), its
  sha256 is recorded. **Apps**: `tasks`, `landing`, `my-project` (copied from `$BRAIN_APPS`, default
  `~/tmp/brainllm/apps`; never built in place), `fresh` (`brain new`) and `data` (`brain new` plus an
  `api()` + `resource()` view, a referenced `public/logo.svg` and an unreferenced `public/notes.txt`). Each is built
  with `brain build` and no flags.
- **Server** (`lib/static.mjs`): a static host that does only what the build asks for. Headers come from
  `dist/_headers` (all matching rules merged, as Netlify does); like Netlify and Cloudflare Pages it adds a strong
  ETag to every file and answers `If-None-Match` with `304`; clean URLs (`/x`, `/x/` → `x/index.html`);
  precompressed `.br`/`.gz` served when accepted; `/_brain/*` never served. `/api/items` answers JSON with
  `no-store` after 1500 ms; `/me.json` answers with `Cache-Control: private` (checks that the worker never stores it). Every request is logged with the bytes written to the socket — this log, not the
  browser, is the source of truth for "bytes and requests that reached the network", because requests made by the
  service worker never appear in the page's own Network events.
- **Browser** (`lib/browser.mjs`): headless Chrome over CDP, one persistent profile per run, flattened sessions.
  Throttling: `Network.emulateNetworkConditions` latency 150 ms, 1.6 Mbps down, 750 Kbps up, applied to the page
  **and to every service-worker target** (otherwise the worker's fetches are unthrottled and "offline" does not reach
  it); `Emulation.setCPUThrottlingRate` 4; 412×915 mobile viewport. LCP from a `largest-contentful-paint`
  `PerformanceObserver` injected before the document.
- **First visit**: cold profile, navigate to `/`, wait until the worker is `activated` and the server has been quiet
  for 1.2 s (so the precache is counted). Then every other route is visited with a full navigation.
- **Second visit**: same profile, a new page, a new navigation to `/`; wait for `load` and 1.2 s of quiet.
- **Offline**: the server is closed **and** the page and the worker are emulated offline; every visited route is
  navigated again and must render its own `<h1>` (the text seen online) without a browser error page. A never-visited
  route is also opened and recorded.
- **`brain run`**: (a) a fresh profile on the dev server must have no registration; (b) a profile with the production
  worker registered on `127.0.0.1:<port>` then loads `brain run` on the same port: the registration and every
  `brain-*` cache must be gone.
- **Opt-outs** (app `data`, one `brain.json` per build): `sw:false` (no registration snippet, `sw.js` is the kill
  worker, a fresh visitor never registers, a past visitor's worker is removed when the `sw:false` build is deployed
  on the same origin) · `http:false` (no `_headers`, no nginx snippet, no `immutable` on any response) ·
  `offline:false` (visited routes must not open offline) · `data:false` (router back-navigation and the next visit
  must wait for the 1.5 s API; `brain:data` must not hold the response).
- The `data` view calls the API only after a click on "Load": a plain `resource(itemsApi.get)` is executed by the
  prerender in `brain build` and the page ships the error state (see failures).

## Latest run — binary `e9453256ff44`, 2026-10-13 (task #286)

The bench no longer crashes on case 5 (it used to throw on `c.navs` when `/` of `tasks` had no island). Full tables:
`results/summary.md`. Verdict rows that changed against the run below:

| criterion | result | evidence |
|---|---|---|
| 4c. `data:false` layer really off | FAIL | after a router back-nav, the first click on "Load" is lost: no request reaches `/api/items` and the list stays empty for 8 s; a second click loads it in 1511 ms. With the default cache the same click shows the items in 21 ms. Next visit 1508 ms (default 20 ms) and `brain:data` empty: the layer itself is off. Likely RUNTIME (island resume after client navigation drops the replayed click). Repro: build the `data` app with `"cache": {"data": false}`, open `/`, click Load, go to `/about` and back with the router links, click Load once: nothing happens. |
| 5. deleted chunk → 1 document load, no loop, island alive | BLOCKED | `tasks`' `/` reads the tasks channel, so it is mounted on load and has no island; the case ran on the local-state copy (it has one), but its `/` now lists the island chunk in `<link rel="modulepreload">`, so the tab fetched it before the deploy (200 from A) and resumed it from memory after (0 document loads, no request for the chunk). The precondition — a chunk the tab has not fetched yet — cannot be produced with today's build. |

Every other verdict row passes: cold visit downloads precached chunks once (0 B duplicated in all 5 apps), the worker
never stores `/api/*` or private responses, `offline:false`, 4a, 4b, `server.mjs` headers, repeat visit (0 B JS/CSS;
LCP 544→220 ms tasks, 768→200 landing, 800→208 my-project, 852→220 fresh), prerender of a plain `resource()`, hashed
`public/` paths, and the build signature cases 1, 3a–3c and 4.

Harness changes in this run: the `data` probe app is found by `data-p` attributes instead of classes (`brain build`
renames component classes, #260); a case whose precondition is not met is a BLOCKED row with the reason.

## Results — binary `7ba9716f8e24`, Chrome 155.0.8059.40, 2026-10-13, medians of 3 runs

Run after RUNTIME (six defects), TOOLCHAIN (three), BUILD #181 and COMPILER (B1102). `results/summary.md` holds the
full tables. Row 4b was added after the run with the same binary (`brain check --json` on the `data:false` app).

### Verdict

| item | result | evidence |
|---|---|---|
| 1. cold visit downloads precached chunks once | PASS | 0 B duplicated in all five apps |
| 2. worker never stores `/api/*` (no-store) nor `private` responses | PASS | no `brain-*` cache holds `/api/items` or `/me.json`, default and `data:false` builds |
| 3. `offline:false`: browser error page offline, no `_offline` built | PASS | `/`, `/about` and an unvisited route show the browser error page; no `_offline` in `dist/` |
| 4a. `data:false` emits `<meta name="brain-data-cache" content="off">` | PASS | present in `dist/index.html` |
| 4b. `data:false` warns B1102 for `api()` `cache:` | PASS (check only) | `brain check --json` reports B1102 at `api/items.jsbrain:1:58`; `brain build` does not print it |
| 4c. `data:false` layer really off | PASS | router back-nav shows items after 1506 ms (default 20 ms), next visit 1509 ms (default 21 ms), API 1500 ms; `brain:data` empty (default holds `/api/items`) |
| 5. deleted chunk → 1 document load, no loop, island alive | PASS | tab events `document /` → `load` (1 load); final build = B; the island adds a task |
| 6. server.mjs: `/sw.js` no-cache, static HTML ETag = signature | PASS | `/sw.js` `no-cache`; ETag of `/` `"uxm6s0we"` = printed signature |
| repeat visit: 0 B JS/CSS, LCP lower, visited routes offline | PASS | see the table below |
| prerender of a plain `resource()` does not freeze the error | PASS | `dist/index.html` ships `<p class="loading">Loading…</p>` |
| `public/` paths inside JS are hashed | **FAIL** (BUILD #181) | the client render still uses `/logo.svg`: chunk `c7ku4sfi.js` holds `<img src=/logo.svg alt=Logo …>` (minified markup, unquoted attribute) |
| manifest icons point at files that exist | PASS | icons `/_b/gdiv1rd4.svg`, `/_b/14bhc8bp.png`, `/_b/i3h3vxfy.png`, none missing |
| `/_health` on static apps | not exercised | `brain start` (§34, #119) does not exist yet |

### Repeat visit

| app | first bytes / requests / LCP | second bytes / requests / LCP | offline routes OK | notes |
|---|---|---|---|---|
| tasks | 32.7 KB / 18 / 736 ms | 3.7 KB / 4 / 192 ms | 2/2 (`/`, `/about`) | 2nd: `/`, `/manifest.webmanifest`, `/sw.js` ×2, all 304 |
| landing | 20.8 KB / 12 / 660 ms | 2.6 KB / 3 / 208 ms | 1/1 | 2nd: three 304s |
| my-project | 20.5 KB / 11 / 656 ms | 2.6 KB / 3 / 272 ms | 1/1 | 2nd: three 304s |
| fresh (`brain new`) | 30.0 KB / 15 / 784 ms | 3.5 KB / 4 / 192 ms | 2/2 | 2nd: four 304s |
| data | 35.5 KB / 19 / 728 ms | 3.5 KB / 4 / 192 ms | 2/2 | 2nd: four 304s |

The repeat visit transfers 0 bytes of JS/CSS everywhere, its LCP is 2.4–4.1× lower, every visited route opens
offline, and a never-visited route offline shows the built-in "You are offline" page. Bytes grew against the
previous binary because `_headers` now sends security headers on `/*` (CSP, HSTS, COOP…): each response, 304s
included, carries about 0.8 KB of headers (a 304 for `/` is 989 B with an empty body).

### `brain run`

| check | result |
|---|---|
| fresh profile on `brain run` | 0 registrations — PASS |
| production worker on the same origin, then `brain run` | registrations 1 → 0 → 0, no `brain-*` cache left — PASS |

### Opt-outs

| switch | result | evidence |
|---|---|---|
| `sw: false` | PASS | no snippet, `sw.js` = kill worker, fresh visitor 0 registrations, past visitor 1 → 0, caches `[]` |
| `http: false` | PASS | no `_headers` / nginx snippet, no `Cache-Control` on any response, `public/` not hashed |
| `offline: false` | PASS | item 3 |
| `data: false` | PASS (layer off; B1102 pending) | items 4a–4c |

### Build signature (§35.7)

Method (`lib/signature.mjs`): (1) app `tasks` built as A and as B (B changes the `<h1>` of `/` and `/about`); a tab on
A with its worker controlling, then B is swapped under the same server and the tab follows a router link to `/about`
and back to `/`; every request is classified by the file sets of A and B and by its `X-Brain-Build`. (2) A tab on
`/about` of A follows a router link to `/` (island not imported yet; `tasks`' own `/` reads the tasks channel, so the
build mounts it on load and ships no island: in that case the case runs on a copy whose `/` is the same task form with
local state, an island, and the row says "adapted"; with no island at all the row is BLOCKED with the reason); then B
is deployed and A's island chunk is gone
from the server, every worker cache and the HTTP cache (`Network.clearBrowserCache`); typing in the island must cost
one document load, and after it the form must add a task. (3) A `brain new` app built twice in place and once in a
fresh copy; then one byte changed in a `.brain`, a `.cssbrain`, a referenced and an unreferenced `public/` file.
(4) A `brain new` app with a `load()` view, served by `node dist/server.mjs` (what §34 runs as children); static apps
have no `/_health` because `brain start` does not exist yet. A case whose precondition is not met, or whose code
throws, is recorded as a BLOCKED row with the reason; the bench never crashes on it.

| case | result | evidence |
|---|---|---|
| 1. deploy A→B with an open tab | PASS | A `p29hvwhn` → B `paazkgkz`; tab: same-document `/about` → document `/about` ("About B" in 55 ms) → same-document `/` ("My tasks B"); 0 files of A returned, 0 responses carrying build A |
| 2. deleted chunk → one full reload, no loop, island alive | PASS | A's `/_b/2nbrpzvj.js` → 404 from B; tab: `document /` → `load` (1 load); page build `kmzrumiv` (B); the island adds a task |
| 3a. identical rebuild keeps the signature | PASS | `tmb0drq0` three times (same dir twice, fresh copy); 0 files differ |
| 3b. the signature is the same everywhere | PASS | meta, `sw.js`, `_headers` `X-Brain-Build`, nginx snippet all `tmb0drq0` |
| 3c. one changed source byte changes it | PASS | `.brain` → `6365agli` · `.cssbrain` → `7kph98bo` · `public/favicon.svg` → `c0q32h12` · unreferenced `public/notes.txt` → `umk5o6u4` |
| 4. `/_health` = printed signature | PASS | `node dist/server.mjs`: `/_health` `{"ok":true,"build":"utk98d03"}` = printed; `X-Brain-Build` on `/` and `/about`; `brain start` still "unknown command" (no `/_health` for static apps) |
| server.mjs headers | PASS | `/sw.js` `no-cache`; static HTML ETag = signature |

## Open failures and likely owner

1. **Client-rendered markup uses the unhashed `public/` URL** (BUILD #181). The JS chunk renders
   `<img src=/logo.svg …>`; the rewrite probably misses unquoted attributes in minified markup. The file is served at
   `/logo.svg` too, so nothing breaks, but it is fetched under a second, non-immutable URL.
2. **`brain build` does not print B1102** (COMPILER/TOOLCHAIN, minor): only `brain check` reports it.
3. **No `/_health` for static apps**: `brain start` (§34, TOOLCHAIN #119) does not exist yet.

Fixed and verified: duplicate precache downloads, the worker caching API and private responses, the router
swallowing the missing-chunk reload, `server.mjs` `/sw.js` max-age and HTML ETag, `data:false` (meta + layer off),
`offline:false` (no offline page, no `_offline`), manifest icons pointing at moved `public/` files, the prerender of `resource()`, B1102 under `data:false`.

Not failures, recorded for context: `dist/` uses `.html`/`.js`/`.css` and has no `.br` files (only `.gz`), unlike §10.
