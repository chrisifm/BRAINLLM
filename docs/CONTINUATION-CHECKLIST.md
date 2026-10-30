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

Version: **0.4.1-dev.0**, branch `development/0.4.1`, integration checkpoint `0a411f9`. Everything below is local development unless explicitly
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
  These are component results; CLI startup and Node manifest consumers remain separate acceptance items.

All new Linux results above use **Rust 1.96.1 diagnostic tooling** unless noted otherwise. They are not the final
Rust 1.99.1 release gate and do not establish cross-platform execution or whole-application memory coverage.

## Remaining acceptance before freezing the next version

- [x] Integrate Node bridge sources 5906096 + b5d6c5f locally (8c6434a + fd52ab6).
- [ ] Run the actual pack/private-helper/dev-sidecar Rust embedding tests on the combined tree.
- [x] Integrate the completed Rust development/static gateway component (d03d2fc).
- [ ] Execute the added empty-context environment oracle and all serve tests on the combined tree. No ambient
  process environment may be added to ctx.env when its loader is absent.
- [x] Integrate the scoped liveResource client/compiler/configuration/packaging component (0a411f9).
- [ ] Complete and verify CLI development/test/static startup and Node private-manifest consumers against the
  actual combined gateway/configuration source, including selected-port Origin preflight and disabled defaults.
- [ ] Wire one reviewed brain.json backend.origins/sockets configuration into backend startup, Node SSR manifest,
  Rust development proxy and exact public socket origin. Preserve disabled defaults and explicit operator overrides.
- [x] Candidate nginx deployment templates passed six actual parser/TLS/TCP checks: four disabled topologies and
  two socket-enabled Rust topologies; query/method/protocol/body rejection and credential projection included.
- [ ] Integrate deployment sources and verify compiler-rendered PM2/nginx configurations against those tested
  templates. Node routing preserves its credential projection and bridge lifecycle; it does not rename cookies.
- [ ] Run actual browser plus native PostgreSQL end-to-end CRUD/live/auth/session-switch/shutdown checks. Prefer an
  installed connected Viewer as documented; report headless fallback, AX and heap coverage precisely.
- [ ] Re-run administrative budget/required/no-JS cases on the final combined tree. Keep the 30 KiB reference gate;
  do not hide the zero-headroom fixture or increase limits to pass.
- [ ] Investigate source-map performance before any compiler-speed claim: seven paired diagnostic DEBUG runs showed
  large-prod **72.91 to 148.55 ms (+103.74%)**, despite the functional correction. Release-profile paired measurement
  is prepared but not executed; classify tooling/profile/inputs accurately and fix material production regressions.
- [ ] Run final workspace/runtime/fixture/source-input/privacy/English gates on the combined source and current
  stable Rust 1.99.1, plus a fresh frozen LLM gate for author-facing SPEC/CLI changes. No next-version publication yet.
- [ ] Keep the original main checkout/index and all unrelated sessions intact; commits/push/review do not imply a main merge.

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

## Resume procedure

1. Read this file, then inspect the exact workflow IDs above and query npm for the exact version.
2. Inspect `git status`, worktrees and running owned jobs before editing; preserve other sessions.
3. Reuse completed evidence. Fix actual publication blockers only until all publication acceptance boxes are checked.
4. Follow the deferred checklist in isolated, reviewable slices. Bump the next development version before changes.
5. Never log credentials, expose production traces or publish local-only handoff/proposal material.
