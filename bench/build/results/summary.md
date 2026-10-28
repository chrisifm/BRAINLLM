## Build time (ms, median ± stddev, 20 runs)

| Tool | Cold | Warm | Warm min | Warm median vs BRAINLLM |
|---|---:|---:|---:|---:|
| BRAINLLM | 55.3 ± 38.7 | 48.2 ± 13.4 | 29.4 | 1.0× |
| Svelte 5 + Vite | 378.4 ± 106.9 | 357.8 ± 43.2 | 322.6 | 7.4× |
| React 19 + Vite | 352.5 ± 61.1 | 338.7 ± 57.1 | 278 | 7.0× |

## Tool cold start (ms, median ± stddev)

| Command | Time |
|---|---:|
| `~/tmp/qa286/build-work/bin/brain help` | 3.8 ± 0.4 |
| `npx vite --version` | 251.2 ± 41.4 |

## Route payload — what the browser receives for the dashboard route (KiB; each file compressed on its own)

| App | Route | HTML gz | CSS gz | JS gz | Total raw | Total gzip -9 | Total brotli -q 11 | JS files |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| BRAINLLM | `/dashboard` (GET /dashboard from dist/server.mjs) | 2.24 | 0.14 | 9.21 | 24.22 | 11.59 | 9.98 | 9 |
| Svelte 5 + Vite | `/` (dist/index.html) | 0.46 | 0.40 | 15.86 | 42.62 | 16.72 | 15.03 | 1 |
| React 19 + Vite | `/` (dist/index.html) | 0.46 | 0.38 | 67.34 | 219.49 | 68.17 | 58.66 | 1 |

## Client files in dist/ (KiB; JS+CSS+HTML a browser can be served, each file compressed on its own)

| App | Files | Raw | gzip -9 | brotli -q 11 | + service worker and offline page (gz) |
|---|---:|---:|---:|---:|---:|
| BRAINLLM | 11 | 27.56 | 12.45 | 10.90 | 2.87 |
| Svelte 5 + Vite | 3 | 42.62 | 16.72 | 15.03 | — |
| React 19 + Vite | 3 | 219.49 | 68.17 | 58.66 | — |

## Per file (bytes)

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

## Versions

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
