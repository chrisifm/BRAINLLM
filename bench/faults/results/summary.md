# Fault injection bench — results

Binary sha256 `23b9532638cc` · 2026-10-26T08:57:43.840Z · total 72.6 s (build 2.1 s) · 0 failing cell(s) · verdict **PASS**

C1 isolation · C2 retry · C3 reported (dev: brain_errors with `comp/inner.brain:<line>`; prod: logs/app.log with rid) · C4 a failing handler keeps the component + toast · C5 dev compile error isolated and recovered by HMR.

| Fault | Mode | C1 | C2 | C3 | C4 | C5 |
|---|---|---|---|---|---|---|
| render (markup expression) | dev | PASS | PASS | PASS | n/a | n/a |
| render (markup expression) | prod | PASS | PASS | n/a | n/a | n/a |
| fx | dev | PASS | PASS | PASS | n/a | n/a |
| fx | prod | PASS | PASS | n/a | n/a | n/a |
| drv | dev | PASS | PASS | PASS | n/a | n/a |
| drv | prod | PASS | PASS | n/a | n/a | n/a |
| resource() without {:catch} | dev | PASS | PASS | PASS | n/a | n/a |
| resource() without {:catch} | prod | PASS | PASS | n/a | n/a | n/a |
| click handler (sync) | dev | PASS | n/a | PASS | PASS | n/a |
| click handler (sync) | prod | PASS | n/a | n/a | PASS | n/a |
| async handler (rejects after await) | dev | PASS | n/a | PASS | PASS | n/a |
| async handler (rejects after await) | prod | PASS | n/a | n/a | PASS | n/a |
| SSR render (server.mjs, ?boom=1) | dev | PASS | PASS | PASS | n/a | n/a |
| SSR render (server.mjs, ?boom=1) | prod | PASS | PASS | PASS | n/a | n/a |
| prerender (build time) | dev | PASS | PASS | PASS | n/a | n/a |
| prerender (build time) | prod | PASS | PASS | PASS | n/a | n/a |
| load() throws (?load=1) | dev | PASS | n/a | PASS | n/a | n/a |
| load() throws (?load=1) | prod | PASS | n/a | PASS | n/a | n/a |
| compile error in Inner (dev, HMR) | dev | n/a | n/a | n/a | n/a | PASS |

## Blocked

None.

## Failures

None.

## Evidence per cell

