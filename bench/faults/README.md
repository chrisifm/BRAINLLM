# Fault injection bench — component isolation under failure (QA #176, dev and prod)

`docs/INTERNALS.md` §42 says every component is isolated by default: a failure stays inside the component where it
happened, the component shows a fallback with **Retry**, a failing handler keeps its component and shows a toast, the
error goes to `brain_errors` with its `file.brain:line`, SSR and prerender render the fallback into the HTML, and in
`brain run` a component that does not compile shows a box in its place while the rest keeps hot-reloading. `SPEC.md`
"Errors at runtime" promises the same to app authors. This bench injects a fault in every place listed there and
checks each promise in real headless Chrome, in both modes:

- **dev**: `brain run --port 5197`;
- **prod**: `brain build` + `brain start` (the app has a `load`, so `dist/server.mjs` runs under the supervisor).

## Run

```sh
cargo build -p brain-cli
node bench/faults/run.mjs                    # both modes, ~60 s
node bench/faults/run.mjs --modes dev        # one mode
node bench/faults/run.mjs --only render,ssr  # some faults
```

`BRAIN` names the binary to test (default `target/debug/brain`); it is first copied to the work dir
(`BRAIN_FAULTS_WORK`, default `~/tmp/brainllm-faults`) and fingerprinted (sha256, first 12 hex), so a concurrent rebuild
of `target/` cannot change what is tested. The app is copied there too (the compile-error case edits
`comp/inner.brain`; the repo copy is never touched) and removed at the end. `BRAIN_CHROME` sets Chrome
(`bench/lib/cdp.mjs`). Chrome, `brain run` and `brain start` are the runner's own children: the supervisor is stopped
with `brain stop`, anything left is killed only by the PIDs the runner spawned or read from `brain status --json`.

**Output:** a table on stdout, `results/summary.json` and `results/summary.md` (committed, home dir written as `~`), and
`results/raw/<mode>.json` with every snapshot, click target, console line and the full server log (gitignored).
**Exit code:** `0` every applicable criterion passes · `1` any FAIL · `2` the bench could not run.

## The app

`app/` is a regular BRAINLLM app (`brain check` returns `[]`). `Outer` → `Middle` → `Inner` are nested three levels;
`Outer` and `Middle` each have their own counter and a `Sibling` counter, and the view has a counter and a sibling too.
Faults are switched from the page through a store (`channels/faults.jsbrain`, buttons in `comp/controls.brain`, outside
`Outer`), so turning one off needs no reload. `comp/inner.brain` throws through one helper, `explode(where)`:

| Fault | How Inner fails | View |
|---|---|---|
| render | a markup expression: `{faults.render ? explode('render') : 'render-ok'}` | `/lab` |
| fx | `fx(() => { if (faults.fx) explode('fx') })` | `/lab` |
| drv | `drv(() => faults.drv ? explode('drv') : 'drv-ok')`, shown in the markup | `/lab` |
| resource | `resource()` returns a rejected promise; awaited as `{#await res}` with no `{:catch}`. B1101 forbids that, so the bench app turns it off in `brain.json` (`"lint": {"B1101": "off"}`, INTERNALS §20). If the compiler still rejects the file, the work copy gets a `{:catch}` so the other cases run, and this case is BLOCKED with the compiler's message (see Notes) | `/lab` |
| click | a sync click handler that counts, then throws | `/lab` |
| async | an async click handler that counts, awaits `tick()`, then throws | `/lab` |
| SSR | setup throws when the `load` prop `boom` is set (`/ssr?boom=1`) and `ssrHealed` is off: `server.mjs` renders it | `/ssr?boom=1` |
| prerender | the same setup throw with `ssr={true}`: `brain build` prerenders `/pre` with Inner failing | `/pre` |
| load | `load()` throws (`/ssr?load=1`); §42 keeps `_error.brain` for this case | `/ssr?load=1` |
| compile | dev only: `hits.push(1)` (B0113) is written into `comp/inner.brain` while `brain run` serves `/lab`, then the file is restored | `/lab` |

## Criteria

