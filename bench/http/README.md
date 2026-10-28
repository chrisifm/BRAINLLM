# HTTP client bench — `api()` under faults (QA #172, dev and prod)

Every behaviour of the HTTP client in `docs/INTERNALS.md` §40 and §41.4 is reproduced in real headless Chrome
against a fault-injection server, on the app in `app/`, served two ways:

- **dev**: `brain run --port 5198`, behind the fault server (one origin: the server proxies the app and answers the
  API itself, so `api()` calls stay same-origin);
- **prod**: `brain build`, with `dist/` served statically by the same fault server.

Each scenario asserts from **both** the DOM (each action prints its outcome as one JSON line in
`<output data-r="…">`) and the **server's request log** (count, method, status, timing, aborted, headers).

## Run

```sh
cargo build -p brain-cli
node bench/http/run.mjs                    # both modes, ~95 s (S06 waits for the 10 s timeout twice)
node bench/http/run.mjs --modes dev        # one mode
node bench/http/run.mjs --only S07,S12     # some scenarios (skips the C* checks)
node bench/http/run.mjs --no-build         # prod against the existing .work/app/dist
```

`BRAIN` overrides the binary (default `target/debug/brain`), `BRAIN_CHROME` the Chrome binary (`bench/lib/cdp.mjs`).
Chrome, `brain run` and the fault server are the runner's own children, stopped by their own handles on exit.

**Output:** a table on stdout, `results/summary.json` and `results/summary.md` (committed), and `results/raw/<mode>.json`
with the full server log and console lines per scenario (gitignored).
**Exit code:** `0` no FAIL (BLOCKED is allowed) · `1` any scenario FAILs · `2` the bench could not run.

## BLOCKED vs FAIL

A scenario whose feature is not compiled or wired yet reports **BLOCKED** with the reason, never FAIL:

1. The runner runs `brain check --json` on `app/`. Every file with an error is left out of the working copy
   (`.work/app`, gitignored); the scenarios that need that file are BLOCKED with the diagnostic
   (for example `B0203 views/error-states.brain:29 component <ErrorState> is not imported`). The rest still runs.
2. Wiring that `brain check` does not reveal is detected at runtime: if the served HTML of `/policy` still contains a
   literal `<meta http …>`, the app-wide policy was not compiled, and S19/S20 are BLOCKED.
3. Compiler checks (C1–C3) run `brain check` on probe files; a check that reports nothing is BLOCKED.

## The fault server (`server.mjs`, no dependencies)

`node bench/http/server.mjs --port 5197 [--upstream http://127.0.0.1:5196 | --static path/to/dist]` also runs it standalone.

| Path | What |
|---|---|
| `/f/…`, `/api/f/…` | the scripted API (the log keeps the raw path, so a `<meta http base>` prefix is visible) |
| `POST /__ctl/plan` | `{"GET /f/items": [answer, …]}`: answers consumed in order, the last one repeats |
| `GET /__ctl/log` | `{ peak, inflight, log: [{ seq, method, path, raw, query, t0, t1, status, aborted, reset, rid, bytes, ct, headers, inflight }] }` |
| `POST /__ctl/reset` | clears log, plan and the clock |
| anything else | the app (proxied to `brain run`, or static from `dist/`) |

