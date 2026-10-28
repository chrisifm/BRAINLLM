# BRAINLLM

Web framework whose primary user is an LLM: a `.brain` format (HTML + BrainScript + CSS in one file), fine-grained
reactivity without a virtual DOM, and its own Rust toolchain (single `brain` binary) with no Vite, webpack, esbuild
or Babel. Spec for the AI: `SPEC.md`. Internal contracts: `docs/INTERNALS.md`. Contributing: `CONTRIBUTING.md`.

## Base architecture

`.brain` → `brain` (Rust: lex → parse → check → codegen → pack → min) → direct-DOM JS + per-view CSS → JS runtime
≤4 KB gz (signals, delegation, resumable hydration). The LLM works through the CLI or MCP and reads `brain spec`
instead of guessing the API.

## Stack

| Layer | Technology | Why |
|---|---|---|
| Toolchain | stable Rust 1.96, Cargo workspace | native speed, memory safety, wasm32 target for a playground |
| Component language | `.brain` = HTML + BrainScript (closed JS subset) + CSS | what the LLM already knows, with fewer ways to go wrong |
| Runtime | hand-written ES modules, no build | ≤4 KB gz, zero dependencies |
| Tests | `cargo test` + snapshots; `node --test` for the runtime | no external test frameworks |
| External dependencies | low-level only (`notify`) | no third-party parsers, bundlers or minifiers |

## Phases