| Fault | Mode | Criterion | Evidence |
|---|---|---|---|
| render | dev | C1 | fallbacks ["in Middle: \"This section could not load. Retry\""]; siblings 6/6 count; nav ok |
| render | dev | C2 | remounted: "render-ok drv-ok res-ok inner 0 Throw in click handler Throw", inner counter 0→1; Retry with the fault still on → falls back again with Retry |
| render | dev | C3 | brain_errors: 2 entry; frames comp/inner.brain:6, comp/inner.brain:26 (want inner.brain:6 or 26); runtime brain-err has it as comp/inner.brain:6 ([object Object]) |
| render | prod | C1 | fallbacks ["in Middle: \"This section could not load. Retry\""]; siblings 6/6 count; nav ok |
| render | prod | C2 | remounted: "render-ok drv-ok res-ok inner 0 Throw in click handler Throw", inner counter 0→1; Retry with the fault still on → falls back again with Retry |
| fx | dev | C1 | fallbacks ["in Middle: \"This section could not load. Retry\""]; siblings 6/6 count; nav ok |
| fx | dev | C2 | remounted: "render-ok drv-ok res-ok inner 0 Throw in click handler Throw", inner counter 0→1 |
| fx | dev | C3 | brain_errors: 1 entry; frames comp/inner.brain:6, comp/inner.brain:10 (want inner.brain:6 or 10); runtime brain-err has it as comp/inner.brain:6 ([object Object]) |
| fx | prod | C1 | fallbacks ["in Middle: \"This section could not load. Retry\""]; siblings 6/6 count; nav ok |
| fx | prod | C2 | remounted: "render-ok drv-ok res-ok inner 0 Throw in click handler Throw", inner counter 0→1 |
| drv | dev | C1 | fallbacks ["in Middle: \"This section could not load. Retry\""]; siblings 6/6 count; nav ok |
| drv | dev | C2 | remounted: "render-ok drv-ok res-ok inner 0 Throw in click handler Throw", inner counter 0→1 |
| drv | dev | C3 | brain_errors: 1 entry; frames comp/inner.brain:6, comp/inner.brain:8 (want inner.brain:6 or 8); runtime brain-err has it as comp/inner.brain:6 ([object Object]) |
| drv | prod | C1 | fallbacks ["in Middle: \"This section could not load. Retry\""]; siblings 6/6 count; nav ok |
| drv | prod | C2 | remounted: "render-ok drv-ok res-ok inner 0 Throw in click handler Throw", inner counter 0→1 |
| resource | dev | C1 | fallbacks ["in Middle: \"This section could not load. Retry\""]; siblings 6/6 count; nav ok |
| resource | dev | C2 | remounted: "render-ok drv-ok res-ok inner 0 Throw in click handler Throw", inner counter 0→1 |
| resource | dev | C3 | brain_errors: 1 entry; frames comp/inner.brain:12 (want inner.brain:6 or 12); runtime brain-err has it as comp/inner.brain:12 ([object Object]) |
| resource | prod | C1 | fallbacks ["in Middle: \"This section could not load. Retry\""]; siblings 6/6 count; nav ok |
| resource | prod | C2 | remounted: "render-ok drv-ok res-ok inner 0 Throw in click handler Throw", inner counter 0→1 |
| click | dev | C1 | fallbacks 0; siblings 6/6 count; nav ok |
| click | dev | C3 | brain_errors: 1 entry; frames comp/inner.brain:6, comp/inner.brain:15 (want inner.brain:6 or 15); runtime brain-err has it as comp/inner.brain:6 ([object Object]) |
| click | dev | C4 | Inner kept; handler ran (inner counter 1), then counts 1→2; toast "Something went wrong. Try again." |
| click | prod | C1 | fallbacks 0; siblings 6/6 count; nav ok |
| click | prod | C4 | Inner kept; handler ran (inner counter 1), then counts 1→2; toast "Something went wrong. Try again." |
| async | dev | C1 | fallbacks 0; siblings 6/6 count; nav ok |
| async | dev | C3 | brain_errors: 1 entry; frames comp/inner.brain:6, comp/inner.brain:20 (want inner.brain:6 or 20); runtime brain-err has it as comp/inner.brain:6 ([object Object]) |
| async | dev | C4 | Inner kept; handler ran (inner counter 1), then counts 1→2; toast "Something went wrong. Try again." |
| async | prod | C1 | fallbacks 0; siblings 6/6 count; nav ok |
| async | prod | C4 | Inner kept; handler ran (inner counter 1), then counts 1→2; toast "Something went wrong. Try again." |
| ssr | dev | C1 | fallbacks ["in Middle: \"This section could not load. Retry\""]; siblings 6/6 count; nav ok |
| ssr | dev | C2 | remounted: "render-ok drv-ok res-ok inner 0 Throw in click handler Throw", inner counter 0→1 |
| ssr | dev | C3 | brain_errors: 1 entry; frames comp/inner.brain:6, comp/inner.brain:22 (want inner.brain:6 or 22); runtime brain-err has it as comp/inner.brain:6 (no comp) |
| ssr | prod | C1 | HTML 200, fallback in Middle true, Inner in HTML false; fallbacks ["in Middle: \"This section could not load. Retry\""]; siblings 6/6 count; nav ok |
| ssr | prod | C2 | remounted: "render-ok drv-ok res-ok inner 0 Throw in click handler Throw", inner counter 0→1 |
| ssr | prod | C3 | app.log: "component failed during SSR" rid cdfa950f-821d-4915-8aba-e4cd6ece81a0 comp comp/inner.brain where render |
| prerender | dev | C1 | fallbacks ["in Middle: \"This section could not load. Retry\""]; siblings 6/6 count; nav ok |
| prerender | dev | C2 | remounted: "render-ok drv-ok res-ok inner 0 Throw in click handler Throw", inner counter 0→1 |
| prerender | dev | C3 | brain_errors: 1 entry; frames comp/inner.brain:6, comp/inner.brain:22 (want inner.brain:6 or 22); runtime brain-err has it as comp/inner.brain:6 (no comp) |
| prerender | prod | C1 | HTML 200, fallback in Middle true, Inner in HTML false; fallbacks ["in Middle: \"This section could not load. Retry\""]; siblings 6/6 count; nav ok |
| prerender | prod | C2 | remounted: "render-ok drv-ok res-ok inner 0 Throw in click handler Throw", inner counter 0→1 |
| prerender | prod | C3 | brain build reported it |
| load | dev | C1 | HTTP 500; view shows "Something went wrong" (want the _error.brain page); layout nav kept, navigation ok |
| load | dev | C3 | dev log: [load] views/ssr.brain: Error: fault:load |
| load | prod | C1 | HTTP 200 (streamed route: headers sent before load(); logged as "load failed (streamed)" with the page reference 6d2bc56b-a0ac-4b4c-9fff-b8e6b577581b); view shows "Something went wrong" (want the _error.brain page); layout nav kept, navigation ok |
| load | prod | C3 | app.log: "load failed" rid 78f05d09-b803-41bf-a7b0-a0a3906c52f8 status 200, page reference 78f05d09-b803-41bf-a7b0-a0a3906c52f8 |
| compile | dev | C5 | brain check: B0113 comp/inner.brain:5 `hits.push()` mutates the sig in place and the view is not notified; on save: error box in Inner's place, Inner gone, 6/6 siblings count; fresh load while broken: box in Inner's place ("comp/inner.brain does not compile B0113 5:3 `hits.push()` mutates the "), 6/6 siblings count; after the fix: Inner back via HMR (no reload) |

Inner lines (comp/inner.brain): throw 6; call sites render 26, fx 10, drv 8, resource 12, click 15, async 20, ssr 22.
Raw per-mode data (every snapshot, click target, console line and the full server log): `results/raw/<mode>.json` (not committed).