All clicks are real mouse events (CDP `Input.dispatchMouseEvent` at the element's centre), so anything covering the
page blocks them as it would block a person; the element on top is recorded.

| # | Criterion | Measured |
|---|---|---|
| C1 | Only that component shows its fallback; the rest keeps working | Inner's nodes are gone, exactly one fallback (`.brain-fallback`, `[role=alert]` or a dev error box) is on the page and it sits inside `Middle`; the view, Outer, Middle and the three siblings each count one click; the nav link reaches `/about`. Prod SSR/prerender also check the HTML: 200, fallback inside Middle, no Inner. Handlers: no fallback at all. `load`: the `_error.brain` page (500) inside the intact layout, navigation works. |
| C2 | Retry recovers it | The fallback has a Retry button; the fault is turned off, Retry is clicked, Inner is back with `render-ok drv-ok res-ok` and its counter counts. For render, Retry is also clicked once with the fault still on: it must fall back again, with Retry (2 failures < 3 in 10 s). |
| C3 | The error is reported | dev: an entry in `brain_errors` (the dev server's browser-error ring; `brain run` prints each one as `[browser] {…}`, the same ring the MCP tool reads) whose stack has `comp/inner.brain:<line>` with the real `.brain` line of the throw or of the call site. prod: SSR and `load` failures are in `logs/app.log` with the request id; a prerender fallback must be reported by `brain build`; client errors have no production sink (n/a). |
| C4 | A failing handler keeps its component | After the click: Inner is still there, no fallback, the handler's own count went to 1, the Inner counter still counts, and a toast is visible. |
| C5 | Dev compile error is isolated and HMR recovers | On save: no full-page overlay, an error box in Inner's place, the six counters still count. Fresh load while broken: the box (with the code and the fix) in Inner's place, the counters count. After restoring the file: Inner is back without a manual reload. |

Owners: RUNTIME `runtime/brain-guard.js` (fallback, Retry, handler toast) and `runtime/brain-err.js` (capture);
COMPILER (guard wrapping in codegen); TOOLCHAIN (`crates/brain-serve`: dev overlay, `brain_errors`); BUILD
(`runtime/server/server.mjs` SSR and log, `crates/brain-pack/src/prerender.rs`).

## Current results

**Run of 2026-10-13 (task #286), binary `e9453256ff44`: FAIL, 1 failing cell, 1 case BLOCKED** (190 s on a loaded
machine). Full evidence per cell: `results/summary.md`.

| Fault | dev C1 | dev C2 | dev C3 | dev C4 | prod C1 | prod C2 | prod C3 | prod C4 |
|---|---|---|---|---|---|---|---|---|
| render | PASS | PASS | PASS | n/a | PASS | PASS | n/a | n/a |
| fx | PASS | PASS | PASS | n/a | PASS | PASS | n/a | n/a |
| drv | PASS | PASS | PASS | n/a | PASS | PASS | n/a | n/a |
| resource without `{:catch}` | BLOCKED | BLOCKED | BLOCKED | BLOCKED | BLOCKED | BLOCKED | BLOCKED | BLOCKED |
| click handler | PASS | n/a | PASS | PASS | PASS | n/a | n/a | PASS |
| async handler | PASS | n/a | PASS | PASS | PASS | n/a | n/a | PASS |
| SSR | PASS | PASS | PASS | n/a | PASS | PASS | PASS | n/a |
| prerender | PASS | PASS | PASS | n/a | PASS | PASS | PASS | n/a |
| load | PASS | n/a | PASS | n/a | **FAIL** | n/a | PASS | n/a |
| compile error (C5, dev) | PASS | | | | | | | |

- **resource: BLOCKED.** `brain build` rejects `comp/inner.brain` with B1101 although `brain.json` has
  `"lint": {"B1101": "off"}` (see Notes).
- **load, prod, C1: FAIL (BUILD, `runtime/server/server.mjs` streamed path).** When `load()` throws on a streamed
  route, the server logs `load failed (streamed)` and writes the `_error.brain` markup, but the page's boot module
  still imports and mounts the route's own view over it: in the browser the `<h1>` reads "Something went wrong" for
  a moment, then "Server render" (the normal view, without its data), with HTTP 200. Repro: build this app, run
  `PORT=5393 node dist/server.mjs`, open `/ssr?load=1` and read the `<h1>` after 300 ms and after 2 s.

### Previous run (binary `a005208f73ce`)


Binary sha256 `a005208f73ce` (2026-10-13), Node v26.8.1, macOS arm64: **FAIL**, 12 failing cells. Three runs (61–63 s
each, on three successive binaries of a tree other sessions were rebuilding) gave identical verdicts. Full evidence per
cell: `results/summary.md`.

| Fault | dev C1 | dev C2 | dev C3 | dev C4 | prod C1 | prod C2 | prod C3 | prod C4 |
|---|---|---|---|---|---|---|---|---|
| render | PASS | **FAIL** | **FAIL** | n/a | PASS | **FAIL** | n/a | n/a |
| fx | PASS | PASS | **FAIL** | n/a | PASS | PASS | n/a | n/a |
| drv | PASS | PASS | **FAIL** | n/a | PASS | PASS | n/a | n/a |
| resource without `{:catch}` | PASS | PASS | **FAIL** | n/a | PASS | PASS | n/a | n/a |
| click handler | PASS | n/a | **FAIL** | PASS | PASS | n/a | n/a | PASS |
| async handler | PASS | n/a | **FAIL** | PASS | PASS | n/a | n/a | PASS |
| SSR | PASS | PASS | **FAIL** | n/a | PASS | PASS | PASS | n/a |
| prerender | PASS | PASS | **FAIL** | n/a | PASS | PASS | **FAIL** | n/a |
| load | PASS | n/a | PASS | n/a | PASS | n/a | PASS | n/a |
| compile error (C5, dev) | **FAIL** | | | | | | | |

What works: isolation holds everywhere (one fallback, inside Middle; six counters and navigation keep working in every
case and mode); Retry recovers every boundary fault, including an SSR or prerendered fallback in production; both
handler kinds keep Inner and show the toast "Something went wrong. Try again."; SSR answers 200 with the fallback in
the HTML and logs `component failed during SSR` with `rid` and `comp/inner.brain`; a failing `load` renders
`_error.brain` with status 500 and the same rid in the page and in `logs/app.log`.

### Open failures

1. **Retry while the render fault is still on leaves the fallback and a half-built Inner side by side (RUNTIME,
   `runtime/brain-guard.js` `build`/`again`), dev and prod.** Probable cause (read from the code, not instrumented): the throw comes from a markup
   binding during `C($p)`, so the boundary's error handler (`fall`) runs inside `build()`: it inserts the fallback, then `build()` continues,
   assigns `nodes = I.nodes(C($p))` and `again()` inserts those too. The page shows `This section could not load.
   Retry` next to a dead Inner (`res-ok inner 0 Throw in click handler …`). Expected: the fallback alone, with Retry
   (2nd failure).
2. **No boundary-caught error reaches `brain_errors` (TOOLCHAIN `crates/brain-serve/src/client.js` + RUNTIME
   `runtime/brain-err.js`), dev, all 8 client faults.** `client.js` forwards only `window` `error` and
   `unhandledrejection`; the boundary catches render/fx/drv/resource/handler/setup errors and only calls
   `console.error`, and `brain-err`'s `errors` list stays empty without devtools. So `brain_errors` (and `brain run`'s
   terminal) shows 0 entries for every fault. Second gap for the same criterion: even an uncaught error would carry
   generated JS lines, not `.brain` lines — the throw at `inner.brain:6` appears as `inner.brain:14`, the click call
   at line 15 as `:25` (the dev source map is applied only by brain-err, which never runs here).
3. **A full-page overlay on save covers the whole app (TOOLCHAIN `crates/brain-serve/src/client.js` `overlay()` +
   `server.rs` HMR `type:"error"` broadcast), dev.** After a save with B0113 in Inner, `#__brain_overlay`
   (`position:fixed; inset:0`) is on top of every counter: all six real clicks land on the overlay. The per-component
   error box exists, but only on a fresh load (`serve_brain`), where it works: box in Inner's place with code and fix,
   six counters count. After the fix the page comes back through a full reload (the error module has no component
   hash, so `hot()` reloads); acceptable, but the state of the rest of the page is lost.
4. **`brain build` is silent about a component that fell back during prerender (BUILD
   `crates/brain-pack/src/prerender.rs`).** `/pre` is shipped with the fallback in its HTML (correct per §42), but the
   build prints no warning and exits 0, so nobody learns that a page was prerendered broken; `server.mjs` logs the
   same case at request time.

### Notes

- B1101 now also catches `{#await res ?? null}` (QA #176), which this bench used to test the uncaught case. The bench
  app now opts out the documented way, `"lint": {"B1101": "off"}` in `brain.json`. On 2026-10-13 that override is
  honored by `brain check` (no diagnostic) but not by the compile step: `brain build` and `brain run` still reject
  `comp/inner.brain` with B1101, so the resource case is BLOCKED until COMPILER applies `lint` in the compiler too. The
  harness checks this on every run (a probe build of a copy) and runs the real case by itself once the build accepts
  the file.
- The async handler's toast is the generic line; §42 uses the `ApiError` message only for HTTP errors, which this
  bench does not inject (bench/http covers `api()`).
