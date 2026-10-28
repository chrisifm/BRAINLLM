# js-framework-benchmark: BRAINLLM vs React, Vue, Angular, Svelte, Solid (keyed)

These numbers come from the official [krausest/js-framework-benchmark](https://github.com/krausest/js-framework-benchmark)
runner (puppeteer, headless Chrome) with its own correctness checks. The BRAINLLM implementation in `brainllm/` is
plain BrainScript that follows `SPEC.md`. It contains no hand-written JS outside the framework.

## Results (run 2026-10-13, 15 iterations per CPU benchmark, the runner's default)

Values are medians. The runner computes the slowdown factor as value / fastest value of the frameworks compared here, then
takes the weighted geometric mean with its own weights (`webdriver-ts-results/src/Common.ts`). The "vs vanillajs" row
divides by vanillajs instead of by the fastest framework. Full numbers, 95% CIs and run counts are in `results/summary.json`.

### CPU (ms, lower is better; the runner applies CPU throttling to `_x16`, `_x8` and `_x2`)

| Benchmark | vanillajs | solid 1.9.3 | svelte 5.42.1 | vue 3.5.39 | **brainllm 0.1.0** | react-hooks 19.2.0 | angular-cf 22.2.0 |
|---|---:|---:|---:|---:|---:|---:|---:|
| 01 create 1,000 rows | 21.4 | 22.2 | 22.9 | 26.0 | **25.3** | 25.5 | 44.0 |
| 02 replace all rows | 20.9 | 22.7 | 23.9 | 27.6 | **26.7** | 28.4 | 45.1 |
| 03 partial update (x16) | 14.0 | 15.9 | 16.9 | 16.0 | **16.2** | 17.3 | 25.3 |
| 04 select row | 3.5 | 5.5 | 6.7 | 5.8 | **7.3** | 7.9 | 8.3 |
| 05 swap rows | 14.8 | 18.3 | 18.0 | 17.4 | **18.4** | 105.8 | 17.1 |
| 06 remove row | 13.0 | 12.6 | 12.9 | 14.4 | **13.2** | 14.7 | 21.0 |
| 07 create 10,000 rows | 201.8 | 226.5 | 226.6 | 244.0 | **296.1** | 426.1 | 331.2 |
| 08 append 1,000 rows (x2) | 24.8 | 25.4 | 25.0 | 27.4 | **28.8** | 28.7 | 46.9 |
| 09 clear rows (x8) | 9.6 | 11.5 | 12.4 | 13.7 | **13.7** | 17.4 | 31.7 |
| **Weighted geomean (vs fastest)** | **1.00** | **1.10** | **1.15** | **1.23** | **1.26** | **1.49** | **1.97** |
| Weighted geomean (vs vanillajs) | 1.00 | 1.10 | 1.14 | 1.22 | 1.26 | 1.48 | 1.96 |

### Memory (MB, one run each, the runner's default)

| Benchmark | vanillajs | solid | svelte | vue | **brainllm** | react-hooks | angular-cf |
|---|---:|---:|---:|---:|---:|---:|---:|
| 21 ready memory | 0.54 | 0.59 | 0.67 | 0.86 | **0.56** | 1.18 | 1.53 |
| 22 run memory (1k rows) | 1.87 | 2.69 | 2.87 | 3.89 | **3.93** | 4.43 | 4.62 |
| 25 create/clear 1k x5 | 0.65 | 0.77 | 0.98 | 1.19 | **0.84** | 1.98 | 2.23 |
| Geomean vs fastest | 1.00 | 1.23 | 1.42 | 1.82 | **1.41** | 2.51 | 2.89 |

### Size and startup

| Metric | vanillajs | solid | svelte | vue | **brainllm** | react-hooks | angular-cf |
|---|---:|---:|---:|---:|---:|---:|---:|
| 42 transferred, compressed (kB, server brotli) | 2.5 | 4.5 | 9.7 | 23.3 | **5.2** | 51.4 | 45.5 |
| 41 transferred, uncompressed (kB) | 11.3 | 11.5 | 26.6 | 64.3 | **12.5** | 190.3 | 147.0 |
| JS bundle, gzip -9 (bytes, sum per file) | 2,521 | 4,810 | 10,690 | 25,857 | **4,570** (4 modules) | 61,132 | 51,201 |
| 43 first paint (ms) | 511.5 | 456.6 | 449.0 | 449.5 | **490.9** | 484.0 | 498.3 |

In headless mode, first paint is dominated by browser and page-load overhead, and it does not separate the
frameworks here.

### Reading the BRAINLLM numbers

- **Overall:** BRAINLLM is at 1.26x, close to Vue (1.23x), ahead of React (1.49x) and Angular (1.97x), and behind
  Solid (1.10x) and Svelte (1.15x).
- **07 create 10,000 rows is the outlier** (296 ms against about 227 ms for Solid and Svelte). Above 500 items,
  `{#each}` creates new rows in slices of 5 ms or less and yields between slices
  (`runtime/brain.js:334-343`, yield helper at `runtime/brain.js:213`). The runner still measures up to the last commit,
  so the number includes every slice and stays honest. The cost is 3–4 commits (intermediate paints) instead of 1:
  paint time is 236.6 ms against 197.2 ms for Solid, and script time is 47.7 ms against 22.1 ms. This is a deliberate
  INP trade-off, not a correctness problem.
- **04 select row** (7.3 ms) uses idiomatic `class:danger={selected === r.id}`. Each row subscribes to `selected`, so
  a selection change re-runs one small effect per row. Solid avoids that cost with `createSelector`.
- **01 create rows includes hydration.** The page is prerendered and hydrates lazily on the first interaction
  (`runtime/brain.js:443`). The first click on "Create 1,000 rows" imports the view module, mounts the view and
  re-dispatches the click, and that cost is inside the 25.3 ms.
- **Memory:** the ready and clear numbers are near vanillajs (no VDOM). Run memory (3.93 MB) is close to Vue: each row
  allocates an item signal and an index signal plus one effect per binding.

## Correctness

- `npm run isKeyed -- keyed/brainllm` reports: "brainllm-v0.1.0-keyed is keyed for 'run benchmark' and keyed for
  'remove row benchmark' and keyed for 'swap rows benchmark'".
- The full suite ran with no runner error. The DOM assertions of every benchmark passed: row counts, `tr.danger`
  count, label text after update, and the swap and remove positions.

## Machine

| | |
|---|---|
| CPU | Apple M5 (`sysctl -n machdep.cpu.brand_string`) |
| OS | macOS 26.6.2 |
| Browser | Google Chrome 155.0.8059.40 (system install, `--headless=new`) |
| Node | v26.8.1 |
| Benchmark commit | `bf894b681fb56433ccb6efb4a3c93e35eda2b20d` |
| Total runner time | 37 min (01:59 → 02:36, 7 frameworks, all benchmarks) |

## Reproduce

```bash
# 1. Benchmark checkout and its tooling. --legacy-peer-deps is needed only because of an eslint peer conflict
#    in the root devDependencies at this commit.
git clone https://github.com/krausest/js-framework-benchmark ~/tmp/jsfb
cd ~/tmp/jsfb && git checkout bf894b681fb56433ccb6efb4a3c93e35eda2b20d
npm ci --legacy-peer-deps && npm run install-local

# 2. Reference implementations, with their own lockfiles and build commands
for f in vanillajs react-hooks vue angular-cf svelte solid; do
  (cd frameworks/keyed/$f && npm ci && npm run build-prod)
done

# 3. BRAINLLM: brain build -> frameworks/keyed/brainllm (rebases /_b/ URLs, writes package.json + package-lock.json).
#    Pin the compiler with BRAIN=/path/to/brain; the script prints the binary's sha256.
bench/jsfb/build.sh ~/tmp/jsfb   # or JSFB=<checkout>; with no argument the default is ~/tmp/jsfb

# 4. Server (terminal 1) and runner (terminal 2)
npm start
cd webdriver-ts
npm run isKeyed -- keyed/brainllm
npm run bench -- --headless --framework keyed/vanillajs keyed/brainllm keyed/solid keyed/svelte keyed/vue keyed/react-hooks keyed/angular-cf

# 5. Summary (this table + results/summary.json)
cd <repo root>
cp ~/tmp/jsfb/webdriver-ts/results/*.json bench/jsfb/results/raw/
node bench/jsfb/summarize.mjs bench/jsfb/results/raw bench/jsfb/results/summary.json
```

`results/raw/` (the runner's JSON) is git-ignored. `results/summary.json` (under 10 kB) is committed.

### Which BRAINLLM build was measured

- The app was built by `target/debug/brain` at about 01:57, with the repo uncommitted at the time. A later rebuild by
  another session overwrote that binary at 02:19, so its sha256 was not captured. The measured artifact is pinned by its
  output hashes instead (files as installed, after the `/_b/` rebase):
  `_b/cl628oyy.js` (runtime) `7686e383…a0a8` · `_b/o43iokyb.js` (view) `c091665b…4497` · `_b/pw8g2mmd.js` `df3761d2…6ace`
  · `_b/e231l6rr.js` `c4101565…0ed3` · `_b/4pbe4lut.css` `9807c170…d2e` · `index.html` `b6b4a077…4867`.
- Binary `e51def85b7b23a7aac771be46443ffbcb6eacba7b02b2a6c443fe7f1f089cdc2` (repo HEAD `efc66c87`, frozen copy at
  `~/tmp/jsfb/brain-e51def85`) rebuilds this app without errors. Its runtime is byte-identical. Its view
  module differs only in mangled local names and an added `seo` metadata function, with no behavioral change.
  Reproduce with `BRAIN=~/tmp/jsfb/brain-e51def85 bench/jsfb/build.sh`.

### Building with today's toolchain (task #286)

- **Checkout.** `build.sh` takes the js-framework-benchmark checkout as its first argument, else `$JSFB`, else
  `~/tmp/jsfb`. It stops with "not a js-framework-benchmark checkout" only when that directory has no
  `frameworks/keyed/`.
- **No service worker.** `brain build` now adds a service worker and an offline page by default. The runner measures
  DOM operations on one page, so `brainllm/brain.json` turns both off (`"cache": {"sw": false, "offline": false}`):
  the page has no registration script, like the other frameworks. With `"sw": false` the build still writes a
  kill-switch `sw.js` (it unregisters a worker left by an earlier deploy); nothing requests it here, so `build.sh` does
  not install it, and it fails if `index.html` still registers a worker.
- **URL rebase.** Besides the `/_b/` URLs, `build.sh` rebases the root-absolute `manifest.webmanifest` link, and
  recomputes every `integrity="sha384-…"` (SRI) from the installed file, because the rebase changes the bytes of the
  files it rewrites. It drops files the page never requests (`_brain/`, `_headers`, `llms*.txt`, `*.md`, `robots.txt`,
  `*.gz`).
- **Status.** Built on 2026-10-13 with binary `e9453256ff44`: the build passes every guard, and in the runner's own
  server (`npm start`, port 8080) the page creates 1,000 rows, swaps rows and selects a row. **The full runner was not
  re-run, and it should not be until a runtime defect is fixed:** the per-component circuit breaker (B0908, runs budget
  1,000 per second, `runtime/brain-guard.js`) trips on the second list operation of the same second (for example
  "Create 1,000 rows" then "Clear"), and replaces the whole view with "This part stopped to protect the page".
  The runner chains operations exactly like that, so the CPU table above (built before the breaker existed) stays the
  last valid run.

## Notes on the implementation (`brainllm/`)

| File | Role |
|---|---|
| `app.brain` | Shell: loads the benchmark's shared `/css/currentStyle.css` (Bootstrap) |
| `views/home.brain` | The whole app: `rows`/`selected` signals, the six actions, keyed `{#each rows as r (r.id)}`, `class:danger` |
| `ui/action-button.brain` | One benchmark button (`id`, `text`, `emits="press"`) |
| `lib/build-data.jsbrain` | Pure adjectives/colours/nouns generator `buildData(count, firstId)` |
| `tokens.cssbrain` | Tokens only, with no reset, so Bootstrap renders unchanged |

These adaptations satisfy the BRAINLLM checker without changing the DOM that the benchmark requires:

1. **Static ids are made unique per instance** (`id="run"` would render as `run-1`). The button ids are passed as a prop
   and rendered as `id={id}`, which is not rewritten. Solid uses the same component pattern.
2. **`on:click` on an `<a>` without `href` is B0402.** The benchmark mandates `<a>` for select and remove, so the
   anchors carry `role="button"`, plus `aria-label="Remove"` on the icon-only anchor. The DOM structure is unchanged.
3. **The id counter cannot live in a `.jsbrain` module** (B0801, mutable module-level state). It is a plain `let` in
   the view, and `buildData` takes the first id as a parameter.
4. **The page is wrapped in `<main class="container">`** instead of `div#main > div.container`, because a view needs
   one `<main>`. The runner does not select on `#main`.

## Toolchain findings (not worked around in crates/)

- **`brain check`/`brain doctor` miss `.jsbrain` errors that `brain build` enforces.** The project walk
  `brain_files_app` collects only `*.brain` (`crates/brain-cli/src/main.rs:322`), so B0801
  (`crates/brain-check/src/lib.rs:409`) is reported only when the module is checked directly or compiled by `build`.
  Minimal reproduction:
  ```
  // lib/next-id.jsbrain
  let nextId = 1
  export default function nextIdFn() { return nextId++ }
  // views/home.brain imports it; then:
  brain check --json        -> []
  brain doctor              -> "check 2 file(s) · 0 error(s)" ... "✓ no errors" (exit 0)
  brain build               -> error: lib/next-id.jsbrain does not compile: B0801 ...
  brain check lib/next-id.jsbrain --json -> [{"code":"B0801",...}]
  ```
  As a result, the SPEC loop ("done when `brain doctor` exits 0") can end with an app that does not build.
- `brain build` emits root-absolute asset URLs (`/_b/…`), and there is no base-path option. Serving from a sub-path
  needs the URL rebase that `build.sh` performs.
