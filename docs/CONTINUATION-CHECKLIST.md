# Release and continuation checklist

Updated: 2026-10-30. This is the resumption entry point when a session loses context or tokens.
Keep checkboxes tied to observed evidence. An implementation, a passing source test and a published package
are different states. Do not mark the whole roadmap complete because this release is stable.

## Current priority

1. Finish publishing the frozen stable 0.4.0 checkpoint and verify public installation.
2. Publish the English release notes and update the local HTML report with verified availability.
3. Continue excluded work separately. Do not add features to the frozen release to delay publication.

Chris explicitly authorized the stable release and continuation afterward. Preserve unrelated work and other
sessions. Do not merge into main automatically or reset the original checkout/index.

## Frozen source and publication

- Release branch: `release/0.4.0-stable`.
- Release tag: `v0.4.0`; resolve its selected commit with `git rev-parse v0.4.0^{commit}`.
  The production correction is verified at `f1608c622586e349fd858cf3fbf3bcf6cddadbbf`; subsequent notes-only
  commits do not replace that code evidence.
- Corrected-source full verification: [41103502428](https://github.com/chrisifm/BRAINLLM/actions/runs/41103502428), **success**.
- Next publication: select the newest `v0.4.0` run in the [release workflow](https://github.com/chrisifm/BRAINLLM/actions/workflows/release.yml).
  Earlier run 41097781294 failed its Windows smoke and had inconsistent API state.
- Earlier run 41096620791 was canceled before publication. The correction only selected the Linux CI browser;
  the production source and validated binary did not change. No 0.4.0 package had been published at correction time.
- Validated macOS ARM64 binary SHA-256:
  `c5a264db5930a7166cfd56c91a597ce61867df74337fbd8e98ae35f6a16add3e`.
- Original checkout remains on `main` at `5c31575`; release commits descend from `origin/main` at `a4e2cb9`.
  The original index and working state were preserved using a separate release index.

### Active publication blocker (latest evidence)

- Corrected source: `f1608c622586e349fd858cf3fbf3bcf6cddadbbf` on `diagnostic/windows-0.4.0`.
- Isolated two-Windows run: [41103502488](https://github.com/chrisifm/BRAINLLM/actions/runs/41103502488).
- Full verification of that correction: [41103502428](https://github.com/chrisifm/BRAINLLM/actions/runs/41103502428).
- Corrected remapped binary SHA-256: `c5a264db5930a7166cfd56c91a597ce61867df74337fbd8e98ae35f6a16add3e`.
- Corrected binary passed native macOS new/check/build and actual Viewer/headless replay (three scenarios each);
  no fallback/errors, full AX replay, heap not requested. Its fresh LLM cohort passed 20/20, with stable hashes and zero infrastructure failures.
- Three focused file-URL regressions passed locally: Windows serialization oracle, real Node imports/cache busting,
  and Unix byte/backslash round trips with explicit rejection rather than an incorrect alternate import.
- Both Windows release builds completed, but the actual native smoke failed with compiler exit 2.
- Exact failing subcommand is not yet known: the smoke overwrote one command log and removed its scratch in finally.
- Preserve the gate. Safe phase/error-code feedback and a two-Windows diagnostic workflow are prepared.
- A Windows file-URL defect is reproduced: canonical verbatim drive paths become malformed Node import URLs.
  The correction passed both actual Windows journeys and two native file-URL regressions per platform.
- Corrected-source gates passed: 837 Rust tests, 418 runtime tests, full CI, native Windows x64/ARM64,
  native macOS smoke, actual Viewer/headless replay and a fresh 20/20 LLM comparison.
  The final seven-platform distribution matrix and anonymous registry verification remain required.
- GitHub's job status/steps endpoints are stale or return intermittent HTTP 502. Direct REST job logs established
  these failures; do not interpret missing API steps as a successful or still-compiling job.
- Windows x64 compilation: 8m 37s; Windows ARM64 compilation: 7m 50s. These are observed compiler timings.
- GitHub also contradicts itself: GET says in progress, force-cancel POST says HTTP409 not in progress.
  Treat the old run as inconsistent; do not blindly rerun a potentially publishing workflow.
- The latest public npm version remains 0.3.1. Publication of 0.4.0 is not confirmed.
- Metadata update prepared and package-tested: precise frontend/backend/CLI/full-stack keywords and a leading
  Rust-powered description. Not public until the next verified package publication. Category visibility on the
  screenshot directory still needs its actual URL/rules. Do not advertise IoT, robotics or math capabilities.

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
- [ ] Linux x64 final distribution build, smoke, notices and artifact.
- [ ] Linux ARM64 final distribution build, smoke, notices and artifact.
- [ ] macOS ARM64 final distribution build, smoke, notices and artifact.
- [ ] macOS x64 final distribution build, smoke, notices and artifact.
- [ ] Linux x64 musl final distribution build, smoke, notices and artifact.
- [ ] Windows x64 final distribution build, smoke, notices and artifact.
- [ ] Windows ARM64 final distribution build, smoke, notices and artifact.
- [ ] Pack all seven platforms and all 11 exact-version npm packages in CI.
- [ ] Verify all 11 public registry versions are 0.4.0. Never infer this from a tag, local pack or started job.
- [ ] Anonymous exact-version installation passes on Linux, macOS and Windows, using both initializers.
- [ ] Publish the prepared GitHub release draft with the complete English changelog.
- [ ] Update the local HTML report to published only after registry/install verification.
- [ ] Record final platform durations, workflow outcome and public links here.

## Measured platform durations

Observation: 2026-10-30 17:31:48 UTC, run 41097781294. Job duration excludes queue wait and includes compile,
native smoke, notices and artifact upload. Running values are elapsed time, not predicted completion time.
This is one execution, not a historical average.

| Platform | Compile | Job | State at observation |
|---|---:|---:|---|
| Linux x64 | 5m 08s | 5m 25s | Success |
| Linux ARM64 | 4m 27s | 4m 44s | Success |
| macOS ARM64 | 7m 20s | 7m 57s | Success |
| macOS x64 | 11m 55s | 12m 28s | Running |
| Linux x64 musl | 3m 41s | 4m 06s | Running |
| Windows x64 | 1m 55s | 2m 11s | Running |
| Windows ARM64 | — | — | Queued |

### Corrected Windows diagnostic measurements

Run 41103502488, corrected source f1608c6. Both platforms passed their two native URL tests and complete smoke.
These jobs additionally include URL regression execution and compilation-cache upload, so they are not the final
seven-platform release timings. Values are one observation, not averages.

| Platform | Compile step wall time | URL regression step | Native smoke | Whole job |
|---|---:|---:|---:|---:|
| Windows x64 | 9m 04s | 1m 47s | 3.398s | 11m 31s |
| Windows ARM64 | 8m 06s | 1m 40s | 3.528s | 10m 47s |

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