An answer can also come from the header `x-fault: status=503;retryAfter=1` or the query `?_fault=status=503;delay=200`.
Answer fields: `status` · `json` · `body` · `headers` · `retryAfter` (sets `Retry-After`) · `delay` (ms) ·
`reset` (destroys the socket) · `hang` (never answers) · `readRate` (bytes/s while reading the body: a slow upload).
Every response carries `x-request-id` (`rid-<seq>`, or the client's own). Uploads answer
`{ ok, bytes, fields, files: [{ field, filename, size }] }`. The server closes every connection after one request:
Chrome silently re-sends a request whose reused keep-alive socket is reset, which would hide the client's own
retry policy (a reset POST reached the server 7 times before this).

## Scenarios

| # | Behaviour (INTERNALS) | How it is measured |
|---|---|---|
| S01 | 404/403 → `ApiError` `kind 'http'`, `status`, `message`, `id` = `x-request-id`, `e.is(403)` (§40.1) | DOM JSON vs server `rid`; 1 request each |
| S02 | 422 → `e.errors` | `{ email: 'Invalid email' }`; body sent as JSON |
| S03 | 429/503 with `Retry-After: 60` → `e.retryAfter = 60`, not retried (cap 10 s) | 1 request each |
| S04 | response not matching `out` → `kind 'schema'` | |
| S05 | connection reset → `kind 'network'`; GET retried twice, POST never | 3 GET / 1 POST at the server |
| S06 | no answer → `kind 'timeout'` at the policy timeout (default 10 s; 4 s once `<meta http timeout="4s">` is wired) | elapsed ms; server sees the abort |
| S07 | GET/PUT/DELETE retried on 502/503/504/network, backoff 300 ms then 600 ms ±25 % (§40.4) | gaps between attempts in the server log |
| S08 | POST and PATCH never retried | 1 request each |
| S09 | `Retry-After: 1` honoured on 429 and 503 | gap ≈ 1 000 ms |
| S10 | `.cancel()` rejects with `kind 'aborted'`, the author's `finally` runs; an uncaught cancel in a handler stays silent (#228) | DOM + server abort + silence probe |
| S11 | identical GETs in flight share one request | 2 components, 1 request |
| S12 | ≤ 6 per origin; a user action goes before waiting background requests (§41.4) | server `peak`; GETs that arrived before the POST |
| S13 | client navigation aborts background (prefetch) requests and keeps a user request | server aborts; a `window` marker proves the navigation was client-side |
| S14 | offline: GET from cache marked `stale`, write fails at once with `kind 'offline'`, nothing sent | CDP offline emulation; server log read while offline |
| S15 | unmount, and a resource re-run, abort the request silently (no `{:catch}`) | server aborts; silence probe |
| S16 | a write re-runs the resources of its path; `refetch()` forces a reload | GET count 3, the second after the POST |
| S17 | `in: 'file'` → multipart with `.progress` 0 → 1 | 8 MiB read at 4 MiB/s; DOM polled every 25 ms |
| S18 | an upload in flight can be cancelled (`kind 'aborted'`, `finally`, silent) | server sees the upload aborted mid-body |
| S19 | 401 → `login?next=<current path>` from `<meta http login>` | final `location` |
| S20 | `<meta http base>` prefixes `api()` paths | raw path at the server |
| S21 | `<ErrorState>` message + action per kind (§40.2) | text, retry button, request id |
| S22 | `online()` follows the connection state | CDP offline emulation |
| S23 | `api/_http.jsbrain` `http({ headers })` adds headers | header at the server |
| S24 | dev: every request in the `brain:net` feed and the `brain run` terminal; prod: none | |
| C1 | B1104 unknown attribute/value in `<meta http>` | `brain check` on a probe |
| C2 | warning for a direct `fetch()` (§40.5 calls it B1103, which is taken by "unknown type in api()"; the compiler uses B1107) | |
| C3 | B1106 `load()` with independent awaits | |

**Silence probe** (#228, used by S10, S13, S15, S18): an init script watches the whole document for any error
surface that appears: a toast with text (`.brain-toast`), an `<ErrorState>` or boundary fallback (`.error-state`,
`[role=alert]`), a `{:catch}` branch printing `error …`. It also counts unhandled rejections, console errors and new
error lines in the `brain run` output. An abort must produce none.

## Current results

Binary sha256 `d26b4bd21f64` (2026-10-13): **PASS 41 · FAIL 6 · BLOCKED 4 — FAIL** (exit 1), 93 s. Full table in
`results/summary.md`.

| Failing | Mode | Owner | Evidence |
|---|---|---|---|
| S16 refetch | dev + prod | COMPILER `crates/brain-codegen` | `list.refetch()` on a `resource()` compiles to `x.v.refetch()` (the signal is unwrapped before the method call): `TypeError: x.v.refetch is not a function`. The write-triggered re-run works. |
| S23 `_http.jsbrain` | dev + prod | TOOLCHAIN (+ COMPILER) | `http` now compiles in `api/_http.jsbrain`, and it is bundled, but no page imports it; the header never reaches the server. |
| S13 navigation | prod | TOOLCHAIN `brain-pack` | the router chunk imports `'./brain.js'`, which the build never emits (`dist/_b/brain.js` 404), so the router does not load: every link is a full page load and the background requests are not aborted. Dev passes. |
| S22 `online()` | prod | COMPILER/TOOLCHAIN (hydration) | `/online` (no event handler, only a reactive `online()` read) loads no view module in production, so the text never changes. Dev passes. |

BLOCKED: S19 and S20, because `<meta http …>` passes `brain check` (B1104 now validates it, see C1) but is emitted
verbatim into the HTML: no `http()` policy is generated (COMPILER #170 / TOOLCHAIN #171). Because of that, S06
measures the default 10 s timeout.
