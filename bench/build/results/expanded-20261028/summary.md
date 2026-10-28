# Expanded dashboard results — October 28, 2026

BRAINLLM frozen source `7889787`; Apple M5, 10 cores, 24 GiB RAM, macOS Darwin 25.6 arm64; Node 26.12.0; Chrome/156.0.8078.49.

Twenty warm production builds after two warmups. Lower values mean less time or fewer bytes for this fixture. Shared-machine load and fixed candidate order limit generalization. See [methodology](../../expanded.md).

| Stack | Median build (ms) | Std. dev. (ms) | Min–max (ms) | Initial route gzip (KiB) | Browser checks |
|---|---:|---:|---:|---:|---:|
| BRAINLLM | 34.32 | 13.23 | 28.55–68.17 | 11.06 | 9/9 |
| React + Vite | 155.49 | 16.71 | 137.12–188.92 | 68.17 | 9/9 |
| Svelte + Vite | 243.53 | 56.14 | 200.95–400.35 | 16.72 | 9/9 |
| Vue + Vite | 211.76 | 53.56 | 175.31–393.62 | 27.26 | 9/9 |
| Angular CLI | 1262.50 | 69.03 | 1052.48–1347.23 | 54.60 | 9/9 |
| Astro + React island | 504.78 | 77.45 | 437.34–674.84 | 71.68 | 9/9 |
| React + webpack | 1334.54 | 560.99 | 1115.85–3396.24 | 68.78 | 9/9 |

All seven applications completed the same nine CRUD smoke assertions with zero uncaught runtime exceptions. Payload totals cover initial successful same-origin HTML/CSS/JavaScript response bodies, compressed individually at gzip level 9; they exclude headers, images, later requests and offline precache. These figures do not measure browser rendering performance.

The webpack sample has substantial variance; inspect its raw samples before drawing conclusions. BRAINLLM uses SSR, Astro hydrates a React island, and the other references are client-rendered. This is not an all-features comparison.

Raw evidence: `environment.json`, `timing-*.json`, `versions-*.json`, and `browser.json`. Exact byte counts and chart-ready values are in [summary.json](summary.json). Lockfiles are under `bench/build/ref/`. The historical report remains unchanged.
