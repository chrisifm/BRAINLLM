# HTTP client bench — results

Binary sha256 `4cca4e74f4c3` · 2026-10-13T15:19:30.687Z · total 70.7 s · PASS 51 · FAIL 0 · BLOCKED 0 · verdict **PASS**

| # | Scenario | dev | prod | Owner | Evidence |
|---|---|---|---|---|---|
| S01 | HTTP status → ApiError kind http, status, message, x-request-id, is(403) | pass | pass | RUNTIME runtime/brain-data.js | dev: 404 http/404 id rid-1; 403 is(403)=true; requests 1+1 |
| S02 | 422 → e.errors per field | pass | pass | RUNTIME runtime/brain-data.js | dev: 422 errors {"email":"Invalid email"} |
| S03 | 429/503 with Retry-After > 10 s → not retried, e.retryAfter | pass | pass | RUNTIME runtime/brain-data.js | dev: 429 retryAfter 60 · 503 retryAfter 60 id rid-2; 1 request each |
| S04 | response not matching `out` → kind 'schema' | pass | pass | RUNTIME runtime/brain-data.js | dev: kind schema: GET /f/typed: the response does not match its schema: value: expected a list |
| S05 | connection reset → kind 'network'; GET retried ×2, POST never | pass | pass | RUNTIME runtime/brain-data.js | dev: POST network ×1 · GET network ×3 |
| S06 | no answer → kind 'timeout' at the policy timeout (default 10 s) | pass | pass | RUNTIME runtime/brain-data.js | dev: kind timeout after 4002 ms (policy 4000 ms from <meta http>) |
| S07 | safe methods retried on 502/503/504/network with 300 ms then 600 ms (±25 %) | pass | pass | RUNTIME runtime/brain-data.js | dev: GET 502→504→200 backoffs 234/591 ms · PUT ×2 · DELETE ×2 |
| S08 | POST and PATCH are never retried | pass | pass | RUNTIME runtime/brain-data.js | dev: POST 503 ×1 · PATCH 502 ×1 |
| S09 | Retry-After (≤ 10 s) honoured on 429 and 503 | pass | pass | RUNTIME runtime/brain-data.js | dev: Retry-After 1 s → waited 1002 ms (429) and 1008 ms (503) |
| S10 | .cancel() rejects with kind 'aborted', finally runs, nothing else surfaces | pass | pass | RUNTIME runtime/brain-data.js · runtime/brain-guard.js | dev: catch "caught aborted", finally yes; uncaught cancel silent (0 surfaces); server aborted true/true |
| S11 | identical GETs in flight share one request | pass | pass | RUNTIME runtime/brain-data.js | dev: 2 components, 1 request(s) |
| S12 | max 6 per origin; a user action goes before waiting background requests | pass | pass | RUNTIME runtime/brain-data.js | dev: peak 6; action after 6 background GETs; settle order low0,low1,low2,low3,low4,low5,act,low6,low7,low8,low9 |
| S13 | client navigation aborts background (prefetch) requests, keeps user ones | pass | pass | RUNTIME runtime/brain-data.js | dev: client nav=true; background 3/3 aborted; user request 200 |
| S14 | offline: GET answers from cache (stale), write fails at once, nothing sent | pass | pass | RUNTIME runtime/brain-data.js | dev: read {"ok":true,"stale":true,"n":2,"ms":0}; GET times 31 ms; write offline in 1 ms; server saw GET×1 POST×0 |
| S15 | unmount and re-run abort the resource request silently ({:catch} not rendered) | pass | pass | RUNTIME runtime/brain-data.js | dev: unmount aborted=true; re-run aborted→200; error surfaces 0 |
| S16 | a write re-runs the resources of its path; refetch() forces a reload | pass | pass | RUNTIME runtime/brain-data.js | dev: GETs 3 (mount, after write, refetch); final "items 3" |
| S17 | upload (in: 'file'): multipart with .progress 0→1 | pass | pass | RUNTIME runtime/brain-data.js | dev: 17 progress values (15 between 0 and 1), 2.1 s, 8388898 bytes |
| S18 | an upload in flight can be cancelled (kind 'aborted', finally runs, silent) | pass | pass | RUNTIME runtime/brain-data.js | dev: "caught aborted", finally yes, aborted at 1113383 bytes |
| S19 | 401 → login?next=<current path> (from <meta http login>) | pass | pass | RUNTIME runtime/brain-data.js | dev: → /login?next=%2Flogin-gate |
| S20 | <meta http base> prefixes api() paths | pass | pass | COMPILER crates/brain-codegen (<meta http>) · TOOLCHAIN | dev: raw path /api/f/policy |
| S21 | <ErrorState> renders message + action per kind | pass | pass | RUNTIME runtime/ui/error-state.brain · COMPILER (kit registration) | dev: network: "The connection was lost. Retry" +retry · forbidden: "You do not have permission: orders.read" · missing: "Not found." · invalid: "Check the highlighted fields." · limited: "Too many requests. Trying again in 60 se" · server: "Something went wrong on our side. Retry " +retry |
| S22 | online() follows the connection state | pass | pass | RUNTIME runtime/brain-data.js · COMPILER (scope) | dev: online → offline → online |
| S23 | api/_http.jsbrain http({ headers }) adds headers to every request | pass | pass | COMPILER (http in scope) · TOOLCHAIN (loading _http) | dev: x-bench=from-http-headers |
| S24 | request feed: dev logs every api() call; prod emits none | pass | pass | TOOLCHAIN crates/brain-serve | dev: page events 1; terminal: [net] GET /api/f/k404 404 1 ms 21 B |
| C1 | B1104: unknown attribute or value in <meta http> | pass | pass | COMPILER crates/brain-check | B1104 app.brain:2 `timeout="soon"` is not valid in `<meta http>` · B1104 app.brain:2 `<meta http>` has no attribute `bogus` |
| C2 | warning: fetch() used directly in app code (INTERNALS §40.5 names it B1103) | pass | pass | COMPILER crates/brain-check | B1107 views/fetchy.brain:4 `fetch()` in app code skips typed errors, safe retries, dedupe, cancel on unmount and caching |
| C3 | B1106: load() with independent awaits (waterfall) | pass | pass | COMPILER crates/brain-check | B1106 views/waterfall.brain:6 `load` awaits 2 independent requests one after another |

Every source file compiles.

Raw per-mode data (server request logs, console lines): `results/raw/<mode>.json` (not committed).