- [x] 0. Validation: tokens and error rate (semantic vs cryptic names), TC39 Signals, versions
- [x] 1. Workspace scaffold + `APP.md` + `SPEC.md` v0 + git
- [x] 2. JS signal runtime + `node --test`
- [x] 3. `brain-lex` / `brain-parse` → AST with spans; `brain-check` with actionable errors
- [x] 4. `brain-codegen` (templates, events, if/each) + `brain-css`
- [x] 5. `brain-pack` + `brain-min`
- [x] 6. `brain-serve` (HMR, router, `load`) + CLI (`new`, `run`, `build`, `doctor`, `add`, `check`, `fix`, `map`, `spec`)
- [x] 7. `brain-mcp` + `llms.txt`
- [x] 8. SSR + resumable islands · dev memory inspector and devtools (INTERNALS §9)
- [ ] 8b. `brain/ui` kit + `form()` · contract-based data (`api`, `resource`, `--mock`) · `brain shot` + budgets ·
  `brain edit/undo/migrate` · `brain doctor` (loops, leaks, over-engineering) · native error capture (INTERNALS §14–§18)
  — shipped (`brain_shot`, `brain_edit` and `brain_undo` as MCP tools); the AST refactors of `brain edit` (#37) and
  shot baselines/budgets/`migrate` (#40) are deferred past launch
- [x] 9. npm distribution: `npm create @christianirack/brainllm@latest` + `@christianirack/brainllm` with per-platform binaries
- [ ] 10. Benchmarks (vs React/Vue/Angular/Svelte/Solid; build vs Vite) + LLM benchmark (100 views × 3 models)
  — js-framework-benchmark, build and autonomy benchmarks done (`bench/REPORT.md`); the 100 × 3 LLM bench (#64) is deferred
- [ ] 11. Public launch (at the end, push only with Chris's OK): secret sweep of tree and history, English README and
  docs (only measured numbers), futuristic artwork with a brain at the center, LICENSE, public GitHub repo

## Suggested team

LEAD·ARCHITECT (`APP.md`, `SPEC.md`, root `Cargo.toml`, docs) · COMPILER (`crates/brain-{lex,parse,check,codegen,css}`)
· TOOLCHAIN (`crates/brain-{pack,min,serve,mcp,cli}`) · RUNTIME (`runtime/`) · QA (`examples/`, `bench/`).

## Local release closure (2026-10-27)

The current release target is 0.1.0. The unchecked phases above contain deferred work and public-launch decisions;
they are not a claim that every planned feature ships in this release.

Validated locally on macOS arm64 with Node 22.21.1:

- [x] Rust workspace: 449 tests passed, zero failures or ignored tests. Cargo used a temporary runner that copies
  test executables to scratch before execution to avoid slow startup from the existing target directory.
- [x] Runtime: 276 tests passed, including the 4096 B gzip core budget.
- [x] Diagnostic fixtures match; valid examples emit zero diagnostics.
- [x] English-only source check and verification script syntax check pass.
- [x] Host npm packages generated; clean install passes all seven steps in 12.7 s.
- [x] Archived gate277a LLM runs pass the accepted baseline gate (Haiku 19/20, Sonnet 20/20). No new model
  generation was performed; these are historical measurements, not current-release generation results.

- Verification entry point: `bash scripts/verify.sh`. It builds the current CLI before testing fixtures, then runs
  the Rust workspace, runtime, diagnostic fixtures and English-only checks. `.github/workflows/verify.yml` runs it
  on branch pushes, pull requests and manual dispatch, independently of the release packaging workflow.
- Local packaging: `node npm/pack.mjs`, followed by `node bench/install/run.mjs --no-build`. The packer remaps
  machine paths, excludes runtime dot-directories and writes SHA-256 sums for binaries and tarballs.
- LLM gate: `node bench/llm/gate.mjs <run.json>` needs an explicit frozen generation run. Re-evaluating an archived
  run only checks that historical result against the baseline; it does not measure the current source tree.
- Deferred features remain inside the `planned` blocks in SPEC: additional generators, lazy components and Img,
  preview/explain/doctor CLI additions, screenshot baselines and budgets, migrations and AST refactors.
- Public launch remains separate: select a license, supply the actual public repository metadata, validate all
  seven platform binaries, re-measure release benchmarks and audit repository history before publication.
  No push or publication is part of local closure.

## Code-job reliability hardening (2026-10-28)

- [x] Bound dev connection admission and file-event queues; sidecar waiters have a separate 5 s queue budget.
- [x] Enforce absolute response deadlines, kill/reap timed-out Node work and cap protocol buffers.
- [x] Cancel private-browser watchdogs, clean owned process groups/profiles and close dev WebSocket readers.
- [x] Bound complete compiler-cache entries (128 MiB / 1024 entries), stream hashes and use unique atomic writes.
- [x] Coalesce concurrent compilation; invalidate cached parents and old Node module versions after source edits.
- [x] Verification: 458 Rust tests, 276 runtime tests, 46 diagnostic fixtures and English-only checks pass.

These are tested lifecycle and resource bounds, not a measured throughput or total-RSS improvement. The dev
compiler currently serializes cache misses; there is no new general-purpose job scheduler or unsafe thread kill.

## Architecture review and open contributions (2026-10-28)

- [x] Cache keys avoid parsing on lookup and cover adoption, lint, environment, locale and compiler build inputs.
- [x] Context invalidation covers missing files, schemas, configs, layouts and graph changes; HMR revalidates dependencies.
- [x] Bound compiler/HMR, client-data and SSR cache retention. Private/session responses bypass shared SSR storage.
- [x] HTTP queues have finite admission and wait budgets; stale GETs cannot refill data invalidated by a write.
- [x] LLM evolution controller: isolated worktrees, regression tests, validation, immutable binary activation/rollback,
  privacy checks and optional automatic pushes to review branches. Main merges require maintainer review.
- [x] Incremental migration planner and prompts for React, Angular, Vue, Svelte, Next.js and Nuxt. It inventories
  projects and guides tested slices; it is not an automatic transpiler or a claim of universal compatibility.
- [x] MIT license, principal-author identity, contributor instructions, CODEOWNERS and CI with bounded concurrency.
- [x] Local verification: 462 Rust tests (including compiler-identity coverage in an isolated proposal), 281 runtime
  tests, 5 tooling tests, 46 diagnostic fixtures and English-only checks. Release compilation and proposal
  activation/rollback were exercised. Final code also passes targeted HMR/cache tests.
- [x] Cache-key microbenchmark: 10000 lookups of a 1050-byte component; parsed path 0.562 s, direct path 0.013 s
  (43.3x for that operation). Reproduce with `cargo run --release -p brain-codegen --example cache_lookup`;
  evidence: `bench/build/results/compiler-cache-lookup.json`. No whole-build or total-RSS speedup is claimed.

Public repository: `https://github.com/chrisifm/BRAINLLM`. The publication snapshot uses a fresh history; original
local Git state is retained privately. Scans cover the export and original/reachable history. Explicit synthetic
diagnostic examples are reviewed exceptions; author name, email and websites are intentionally public. npm packages
are not published by this repository launch. Existing LLM gate277a results still pass their historical baseline;
no new model-generation benchmark or real external-application migration was performed for this review.
