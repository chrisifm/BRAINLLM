# Release and continuation checklist

Updated: 2026-10-30. This is the resumption entry point when a session loses context or tokens.
Implementation, passing source tests and published packages are separate states. Preserve the original checkout,
index and unrelated work. Never move the published tag or blindly retry a publisher.

## Current priority

1. Finish native socket end-to-end integration and the remaining next-development acceptance checks below.
2. Integrate reviewed changes in an isolated next-version branch and run the required complete gates.
3. Preserve the verified 0.4.0 release, public notes and original checkout.

## Published checkpoint and immutable evidence

- **All 11 npm packages are published at 0.4.0.** Exact public SHA-512 integrity matches every approved tarball;
  all 11 registry records advertise provenance. The publisher job succeeded in
  [41115493750](https://github.com/chrisifm/BRAINLLM/actions/runs/41115493750).
  Its original post-publication checker failed; this does not mean publication failed. Do not republish.
- Immutable tag `v0.4.0`: `a3c240200c25febcf8dde3cfccb3de37dd0b4987`.
  Approved package archive: [41113503223](https://github.com/chrisifm/BRAINLLM/actions/runs/41113503223),
  all seven native recovery gates and all 11 package archives passed. Eleven tarball and 35 native/sidecar
  hashes were independently checked. Full packaging verification 41113508118 passed.
- Production source f1608c622586e349fd858cf3fbf3bcf6cddadbbf and 8283d5270827c6bd9725aba24971691a12098fba
  have identical compiled inputs; only two unembedded notes differ. Preserve actual binary source attribution.
- Source gates: 837 Rust tests, 418 runtime tests, 53 exact invalid fixtures and four fixture-oracle tests;
  [full verification 41103502428](https://github.com/chrisifm/BRAINLLM/actions/runs/41103502428) passed.
- Corrected macOS ARM64 binary SHA-256: `c5a264db5930a7166cfd56c91a597ce61867df74337fbd8e98ae35f6a16add3e`.
  Native smoke and actual Viewer/headless replay passed three scenarios each with full AX replay;
  heap collection was not requested. Fresh matched LLM cohorts passed 20/20 before and 20/20 after.
- Original seven-platform run 41106889626 passed all native smokes, but Windows notices failed because checkout
  changed pinned license LF bytes to CRLF. Exact-hash canonicalization and nine regressions fixed packaging;
  artifact recovery preserved source/toolchain provenance without rebuilding all binaries.
- CI-only repairs are on `fix/registry-smoke-0.4.0`, not the immutable published tag. The checker now accepts the
  actual client-rendered development shell, uses portable test paths and reports safe structured failure stages.
- Public verification 41119484124 passed Windows and macOS. Ubuntu failed Chrome startup after CI introduced
  a long temporary directory. A differential Linux reproduction proved the cause: the same Chromium and flags
  passed with a 14-byte temporary path, but a 70-byte temporary path aborted with "Socket path is too long".
  The repaired workflows create a short, random, owned Linux temporary parent and remove only that parent.
- Final verification: [41120774001](https://github.com/chrisifm/BRAINLLM/actions/runs/41120774001),
  CI repair commit 4ab8258e40ec2f12331753191979e322b10637e7. All three hosted install jobs passed; actual success JSON was independently verified for both initializers on each system.
  A clean Linux ARM64 Docker install already passed both public creators, check/build/doctor with actual browser
  tests and development HTTP using the explicit browser selection. No compiler rebuild or republish is required.
- CI helper regressions: 11/11 passed. Publisher guard: ten cases passed, including canceled tag/manual jobs,
  manual publish=false on a tag and failed native recovery. YAML, English and diff checks passed.
- Review proposal: [PR #10](https://github.com/chrisifm/BRAINLLM/pull/10), targeting the reviewed release branch.
  No automatic merge into main. Full source verification 41120330255 passed on the final helper source; all three hosted install jobs in 41120774001 passed after the workflow-only path correction.
- Full verification 41118057666 exposed an intermittent atomic-change lock lifecycle failure. The next-development integration now contains explicit lock release with focused regression coverage.
- Rust-powered descriptions and relevant frontend/backend/CLI/framework keywords are already public.
  Directory category placement still needs the screenshot site's actual URL and classification rules.

### Publication acceptance

- [x] Freeze the checkpoint and detailed English 34-point inventory in `CHANGELOG.md`.
- [x] Corrected Rust workspace: 837 passing tests, including the separately executed browser lifecycle test.
- [x] Runtime: 418 passing tests.
- [x] Diagnostic fixtures: 53 exact fixture comparisons; warning-boundary oracle: four passing tests.
- [x] Corrected-source full GitHub verification, including actual Linux browser storage and URL-state checks.
- [x] Matched LLM comparison: 0.3.1 **20/20**, 0.4.0 **20/20**, no fixes, no infrastructure failures;
  stable binary/spec/token hashes. Gate and comparison passed without overwriting historical baselines.
- [x] Actual release-binary Viewer and headless replay: three scenarios each, no fallback or reported errors.
  Heap collection was not requested; do not claim a heap proof from that run.
- [x] Public-source/history privacy checks, secret scan, literal embedded-source inventory and English gate.
- [x] Corrected macOS native smoke and package-generator tests; earlier real Cargo/license sidecar and tarball audit.
  Final distribution binary/sidecar hashes must be checked again by the publication matrix.
- [x] Linux x64 final distribution build, smoke, notices and artifact (run 41106889626).
- [x] Linux ARM64 final distribution build, smoke, notices and artifact (run 41106889626).
- [x] macOS ARM64 final distribution build, smoke, notices and artifact (run 41106889626).
- [x] macOS x64 final distribution build, smoke, notices and artifact (run 41106889626).
- [x] Linux x64 musl final distribution build, smoke, notices and artifact (run 41106889626).
- [x] Windows x64 recovered executable, repeated native smoke, exact-toolchain notices and artifact (41113503223).
- [x] Windows ARM64 recovered executable, repeated native smoke, exact-toolchain notices and artifact (41113503223).
- [x] Pack all seven platforms and all 11 exact-version npm packages in CI (41113503223).
- [x] Verify all 11 public registry versions are 0.4.0; matching integrity and provenance confirmed. Never infer this from a tag, local pack or started job.
- [x] Anonymous exact-version installation passes on Linux (clean ARM64 Docker), macOS and Windows (hosted CI), using both initializers. Windows browser testing is explicitly unsupported.
- [x] Confirm all three repaired hosted install jobs in 41120774001; actual success reports verified independently.
- [x] Publish the complete English GitHub release: https://github.com/chrisifm/BRAINLLM/releases/tag/v0.4.0 (2026-10-30 20:09:32 UTC).
- [x] Update the local HTML report to published, including the successful three-system hosted install verification.
- [x] Record final platform durations, actual publication outcome and public release/review links here.

## Measured platform durations

Final build run 41106889626, one observation, not historical averages. Whole jobs include compilation, smoke,
notices and artifact handling; exclude queue wait. All native smokes passed. Windows jobs failed at notices,
then successful recovery run 41113503223 completed exact-toolchain notices and repeated native smoke.

| Platform | Compiler-reported release build | Whole original job | Original outcome |
|---|---:|---:|---|
| Linux x64 | 5m 05s | 5m 25s | Success |
| Linux ARM64 | 4m 23s | 4m 40s | Success |
| Linux x64 musl | 5m 28s | 6m 16s | Success |
| macOS ARM64 | 6m 45s | 7m 17s | Success |
| macOS x64 | 14m 29s | 15m 45s | Success |
| Windows x64 | 8m 27s | 9m 56s | Notices failed after smoke |
| Windows ARM64 | 7m 36s | 9m 00s | Notices failed after smoke |

## What the checkpoint includes

Use the **34 numbered points in CHANGELOG.md 0.4.0** as the detailed feature inventory and landing-copy source.
Read their qualifications; these are not claims of universal inference, browser compatibility or zero memory leaks.
The inventory covers:

- [x] Project-owned Design System, editable UI, component gallery/manifests and mobile-first guidance.
- [x] Guarded upgrades, design adoption, scoped CSS reuse and conservative production cleanup with feedback.
- [x] Deterministic pruning, bounded caches, eligible zero-JS pages, predictive navigation and static compression.
- [x] Fragment HMR, URL-state primitives, structured diagnostics and agent feedback/discovery/quality tools.
- [x] Revision-checked concurrent file edits, hooks, durable queues and project history tools.
- [x] PostgreSQL foundation, resource contracts, policy checks, efficient indexed search and links/slugs.
- [x] JWT/revocable sessions, account flows, Google sign-in templates and safe production error surfaces.
- [x] Validated media, renditions, optional storage adapters, native HTTP/API and request-scoped resource access.
- [x] Form validation, runtime budgets, browser storage, workers and optional SQLite Wasm/OPFS integration.
- [x] Official SDK adapters/selective ESM integration, PM2/nginx templates and agent-oriented E2E tooling.

## Next-development implementation checkpoints

Version: **0.4.1-dev.0**, branch `development/0.4.1`, verified code checkpoint `3987d0b` (additional documentation/CI prerequisites tracked separately). Everything below is local development unless explicitly
linked to a published release. npm remains 0.4.0. Candidate tests and an integrated complete gate are distinct.

- [x] Atomic state/preview/writer leases release explicitly: 15 focused tests passed, including retained-descriptor
  contention and recovery. Source 5277d43 integrated locally as a72edc1.
- [x] Source-marker literal corruption fixed with the existing framework lexer, one code-generation pass and
  explicit generated-location diagnostics. Regex and HMR companion cases covered. Candidate 34/34 tests passed;
  snapshot goldens were not edited. Source ee017de integrated as 9bed977.
- [x] Opt-in administrative CRUD implemented with project-owned DS, guards, CAS/leases and collision refusal.
  Plain-store specialization preserves full fallback and serialization semantics. Candidate runtime 421/421,
  compiler/import checks 16/16 and administrative cases 5/5 passed. Sources 1c513e8 + 4d8e43f + 3fc11f6 integrated
  as 880f8e1 + bc968ee + fc8aafd. The fixed 45-byte project-name fixture reaches **30,720 B exactly: no headroom**.
  Do not claim that arbitrary names, fields, themes or layouts fit that reference budget.
- [x] Doctor preserves independent fresh SEO evidence after a payload-budget failure while keeping build/overall
  failure. It does not audit stale dist after source errors, raise the configured limit or present absent tests
  as a measured zero-leak result. Integrated CLI regression passed on the combined tree; sources 38166e3 + 5020d9c.
- [x] Future publication requires successful same-commit source verification, including approved archive reuse.
  Cancellation, manual pack-only dispatch and failed native recovery remain fail-closed; three policy tests passed.
  Sources 553d91e + 8150927. Reusable-call execution still belongs to the final complete CI acceptance.
- [x] Native backend socket component implemented opt-in, with row/role/session revalidation, bounded admission,
  fixed metadata, backpressure and graceful shutdown. Seven unit and four actual PostgreSQL/TCP tests passed
  (PostgreSQL 18.6). Source fb48cf8 integrated as 171b94b; CLI defaults remain disabled through e06faf5.
- [x] Node Upgrade bridge component completed and reviewed: fixed endpoint, filtered credentials, delegated Origin,
  bounded queues/writes/lifetime, coexistence with other Upgrade handlers and reconnect after closeAllConnections.
  Its 89 Node tests passed. Sources 5906096 + b5d6c5f are integrated as 8c6434a + fd52ab6; Rust embedding checks remain required.
- [x] Rust development/static socket gateway component integrated as d03d2fc from 75bb2c0. All 64 component
  library tests passed, including opaque TCP, HTTP/HMR and managed shutdown. Direct empty-context environment
  regression added as d3fddb8; this additional regression still requires execution on the integrated tree.
- [x] Scoped liveResource client, compiler registration, strict shared socket settings, CSP and optional private
  manifest generation integrated as 0a411f9 from 4aa7b74. Candidate runtime 433/433 and focused Rust 87/87
  passed (20 compiler, 63 packaging, four configuration). Unused applications retain identical asset bytes.
  These are component results; combined acceptance remains required. Clock rollback throttle regression fixed
  as a67c32a from e699338; candidate live/GC tests 13/13 passed.
- [x] Strict Node manifest activation integrated as 72df6f3 from 48b2168. Candidate 19/19 tests passed, covering
  true/false overrides, absent defaults, malformed flags and the required backend.
- [x] Shared CLI development/test/static startup integrated as b440f9a from 86699c0. Candidate five CLI and 65 serve
  tests passed. MCP now uses the same project startup helper (3987d0b); combined compilation remains required.
- [x] Deployment integration afc4ded from 7661924: eight focused Rust tests, six real nginx TLS/TCP checks and
  four actual PM2 configuration parser checks passed; compiler-rendered nginx hashes match tested templates.

All new Linux results above use **Rust 1.96.1 diagnostic tooling** unless noted otherwise. They are not the final
Rust 1.99.1 release gate and do not establish cross-platform execution or whole-application memory coverage.

## Remaining acceptance before freezing the next version

- [x] Integrate Node bridge sources 5906096 + b5d6c5f locally (8c6434a + fd52ab6).
- [x] Run actual pack/private-helper/dev-sidecar Rust embedding checks in the combined 3987d0b suite.
- [x] Integrate the completed Rust development/static gateway component (d03d2fc).
- [x] Execute the added empty-context environment oracle and all 65 serve tests on the combined tree. No ambient
  process environment may be added to ctx.env when its loader is absent.
- [x] Integrate the scoped liveResource client/compiler/configuration/packaging component (0a411f9).
- [x] Integrate CLI development/test/static startup, Node manifest consumers and MCP project-startup callsite.
- [x] Compile the shared CLI/MCP consumers together and pass startup tests, including selected-port Origin
  preflight and disabled defaults. Actual MCP/browser/native network behavior remains a distinct end-to-end gate.
- [x] Wire one reviewed brain.json backend.origins/sockets declaration into backend startup, private build manifest,
  Node SSR and Rust development/static gateways. Disabled defaults and explicit overrides have focused coverage.
- [ ] Validate that shared declaration end to end in an actual native/browser application.
- [x] Candidate nginx deployment templates passed six actual parser/TLS/TCP checks: four disabled topologies and
  two socket-enabled Rust topologies; query/method/protocol/body rejection and credential projection included.
- [x] Integrate deployment sources and verify compiler-rendered PM2/nginx configurations against tested templates.
  Node routing preserves its credential projection and bridge lifecycle; it does not rename cookies.
- [x] Run six actual compiled browser/native PostgreSQL cases on verified d0c7daf binary (Rust1.99.1). It used
  explicit owned headless fallback, preserved user Viewer profiles and did not claim a new AX/whole-heap proof.
- [ ] Re-run appropriate final binary/deployment acceptance after the separate thread-admission correction.
- [x] Re-run administrative budget/required/no-JS cases on combined checkpoint 3987d0b. Keep the 30 KiB reference
  gate; fixed edit is 30,720 B exactly. Rerun when subsequent compiler/runtime changes affect emitted output.
- [ ] Investigate source-map performance before any compiler-speed claim: seven paired diagnostic DEBUG runs showed
  large-prod **72.91 to 148.55 ms (+103.74%)**, despite the functional correction. Release-profile paired measurement
  completed with the same diagnostic toolchain: seven paired release runs measured large-prod **12.59 to 20.57 ms
  (+63.34%)** and simple-dev +24.99%. The material regression remains open; do not claim a global compiler speedup.
- [ ] Run final workspace/runtime/fixture/source-input/privacy/English gates on the combined source and current
  stable Rust 1.99.1, plus a fresh frozen LLM gate for author-facing SPEC/CLI changes. No next-version publication yet.
- [ ] Keep the original main checkout/index and all unrelated sessions intact; commits/push/review do not imply a main merge.

## End-to-end findings under active correction

- [x] Reproduce the actual B1102 mismatch: a documented GET `cache: 'no'` endpoint is rejected by the checker,
  although the runtime implements it. The private fullchain fixture uses the native SDK and did not bypass it.
- [x] Implement the narrow checker correction and regression in f2287f6. An explicit GET opt-out is retained even
  when global data caching is off; invalid values and write-method cache options remain rejected.
- [ ] Execute that checker regression, fresh CLI and complete native/browser fixture after integration. The old
  3987d0b binary still contains the reproduced rejection and must not be reused as the corrected binary.
- [x] Correct and prove development default-value-module errors (41c7fae from b6b80b8). A real ESM regression
  failed before the fix; 92/92 focused Rust 1.99.1 cases passed afterward, including 20 checker library, six worker
  and 66 serve cases. Original diagnostics, pre-HTTP persistent feedback, component fallback and reload/recovery
  remain intact. Static named-import linking can still precede module evaluation and relies on the feedback spool.
- [x] Execute the f2287f6 no-cache checker regression in that stable-toolchain focused batch.
- [x] Integrate span-only marker scanning as 8ea262a from 32eaae9; 44 focused stable-toolchain tests passed.
  Token-payload allocations dropped from 48,800 to one and full scanning from 48,813 to 14 for the measured
  571 KiB fixture. Allocation counts are not retained heap or a global timing result; final paired speed measurement
  and the fresh combined native binary remain pending.
- [x] Complete actual compiled-app/PG/browser acceptance: six cases passed after the checker/module corrections.
  This is a synthetic isolated application proof; the requesting application's data/backend were not migrated.

### Actual browser/native acceptance progress

Fresh compiler source d0c7daf, Rust 1.99.1, binary SHA-256
`c08686d7a2ba3f8bd96bb152fdd5acee738f90dc5f3095f9525ffd60a649d978`.
The fixture and gate scripts have independent fingerprints. Owned headless Chromium is an explicit fallback
because multiple Viewer profiles were connected without an assigned profile/tab; user sessions were preserved.

- [x] Case 1: real compiled API SDK, HttpOnly JWT, fixed dev gateway, room join and owner-isolated DOM passed.
- [x] Case 2: anonymous HTTP/socket denial without private data; exact native owner-read policy 403 proved.
- [x] Case 3: committed create/update/delete refresh and cross-owner metadata isolation.
- [x] Case 4: missed-change resynchronization and explicit reconnect generation.
- [x] Case 5: principal switching clears private state and closes the old generation.
- [x] Case 6: session revocation and actual owner disposal without destroying the browser document.
- [x] Attribute the intermittent setup exit: cgroup peak256/max256, two limit hits, OS thread errno11 and zero
  OOM kills. The successful finite512-PID run reached262 with zero limit hits; CPU/memory/assertions unchanged.
- [x] Integrate graceful dev thread-admission handling as a8f7be1 from 52c2643: fallible startup workers,
  stack-owned pending request, best-effort503, permit recovery and coalesced diagnostics. Three deterministic
  regressions and all69 serve tests passed on Rust1.99.1; the admission cap remains64.
- [ ] Build a new pinned binary with that guard and the exposed live SPEC, then rerun final native and full gates.
  The passing d0c7daf binary predates these final changes and must retain its actual provenance.
- [x] Failed attempts disposed all three owned contexts/children and confirmed removal of owned Docker resources.
  The final passing run also proved zero remaining app database connections; no requesting application was migrated.

## Additional measured or explicit gaps

- [ ] Linearize repeated DOM sibling paths where ownership permits: a stress fixture with 300 bindings/siblings
  emits about 586 KiB through repeated nextSibling chains. Preserve fragment/list/HMR semantics and measure bytes,
  browser execution and compilation before claiming a speedup.
- [ ] Audit JWT access renewal before socket reconnection. Upgrade must not transmit refresh/Google cookies or
  depend on Set-Cookie in a 101 response. Any automatic normal-HTTP renewal needs bounded cross-tab coordination;
  do not silently weaken token replay detection or claim HttpOnly cookie changes are observable from JavaScript.
- [ ] Attribute the reported application's 42 to 58 KiB payload increase from its actual doctor/ledger output.
  The application's path/report was requested; no unknown application configuration was edited. A temporary
  explicit 58 KiB application limit was recommended, not a framework default increase.
- [ ] Harden legacy broad WebSocket CSP separately from native opt-in, preserving explicit third-party declarations.
  Existing wss: policy is not evidence that self alone works in every browser.
- [ ] Reconcile remaining debt against actual source/tests. Do not check historical native-type/browser/backend
  lists wholesale. Full cross-module flow inference, arbitrary minified-byte provenance/explain-output and general
  CSE/strength reduction/loop-invariant hoisting are not established by these changes.

## Construction observations reconciled (cut a41ed89, reconciled 2026-10-30)

Twenty-two observations recorded during construction, reconciled against later commits. Status words are distinct:
implemented, tested, published. Items that were already fixed are not reopened.

- [x] OBS-01 DOM path cost: tracked by the "linearize repeated DOM sibling paths" gap above; string appending is not linearization.
- [x] OBS-02 Admin budget: 0.5.0 release gate kept the fixed edit at 30,720 B with zero headroom; no budget increase.
- [ ] OBS-03 JWT renewal before socket reconnect: audited; no automatic renewal by design, bounded stop proven by a
  test (0.5.1 development). A tab-coordinated renewal helper remains a proposal.
- [ ] OBS-04 Legacy broad `wss:` CSP: confirmed in `brain-pack` cache headers and the SSR fallback; needs an author
  `connect-src` allowlist first. Proposal only.
- [ ] OBS-05 Named-import linking can precede the original diagnostic; the feedback spool remains the authority.
- [x] OBS-06 GET `cache: 'no'` contract fixed (f2287f6). [ ] Compile documentation examples with the real SDK.
- [ ] OBS-07 Native inference remains partial; backend exact types (B-02..B-07) landed separately in 0.5.1 development.
- [ ] OBS-08 Arbitrary minified-offset provenance is not established; the ledger is accounting, not provenance.
- [ ] OBS-09 Algebraic rewrites (CSE, strength reduction, hoisting) need type/effect proofs before any use.
- [x] OBS-10 Development compile cache trusted mtime alone: reproduced (an atomic replacement keeping the timestamp
  served stale output) and fixed by keying on mtime, size and inode, with a regression test (0.5.1 development).
- [ ] OBS-11 Compact capability metadata (proven vs fallback vs unplanned) in map/card/context/quality.
- [ ] OBS-12 MCP stdio still advertises no resource subscriptions; any addition needs bounded queues and Origin/auth.
- [ ] OBS-13 Socket invalidation broker is single-process and ephemeral; do not scale backend processes as distributed.
- [ ] OBS-14 Real navigation latency and retained heap are unmeasured; build size is not runtime evidence.
- [x] OBS-15 Thread-admission guard (a8f7be1) is in the 0.5.0 release binary, which passed the six native cases.
- [x] OBS-16 Empty environment context invariant is covered by the serve oracle.
- [x] OBS-17 Release verification cleans owned packages per source identity in an isolated target volume.
- [x] OBS-18 Benchmark rows retained; a further byte-identical lexer/source-map speedup awaits paired measurement.
- [x] OBS-19 Harness preflight lessons applied: owned writable snapshot, short exec TMPDIR, explicit toolchain and browser.
- [x] OBS-20 E2E oracles precise (403 for anonymous owner reads, control frames excluded, real cleanup counts).
- [x] OBS-21 Fresh release evidence: 0.5.0 binary, SPEC and LLM gate measured on the same frozen source.
- [x] OBS-22 Checklist drift reconciled in this section without deleting history.

## Memory, loop and leak audit (0.5.1 development)

Three disjoint audits (runtime JS, Rust serve/backend/CLI, compiler). Every confirmed finding has a failing test first.

- [x] Live sockets accepted then closed with "try again later" no longer reconnect forever; budget resets after 30 s healthy.
- [x] Settled handler requests leave the component registry; synchronized stores release channels, listeners and
  sockets on close and unmount; the History router removes its service worker listener; Prompt API sessions are
  bounded to eight.
- [x] Closed HMR sockets leave the dev-server client set; `brain start` escalates SIGTERM to SIGKILL after 12 s.
- [x] Deep nesting reports B0002/B0003 instead of overflowing the stack; span columns are computed in O(log n).
- [x] macOS-only flake in the native socket proxy test: accepted sockets inherit O_NONBLOCK on BSD; the test backend
  resets it (product accept loops already did).
- [ ] RPC rate limiting keys on `X-Forwarded-For` without a trusted-proxy setting; needs an explicit policy.
- [ ] A request body read promise is not settled on client abort (no retained memory measured).
- [ ] `brain-mcp` reads stdin lines without a per-line size cap.
- [ ] Debug builds of the CLI still overflow near 120 nested `if` blocks; release builds are within the budget.

## Existing-backend migration requirements

- [x] Capture all nine capability rows from the requesting project's assessment; distinguish actual limitations
  from existing capabilities in [BACKEND-MIGRATION-READINESS.md](BACKEND-MIGRATION-READINESS.md).
- [x] Read the complete 147-line requesting project `docs/MIGRACION-BACKEND-BRAINLLM.md`, found in the project
  root above its frontend after it became available. Capture all 43 original IDs/priorities B-01 through B-43 in
  the readiness ledger. Exact private requirements are recorded outside this public repository; reading is not
  implementation or migration approval. Source SHA is recorded in the ledger.
- [ ] Complete exact money/date/enum types, relational company/vacancy/application scope and lossless legacy bigint
  compatibility with explicit migration plans; existing role/owner permissions are not organization membership.
- [ ] Preserve typed arrays, partial-update behavior, authorized state transitions and atomic multi-resource
  commands. Also freeze existing API/projection/localization/auth-session/search behavior before any cutover;
  existing object fields, role labels or asynchronous hooks do not imply that compatibility.
- [ ] Implement private PDF/general-document capability with matching limits across every gateway. Existing image
  multipart accepts 5 MiB, not the generic 64 KiB limit, but does not accept PDF.
- [ ] Add genuine Stripe/PayPal signature adapters and provider-specific raw-body/header ingress, durable receipt
  and replay/idempotency coverage; native BRAIN HMAC compatibility is not provider compatibility.
- [ ] Finish declarative search/index control and resumable rebuild/drift/query-plan acceptance. pg_trgm/unaccent/
  FTS indexes already exist; do not replace proven search from scratch.
- [ ] Validate whole-application migration parity before removing any existing backend. Admin generation is in
  current development; the source assessment's "postponed in 0.4" refers to the published baseline.

## Current combined verification

- [x] English and diff checks passed on checkpoint 3987d0b.
- [x] Source-input inventory: 108 literal embeddings, zero findings; computed macro inputs require clean compilation.
- [x] Future publisher policy regressions: three passed.
- [x] Combined immutable snapshot 3987d0b: 221/221 Rust and 453/453 runtime on Node 22.23.4 passed, no skips.
  Includes 192 compiler/lexer/pack/serve cases and 29 CLI/admin/deployment/startup/configuration/doctor cases.
  Fixed admin edit remains 30,720 B with zero headroom; no budget increase.
- [ ] Complete auxiliary tool gates in an appropriate clean writable environment. The first Linux batch returned
  88 passes, six environment failures and six skips: missing Python, read-only pack output, unspecified Chrome and
  noexec temporary fake hooks. These failures are not a complete green gate or a demonstrated product defect.

## Resume procedure

1. Read this file, then inspect the exact workflow IDs above and query npm for the exact version.
2. Inspect `git status`, worktrees and running owned jobs before editing; preserve other sessions.
3. Reuse completed evidence. Fix actual publication blockers only until all publication acceptance boxes are checked.
4. Follow the deferred checklist in isolated, reviewable slices. Bump the next development version before changes.
5. Never log credentials, expose production traces or publish local-only handoff/proposal material.

## Claude handoff readiness

- [x] Prepare a private full continuation document and send its path to the explicitly requested BRAIN-CLAUDE tab.
- [x] Claude acknowledged the correct isolated worktree and preservation of the original main/index; it is waiting
  for an explicit ready message and has not started concurrent edits/jobs.
- [x] Freeze all owned implementation slices at the user's later explicit request to conclude and transfer now.
  All three subagents completed and reported no active owned jobs. Thread guard and benchmark evidence are saved.
- [x] Prepare all project Markdown documents, private continuity notes and the supplied migration documents in an
  indexed private handoff bundle. Account details, pane identifiers and receipt are kept outside this repository.
- [ ] Confirm the explicit ready-message receipt and continuation in the assigned Claude tab. First action:
  build/test the final integrated source with a new immutable snapshot and actual binary/spec provenance.

## Final controlled compiler-scanner evidence

Span-only scanning retains the same validation and exact output. A final seven-pair Rust1.99.1 cohort moved the
fixture project onto owned Linux tmpfs, recorded CPU and wall time separately and compared JS/CSS/server/maps
byte for byte. The large case improved CPU in7/7 rounds (paired median−8.47%); fixed weights, allocation counts
and raw previous measurements remain recorded. Small cases remain noisy; there is no global compiler-speed claim,
formal confidence interval or proof that every historical regression was eliminated. Final release gates and
representative application/runtime/throughput measurements remain distinct requirements.
