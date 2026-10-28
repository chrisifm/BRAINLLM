# #21 final — results summary

Generated 2026-10-14 by `summarize.mjs`. Primary pair: Sonnet; Haiku is the second data point.

## Landing — "Nubo"

| Metric | Sonnet 5.5 (primary) | Haiku 5.5 |
|---|---|---|
| **Generation** | | |
| done (doctor 0 and routes ≥ brief) | yes (routes 6/1) | yes (routes 6/1) |
| wall time | 51.6 s | 137.1 s |
| tool calls / turns | 26 / 27 | 59 / 60 |
| tokens in / out | 197,579 / 7,368 | 1,018,892 / 24,275 |
| cost | $0.2169 | $0.0326 |
| stuck loops · human interventions | 0 · 0 | 0 · 0 |
| **Quality** | | |
| `brain doctor` exit | 0 | 0 |
| `brain test` passed | 0/0 (exit 0) | 2/2 (exit 0) |
| `brain build` on the app as generated | exit 0 | exit 0 |
| dist rules (INTERNALS §10) | ok: 0 source ext · 0 maps · 0 text files without .gz (—) · .br 0 | ok: 0 source ext · 0 maps · 0 text files without .gz (—) · .br 0 |
| Content-Type via `brain start` | ok (12 URLs, gzip 12) | ok (13 URLs, gzip 13) |
| Lighthouse a11y / SEO per page | `/` 100/100 | `/` 100/100 |
| `brain seo audit` score | `/` 91 | `/` 96 |
| functional smoke | 12/12; console errors 0 | 10/10; console errors 0 |
| smoke failures | — | — |
| **Performance** | | |
| build cold, median / p95 (10 runs) | 120.42 / 165.16 ms | 111.58 / 121.59 ms |
| build warm, median / p95 (10 runs) | 31.06 / 35.95 ms | 37.29 / 45.7 ms |
| main route HTML+JS+CSS raw / gz / br | 43.9 KB / 17.1 KB / 14.8 KB | 44.7 KB / 17.4 KB / 15.1 KB |
| …HTML · JS · CSS (gz) | 4.0 KB · 12.4 KB (5 files) · 0.7 KB | 3.1 KB · 12.5 KB (5 files) · 1.8 KB |
| first-route JS raw / gz / br | 30.2 KB / 12.4 KB / 10.9 KB | 30.6 KB / 12.5 KB / 11.0 KB |
| distinct files loaded · CSS fetched twice (SRI preload discarded) | 9 · 0 | 10 · 0 |
| HMR component | `views/home.brain` | `views/home.brain` |
| HMR style, tool median / p95 (20) | 2.55 / 4.04 ms (css, errors 0) | 2.47 / 3.08 ms (css, errors 0) |
| HMR script, tool median / p95 (20) | 2.61 / 3.96 ms (js, errors 0) | 2.7 / 3.25 ms (js, errors 0) |
| HMR incl. OS file event, median | style 32.2 · script 32.42 ms | style 33.52 · script 34.36 ms |
| HMR state preserved · page reloaded | true · false | true · false |

## Panel — "Nubo Admin"

| Metric | Sonnet 5.5 (primary) | Haiku 5.5 |
|---|---|---|
| **Generation** | | |
| done (doctor 0 and routes ≥ brief) | yes (routes 9/1) | yes (routes 9/1) |
| wall time | 80.1 s | 198.2 s |
| tool calls / turns | 42 / 43 | 74 / 75 |
| tokens in / out | 434,563 / 11,795 | 1,595,641 / 40,033 |
| cost | $0.2790 | $0.0492 |
| stuck loops · human interventions | 0 · 0 | 0 · 0 |
| **Quality** | | |
| `brain doctor` exit | 0 | 0 |
| `brain test` passed | 0/0 (exit 0) | 2/2 (exit 0) |
| `brain build` on the app as generated | exit 0 | exit 0 |
| dist rules (INTERNALS §10) | ok: 0 source ext · 0 maps · 0 text files without .gz (—) · .br 0 | ok: 0 source ext · 0 maps · 0 text files without .gz (—) · .br 0 |
| Content-Type via `brain start` | ok (21 URLs, gzip 21) | ok (25 URLs, gzip 25) |
| Lighthouse a11y / SEO per page | `/` 100/100<br>`/invoices` 100/100<br>`/settings` 100/100 | `/` 100/100<br>`/invoices` 100/100<br>`/settings` 100/100 |
| `brain seo audit` score | `/` 91, `/invoices` 87, `/settings` 87 | `/` 91, `/invoices` 91, `/settings` 91 |
| functional smoke | 13/13; console errors 0 | 13/13; console errors 0 |
| smoke failures | — | — |
| **Performance** | | |
| build cold, median / p95 (10 runs) | 159.19 / 228.45 ms | 170.26 / 251.7 ms |
| build warm, median / p95 (10 runs) | 43.3 / 62 ms | 63.94 / 96.32 ms |
| main route HTML+JS+CSS raw / gz / br | 45.5 KB / 18.6 KB / 16.2 KB | 51.2 KB / 20.8 KB / 18.1 KB |
| …HTML · JS · CSS (gz) | 3.2 KB · 14.6 KB (10 files) · 0.7 KB | 2.7 KB · 16.3 KB (13 files) · 1.8 KB |
| first-route JS raw / gz / br | 35.2 KB / 14.6 KB / 13.0 KB | 38.6 KB / 16.3 KB / 14.4 KB |
| distinct files loaded · CSS fetched twice (SRI preload discarded) | 14 · 0 | 18 · 0 |
| HMR component | `comp/customer-form.brain` | `comp/customer-form.brain` |
| HMR style, tool median / p95 (20) | 1.26 / 1.92 ms (css, errors 0) | 1.69 / 2.15 ms (css, errors 0) |
| HMR script, tool median / p95 (20) | 1.78 / 2.32 ms (js, errors 0) | 2.25 / 4.52 ms (js, errors 0) |
| HMR incl. OS file event, median | style 31.85 · script 32.86 ms | style 31.68 · script 33.51 ms |
| HMR state preserved · page reloaded | true · false | true · false |

Other routes of the Sonnet panel (gz HTML+JS+CSS): `/invoices` 14.1 KB, `/settings` 14.1 KB.

Binary: `brain` release sha256 `018520473d6e` (measurement) · generation binary (frozen by the harness, run tag `final21b`) `018520473d6e` · spec sha256 `e745bbfd456e`.
