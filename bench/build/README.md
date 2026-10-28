# Build benchmark — BRAINLLM vs Svelte 5 + Vite vs React 19 + Vite

**Question.** For the same app, how long does a production build take, and how many bytes reach the browser?
(task #70)

Vite is used **only** as a comparison reference here; it is not a dependency of BRAINLLM.

## 1. The app

The BRAINLLM app is the frozen fixture **`bench/build/app/`**: the users dashboard as it was in `examples/` at the
first commit (`efc66c8`), without `examples/invalid/`. It has the `app.brain` shell, one route
(`views/dashboard.brain`: users CRUD with `load()`, search, `UserForm`, `Badge`), plus `comp/counter`, `comp/todo`,
`lib/format.jsbrain` and `lib/api.js` (in-memory API, 50 ms latency).

The benchmark used to build `examples/` directly. `examples/` has since grown (a second route `/`, new tokens, a new
SEO block), so BRAINLLM was building a bigger app than the Svelte and React ports, which still implement the first
version. The fixture freezes the app the ports implement, so the three builds do the same work. Change the fixture
only together with both ports.

It is ported, feature for feature and with the same markup and CSS, to:

| Reference | Source | Notes |
|---|---|---|
| Svelte 5 + Vite | `ref/svelte/` | runes (`$state`, `$derived`, `$effect`), scoped `<style>` per component |
| React 19 + Vite | `ref/react/` | hooks, one plain `.css` file per component, nested under the component's root class |

Both use the same `index.html` shell (head, skip link, nav, footer) and the same `<head>` SEO tags as the BRAINLLM
view. No router and no extra libraries. Versions are pinned exactly in each `package.json` and `package-lock.json`
(`npm ci`).

Differences that remain, on purpose (each is the default output of its tool for this app):

- **Server rendering.** The route has `load()`, so `brain build` emits a production server (`dist/server.mjs`) and
  renders `/dashboard` on each request with the users in the HTML; it emits no static HTML file for that route. The
  references are static Vite builds: `dist/index.html` is an empty shell and the client calls `api.list()` once on
  mount. The users therefore arrive in BRAINLLM's HTML and in the references' JS.
- **Service worker and offline page.** `brain build` adds them by default (`"cache"` in `brain.json`, SPEC §cache);
  the references have no equivalent. They are measured in their own column, not in the route payload or the client
  total.
- `counter` and `todo` are not used by the route in any of the three apps, so none of them ships them.
- `log(n)` in `counter` becomes a dev-only `console.log` behind `import.meta.env.DEV` in the references.

## 2. Method (reproducible)

```sh
bench/build/run.sh                     # RUNS=20 WARMUP=2 BENCH_WORK=$HOME/tmp/brainllm-build
BRAIN_BIN=/path/to/brain bench/build/run.sh   # skip `cargo build` (use a prebuilt binary)
```

| Step | What `run.sh` does |
|---|---|
| Binary | `cargo build --release -p brain-cli` (or `BRAIN_BIN` / `BRAIN` = a prebuilt binary), then copies it to `$BENCH_WORK/bin/brain` (removing the old copy first) so a concurrent rebuild cannot change it mid-run. The sha256 of that copy is recorded. |
| App copies | `app/` (the frozen fixture) → `$BENCH_WORK/app`; `ref/*` → `$BENCH_WORK/{svelte,react}` + `npm ci`. A failing `brain build` stops the run and prints its output. |
| Build time | `hyperfine -N --warmup 2 --runs 20` from each app directory. BRAINLLM: `brain build`. References: `npx vite build --logLevel silent` (the `npx` resolution is part of what a developer runs; its cost is shown separately by the cold-start row). |
| Cold | `--prepare` removes `dist/` and the tool cache before **every** run (`.brain/cache`; `node_modules/.vite`). The OS file cache is not purged (that needs `sudo purge`). |
| Warm | `--prepare` removes only `dist/` |
| Tool cold start | `brain help` vs `npx vite --version` (informational). `brain --version` does not exist (§5, defect 1). |
| Route payload | what the browser receives for the dashboard route: its HTML document + the stylesheets it links + the JS it fetches before it is interactive (`<script src>`, `<link rel=modulepreload>`, inline `import … from`, then every static import, recursively; dynamic `import()` is not followed). BRAINLLM's HTML is the response of its own production server: `summarize.mjs` starts `node dist/server.mjs` on a free port, fetches `GET /dashboard` and stops it by PID. The references' HTML is `dist/index.html`. |
| Client files | every `.js`, `.css` and `.html` in `dist/` that a browser can be served. Server-only output (`dist/server.mjs`, `dist/_server/`, `dist/_brain/`) is excluded; BRAINLLM's `sw.js` and `_offline.html` are reported in their own column. |
| Compression | each file raw, `gzip -9 -n` and `brotli -q 11`, one file at a time (how a static server would serve them) |
| Output | `results/hf-*.json` (raw hyperfine, git-ignored), `results/versions.json` (git-ignored), `results/summary.json` and `results/summary.md` (kept) via `summarize.mjs` |

Neither tool has a persistent build cache that applies here (`brain build` has none; Vite's
`node_modules/.vite` is the dev-server dependency cache, not used by `vite build`), so cold ≈ warm for both and the
difference between the two columns is noise.

## 3. Results

Run of 2026-10-13 on the frozen fixture, brain binary `e9453256ff44`. **Caveat:** the machine was shared with other
jobs (1-minute load average 10 → 14 during the timing, 10 cores), so every timing row has a large stddev and is
slower than on an idle machine; the min column is the least noisy estimate. Byte counts are exact and deterministic,
except the BRAINLLM HTML, which carries a per-request CSP nonce (±a few bytes compressed).

### Build time (ms, median ± stddev, 20 runs)

| Tool | Cold | Warm | Warm min | Warm median vs BRAINLLM |
|---|---:|---:|---:|---:|
| BRAINLLM | 55.3 ± 38.7 | 48.2 ± 13.4 | 29.4 | 1.0× |
| Svelte 5 + Vite | 378.4 ± 106.9 | 357.8 ± 43.2 | 322.6 | 7.4× |
| React 19 + Vite | 352.5 ± 61.1 | 338.7 ± 57.1 | 278 | 7.0× |

### Tool cold start (ms, median ± stddev)

| Command | Time |
|---|---:|
| `~/tmp/qa286/build-work/bin/brain help` | 3.8 ± 0.4 |
| `npx vite --version` | 251.2 ± 41.4 |

### Route payload — what the browser receives for the dashboard route (KiB; each file compressed on its own)

| App | Route | HTML gz | CSS gz | JS gz | Total raw | Total gzip -9 | Total brotli -q 11 | JS files |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| BRAINLLM | `/dashboard` (GET /dashboard from dist/server.mjs) | 2.24 | 0.14 | 9.21 | 24.22 | 11.59 | 9.98 | 9 |
| Svelte 5 + Vite | `/` (dist/index.html) | 0.46 | 0.40 | 15.86 | 42.62 | 16.72 | 15.03 | 1 |
| React 19 + Vite | `/` (dist/index.html) | 0.46 | 0.38 | 67.34 | 219.49 | 68.17 | 58.66 | 1 |

### Client files in dist/ (KiB; JS+CSS+HTML a browser can be served, each file compressed on its own)

| App | Files | Raw | gzip -9 | brotli -q 11 | + service worker and offline page (gz) |
|---|---:|---:|---:|---:|---:|
| BRAINLLM | 11 | 27.56 | 12.45 | 10.90 | 2.87 |
| Svelte 5 + Vite | 3 | 42.62 | 16.72 | 15.03 | — |
| React 19 + Vite | 3 | 219.49 | 68.17 | 58.66 | — |

### Per file (bytes)

| App | File | Raw | gzip -9 | brotli -q 11 |
|---|---|---:|---:|---:|
| brain | `_b/1hqq6mge.css` | 163 | 139 | 118 |
| brain | `_b/54xu4jzh.js` | 0 | 20 | 1 |
| brain | `_b/6ecyojt1.js` | 338 | 249 | 229 |
| brain | `_b/9dnhdxed.js` | 6988 | 3153 | 2837 |
| brain | `_b/bzsivz2u.js` | 3905 | 1954 | 1703 |
| brain | `_b/j4wvmyh0.js` | 806 | 469 | 403 |
| brain | `_b/lmf04nzt.js` | 2607 | 1419 | 1209 |
| brain | `_b/mvshex02.js` | 1418 | 631 | 539 |
| brain | `_b/x9swiwsi.js` | 434 | 286 | 239 |
| brain | `_b/xihp5q0c.js` | 3172 | 1253 | 1090 |
| brain | `_b/zij5q0tj.js` | 8388 | 3178 | 2793 |
| brain | `_offline.html` | 534 | 370 | 239 |
| brain | `sw.js` | 5777 | 2569 | 2144 |
| svelte | `assets/index-DhHglChh.js` | 41733 | 16237 | 14747 |
| svelte | `assets/index-Dvnwz4MV.css` | 941 | 412 | 345 |
| svelte | `index.html` | 969 | 470 | 302 |
| react | `assets/index-C15t65u3.css` | 833 | 385 | 326 |
| react | `assets/index-TNuyrJbz.js` | 222960 | 68954 | 59449 |
| react | `index.html` | 969 | 472 | 293 |

### Versions

| Item | Value |
|---|---|
| brain binary sha256 (12) | e9453256ff44 |
| brain binary size (bytes) | 4262432 |
| rustc | rustc 1.96.0 (ac68faa20 2026-05-25) |
| node | v26.8.1 |
| npm | 11.19.0 |
| vite | 8.3.4 |
| svelte | 5.57.2 |
| @sveltejs/vite-plugin-svelte | 7.3.1 |
| react / react-dom | 19.3.0 / 19.3.0 |
| @vitejs/plugin-react | 6.1.2 |
| hyperfine | 1.20.0 |
| cpu | Apple M5, 10 cores |
| memory (GiB) | 24 |
| load average 1/5/15 min (start → end of timing) | 9.97 14.19 15.03 → 13.78 14.38 15.04 |
| os | macOS 26.6.2 (25.6.0) |

## 4. Reading the numbers

- **Build time.** `brain build` is one native process that parses, checks, compiles, minifies and writes the
  site. `vite build` starts Node, resolves `npx`, loads the config and the framework plugin, then bundles with
  Rolldown. Most of the gap is fixed cost (Node + plugin start). On this run the warm median gap is about 7×
  (BRAINLLM 48 ms, now also writing the server, the service worker and SEO files); the earlier 19–23× came from an
  idle-machine run of a smaller build and is not comparable.
- **Bytes.** For the dashboard route the browser receives 11.59 KiB gz from BRAINLLM (HTML with the users already
  rendered, 9.21 KiB of JS in 9 modules), 16.72 KiB from Svelte and 68.17 KiB from React. BRAINLLM ships its runtime
  core plus the router and one small module per component; the references ship the framework
  runtime in one bundle. Svelte 5's runtime (proxies, effects scheduler) dominates its 15 KB; React's 58 KB is almost
  entirely `react-dom`.
- BRAINLLM's file count is higher (11 client files vs 3) because modules are split per component and
  content-addressed for long-term caching; with HTTP/2 and `modulepreload` they are fetched in parallel, so the count does not add
  round trips.

## 5. BRAINLLM defects found while measuring

| # | Defect | Status on 2026-10-13 (binary `e9453256ff44`) |
|---|---|---|
| 1 | `brain --version` was an unknown command | fixed: prints `brain 0.1.0` |
| 2 | no SRI and no CSP in the build output | fixed: `integrity="sha384-…"` on links, CSP in `dist/_headers` and per request (nonce) in `server.mjs` |
| 3 | a `ui/` component whose uses are all inlined still ships a side-effect chunk | not re-checked |
| 4 | a view with `load()` built to a page with blank values (no server, #51) | fixed: `brain build` emits `dist/server.mjs`, which renders the data |
| 5 | the example's canonical says `/users` while the route is `/dashboard` | kept in the fixture on purpose: the references copy the same `<head>` |
| 6 | an empty module (`_b/54xu4jzh.js`, 0 bytes) is emitted and `modulepreload`ed by every page: one wasted request | `dist/_b/54xu4jzh.js` is 0 bytes and is listed in the page's `<link rel="modulepreload">` (see the per-file table); also seen in the tasks app and the jsfb app |
