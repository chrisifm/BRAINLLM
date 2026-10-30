# Release and continuation checklist

Updated: 2026-10-30. This is the resumption entry point when a session loses context or tokens.
Implementation, passing source tests and published packages are separate states. Preserve the original checkout,
index and unrelated work. Never move the published tag or blindly retry a publisher.

## Current priority

1. Close anonymous public-install verification on all three operating systems.
2. Publish the prepared English GitHub release notes and update the local HTML availability report.
3. Continue excluded work in isolated development branches, with regression evidence for each checked item.

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
- [Public install verification 41119484124](https://github.com/chrisifm/BRAINLLM/actions/runs/41119484124):
  Windows and macOS passed both initializers and their actual supported CLI journeys. Ubuntu reached doctor,
  then its browser runner failed. Earlier Ubuntu public-install run 41118057794 passed. Investigate the runner
  cause; do not remove doctor, accessibility or browser gates. A clean Linux ARM64 Docker install also passed.
- Full verification 41118057666 exposed an intermittent atomic-change lock lifecycle failure. A separate
  next-development-version worktree is addressing explicit lock release with meaningful regression coverage.
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
- [ ] Anonymous exact-version installation passes on Linux, macOS and Windows, using both initializers.
- [ ] Publish the prepared GitHub release draft with the complete English changelog.
- [ ] Update the local HTML report to published only after registry/install verification.
- [ ] Record final platform durations, workflow outcome and public links here.

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

## Continue only after publication is verified

- [x] Make manual publish=false remain pack-only even on tags, preserving automatic tag-push publication. Corrected on the CI repair branch; the immutable published tag is unchanged.
- [ ] Stabilize state/preview atomic-change leases with explicit unlock and retained-descriptor regression coverage in the next development version.

- [ ] Fix the source-map marker collision: `brain-codegen/src/srcmap.rs::finish` must not consume an author's
  quoted/template/HTML text containing `/*@L...*/`. Reproduce first; preserve owned marker mapping and budgets.
- [ ] Resume authenticated backend sockets/rooms only from the isolated proposal, not by restoring an old full tree.
  Verify authorization, bounded connections/messages, Node/Rust proxying and nginx Upgrade behavior end to end.
- [ ] Finish optional generated administrative UI without raising the default 30 KiB route budget. Public resource
  generation currently emits backend/client contracts; its administrative UI status is explicitly planned.
- [ ] Reconcile remaining debt items against actual source/tests and link closure evidence. Earlier broad native-type,
  browser-compatibility and backend checklists contain historical snapshots; do not check them wholesale.
- [ ] Keep conservative type inference limitations explicit; full control-flow/cross-module inference is not proven.
- [ ] Keep browser API/worklet capability checks and fallbacks explicit; not every API can be polyfilled.
- [ ] Evaluate advanced mathematical optimization separately. General CSE, strength reduction and loop-invariant
  hoisting are not established capabilities of this release.
- [ ] Exact arbitrary minified-byte provenance and `brain explain-output <offset>` remain unimplemented.
- [ ] Benchmark the release profile against a comparable workload before publishing performance comparisons.
- [ ] For each new implementation, update this checklist, relevant contracts/docs and changelog, then run the
  required regression/workspace/runtime/fixture/English gates and the LLM gate when author syntax changes.

## Resume procedure

1. Read this file, then inspect the exact workflow IDs above and query npm for the exact version.
2. Inspect `git status`, worktrees and running owned jobs before editing; preserve other sessions.
3. Reuse completed evidence. Fix actual publication blockers only until all publication acceptance boxes are checked.
4. Follow the deferred checklist in isolated, reviewable slices. Bump the next development version before changes.
5. Never log credentials, expose production traces or publish local-only handoff/proposal material.
