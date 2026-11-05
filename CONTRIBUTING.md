# Contributing to BRAINLLM (written for LLM agents)

BRAINLLM is a web framework whose primary user is an LLM. Contributors are also expected to be LLMs. This file is
the map; read it fully before changing anything. Three documents, three audiences:

| File | Audience | Content |
|---|---|---|
| `SPEC.md` | the LLM that **writes apps** | normative contract; `brain spec` serves it without `planned` blocks |
| `docs/INTERNALS.md` | the LLM that **changes the framework** | contracts between crates and runtime, numbered §1–§50 with dates |
| `CONTRIBUTING.md` (this) | the LLM that **changes the framework** | repo map, invariants, recipes, definition of done |

## Pipeline
`.brain` → `brain-lex` → `brain-parse` (AST) → `brain-check` (Diag + edits) → `brain-codegen` (JS + CSS + server JS)
→ `brain-pack` (chunks) → `brain-min` (mangle, `RUNTIME_MAP`) → `dist/`. Dev: `brain-serve` (HTTP/WS/HMR, router,
`load` sidecar). Browser: `runtime/*.js`. Interfaces: `brain-cli` (binary `brain`) and `brain-mcp` (JSON-RPC stdio).

## Repo map
| Path | Responsibility | Tests |
|---|---|---|
| `crates/brain-lex` | tokens, spans, `Diag` (+ `edits`, `level` error/warning/info, `apply`, `to_json`) | `cargo test -p brain-lex` |
| `crates/brain-parse` | AST (`src/ast.rs`), markup, BrainScript, `file_kind` | `tests/snapshots/*.ast` |
| `crates/brain-check` | every rule B00xx–B14xx, `load_ctx`, `scan_secrets` | `tests/rules.rs`, `edits.rs`, `fixtures.rs` |
| `crates/brain-codegen` | `compile`/`compile_with` → `Output {js, css, server_js, server_fns, deps, names, warnings}` | `tests/snapshots.rs`, `e2e.rs` (real runtime), `server.rs`, `normalize.rs`, `llm_gaps.rs` |
| `crates/brain-css` | scoping (`@scope … to ([data-b])`), tokens, mixins, layers | in workspace tests |
| `crates/brain-min` | own JS lexer, ASI-safe printer, scope mangling, `RUNTIME_MAP` | `cargo test -p brain-min` + runtime suite with `BRAIN_RUNTIME=<min>` |
| `crates/brain-metrics` | per-build stage timings, work counters and per-revision facts shared by threads | `cargo test -p brain-metrics` + `crates/brain-cli/tests/build_metrics.rs` |
| `crates/brain-pack` | bundler, prerender/SSG, server manifest, cache + service worker, build signature, SEO step, CSP/SRI | `cargo test -p brain-pack` (`tests/seo_paths.rs`, `tests/ssr.rs`, `cache::tests`) |
| `crates/brain-seo` | SEO module: head rewrite, sitemaps, feeds, llms.txt, Markdown copies, audit B18xx, collections (`content::`) | `cargo test -p brain-seo` |
| `crates/brain-serve` | dev server: HTTP/WS by hand, router (`route.rs`), `load` sidecar (`load.rs`), managed `Dev` handle (stop/logs/errors/load), HMR client, `/@brain/ui` kit, test page | `cargo test -p brain-serve` |
| `crates/brain-cli` | commands, scaffold/templates, `spec.rs` (strip `planned`), `map.rs`, CDP client (`cdp.rs`) + test runner + debug browser (`browser.rs`), `mcp.rs` (tools, resources, prompts, undo journal); `build.rs` embeds `runtime/*.js` + `runtime/ui/*.brain` and fails on bad `planned` markers | `cargo test -p brain-cli` |
| `crates/brain-mcp` | MCP protocol (tools + outputSchema, resources, prompts, notifications) + hand-written JSON | `cargo test -p brain-mcp` |
| `runtime/` | `brain.js` core (signals, DOM, list/when, hydrate, rpc, error routing + B0902 cut) · `brain-guard.js` (per-component boundary, §42) · `brain-island.js` (resumable islands) · `brain-data.js` (`api`, `resource`) · `brain-form.js` · `brain-persist.js` · `brain-worker.js` · `brain-router.js` · `brain-lazy.js` · `brain-dev.js` (dev/hmr) · `brain-wc.js` (define) · `brain-inspect.js` (§9 metrics) · `brain-err.js` (rich capture, source maps) · `brain-ch.js` (provide/use/key/scope, store, bus, expose) · `sw/brain-sw.js` · `server/server.mjs` (production server) · `ui/*.brain` (kit) · `shim/dom.js` (test DOM) | `npm test --prefix runtime`; `test/compiled.test.js` runs real codegen output (`test/fixtures/compiled/regen.sh`) |
| `examples/` | valid apps; `examples/invalid/` one case per rule + `.expected.json` | `node bench/fixtures-check.mjs` |
| `bench/llm/` | LLM benchmark (`run.mjs`, `analyze.mjs`, `gate.mjs`, `baseline.json`) | the release gate |

## Commands
Verification requires Rust stable, Node 22+, Git, ripgrep (`rg`) and Python 3 (standard-library archive fixtures).
Browser integration tests also need Chrome.
CI ensures ripgrep is available; fixture discovery uses Node without a separate file-search tool.

```bash
cargo test --workspace                 # all Rust crates (database tests need the separate harness)
node scripts/backend-check.mjs         # owned temporary Postgres + backend/CLI/HTTP acceptance; PG_BIN may select its bin folder
npm test --prefix runtime              # runtime (Node 22+)
node bench/fixtures-check.mjs          # every invalid fixture yields exactly its expected Diag
node bench/llm/gate.mjs                # first-pass rate must not drop vs bench/llm/baseline.json
scripts/check-english.sh               # no Spanish anywhere (accents or common Spanish words)
node scripts/gen-docs-index.mjs        # regenerate docs/INDEX.md after adding or retitling a doc (--check in scripts/test)
target/debug/brain doctor              # inside any app
```

`bash scripts/verify.sh` runs noninteractively on POSIX. A kernel file lock serializes gates sharing the same
physical Cargo target (from CARGO_TARGET_DIR or Cargo configuration); a competing gate exits 75 before compilation.
The lock uses an empty inherited stdin and follows exec/ordinary children, so an interrupted leader cannot admit a
second gate while a child still holds that target. It adds no supervisor or new process-cancellation behavior.
Use the native Windows diagnostic workflow for its separate supported smoke journey.

The required verify workflow runs the regular suite and a separate disposable-Postgres backend job.
The harness never uses an existing DATABASE_URL; install local Postgres tools or set PG_BIN to their bin folder.
It includes database-dependent ignored tests, but excludes the explicitly manual AVIF cost measurement; run that
separately with `cargo test --release -p brain-backend --lib avif_cost -- --ignored --nocapture`.

## Invariants (a change that breaks one is rejected)
1. **No third-party parsers, bundlers, minifiers or compressors.** Rust deps only for low-level I/O (`notify`).
   Runtime: zero dependencies, no polyfills. Maintained Rust infrastructure crates are allowed for the experimental
   backend and build-time Brotli (2026-10-29, maintainer authorization); use the latest stable direct versions and
   record choices in docs/BACKEND-DESIGN.md. BrainScript/JS/CSS parsing, bundling and minification remain owned here.
2. **Runtime core ≤ 4 096 B gzip** after `brain-min`. New features go to a separate `brain-*.js` module if the core
   has no room.
3. **Hot path uses vanilla primitives:** indexed `for`, direct DOM (`node.data`, `cloneNode`, `insertBefore`), fixed
   object shapes; no `Proxy`, spread or per-iteration closures (INTERNALS §13).
4. **SPEC promises only what works.** Unimplemented text goes inside `<!--planned:#N-->` … `<!--/planned-->`, each
   marker alone on its line (the build fails otherwise). Remove the markers in the same change that ships the feature.
5. **One way per need, named the way a model predicts it.** Before adding syntax or a name, check what LLMs write
   spontaneously (bench/llm results); prefer making the natural form valid over documenting a new one.
6. **Every diagnostic is actionable:** `msg` + `fix`, and exact `edits` whenever the correction is unique. Judgment
   calls (alt text, headings, missing label) never get auto-edits.
7. **Non-Baseline web APIs only behind feature detection**, with a classic fallback (INTERNALS §19).
8. **Deterministic output:** same input and version → same bytes. No timestamps or absolute paths in output.
9. **Security defaults cannot be turned off:** a11y (B04), server/secrets (B07), isolation (B08).
10. **English everywhere in the repo:** code comments, identifiers, every string literal and UI text (sample and test
    data included), diagnostics (`msg`, `fix`), CLI/MCP output, test names, templates and docs.
    `scripts/check-english.sh` must pass.
11. **BRAINLLM is a general-purpose framework.** A report from a real app is evidence, not a spec: understand its context, find the general defect or missing capability, and make the adjustment for every app. Never build for one app: no app names, or tables, fields, functions, roles or rules from one app's domain, in defaults, generated code, messages or docs. If nothing general remains, the change stays in that app. `scripts/check-general.sh` enforces the names.
12. **This repository is public; nothing private enters it.** Every change, test, fixture, example, doc, commit message
    and release note uses generic placeholders (`example.com`, `com.example.web`, `ABCDE12345`, synthetic keys). Never
    commit an app's or account's identifiers (team, client, service or app ids, key ids, domains, return URLs), hosts or
    IP addresses, emails, phone numbers, addresses, tokens or data copied from a real deployment, even when a report
    quotes them. `node scripts/privacy-audit.mjs --history` must pass before every push, tag or release; it also checks
    the private terms each maintainer lists in the untracked `.git/info/private-terms` of their clone.

## Compatibility review

New non-security author diagnostics in a minor release start as warnings with a deterministic mechanical fix where
one exists. Preserve renamed syntax as a deprecated alias for at least one minor release. Security invariants remain
fail-closed and cannot be weakened for compatibility. Record changes in each release's Upgrading section.
`scripts/compatibility-check.mjs` compares explicit reviewed release manifests; its source inventory is partial
coverage and is not a historical baseline. See docs/COMPATIBILITY.md. Provider-neutral benchmark plumbing is tested
by `node --test bench/agents/runner.test.mjs`; actual model evaluation still requires the fresh LLM gate.

## Recipes
**Add a checker rule**
1. Pick the next free code in its family (INTERNALS §5 catalog).
2. Implement it in `crates/brain-check/src/lib.rs` with `push` (no edits), `push_edits` (exact edits, still an error)
   or `push_info` (deterministic normalization, INTERNALS §21: the compiler applies it on its own; test it in
   `crates/brain-codegen/tests/normalize.rs`).
3. Add a positive and a negative case to `tests/rules.rs`; if it has edits, add it to `tests/edits.rs` (the test applies
   the edits and asserts the Diag disappears).
4. Add `examples/invalid/<case>.brain` + `<case>.expected.json` (code, file, line, col exact).
5. Add the code to the INTERNALS §5 catalog. If it changes what app authors must write, add one SPEC line.

**Add a runtime primitive**
1. Implement in `runtime/brain.js` (or a `brain-*.js` module); add tests in `runtime/test/`. Sibling modules reach core
   internals only through `_rt()` (root, owner, nodes, rm) — never `_dev`/`_err`, which install hooks.
2. Check the gzip size of the core; add the export to `RUNTIME_MAP` in `crates/brain-min` (the build fails if missing).
   Internal exports `_dev`, `_err`, `_rt`, `_ev` are part of the map too (sibling modules import them by name).
3. Add the name to `PRIMITIVES` and, if it lives outside the core, to `EXT_PRIMITIVES` (name → module) in
   `crates/brain-check/src/scope.rs`; codegen then imports it automatically (app authors never import primitives).
4. Add its row to the SPEC primitive table inside a `planned` block until codegen and tests are green, then unhide.

**Add markup syntax or a directive**
`brain-parse/src/markup.rs` + `ast.rs` → rule in `brain-check` (and `src/scope.rs` if the directive declares names) → `brain-codegen/src/template.rs` → snapshot in
`tests/snapshots/` → behaviour in `tests/e2e.rs` → fixture → SPEC line. Measure with bench/llm before and after.

**Release: add a change manifest entry (every feature, fix or breaking PR)**
Every CHANGELOG bullet is mirrored by one machine-readable entry that `brain upgrade`, `brain doctor` and MCP `brain_upgrade` read. Adding one is a single new file:
`crates/brain-cli/releases/<next-version>/<id>.json` (the newest folder holds `[Unreleased]`; create the folder for a
new version). `build.rs` embeds every file, so the npm package ships them inside the binary.
```json
{ "id": "file-buckets", "kind": "capability", "title": "Native file buckets for uploads and downloads",
  "why": "What changes for an app author, in one or two sentences.",
  "changelog": ["Native file buckets (`server/files/<name>.jsbrain`)"],
  "guide": "docs/backend/files.md", "codes": [], "apis": ["server/files/<name>.jsbrain"], "affects": ["server/files/**"],
  "adopt": { "file": "server/files/attachments.jsbrain", "snippet": "<meta server>\nexport default { maxBytes: '25mb' }" },
  "uses": [{ "glob": "server/files/*.jsbrain" }] }
```
- `kind`: `capability` (needs `adopt`, `uses`, `guide`), `change`, `deprecation` and `breaking` (need `steps` plus
  `finds` probes or `codes` so the report can point at `file:line`), `fix`, `internal`.
- `changelog`: short phrases copied from the CHANGELOG bullets the entry covers. `cargo test -p brain-cli releases`
  fails when a bullet outside `### Tests`/`### Internal`/`### Upgrading` matches no entry, or a phrase matches no bullet.
- `codemod`: only an id from `releases::CODEMODS`; `codes` must exist in the catalog; `guide` is `brain guide <topic>`
  or a repository path. Probes are app-relative globs (`**`, `*`, `{a,b}`) with an optional `contains` substring.
- At release time rename `[Unreleased]` to the version; the folder name already matches it.

**Add a CLI command or MCP tool**
`crates/brain-cli/src/main.rs` (+ `--json` with `schemaVersion`, exit codes 0/1/2/3 — from #68) and `mcp.rs` (tool with
`inputSchema`/`outputSchema`; destructive tools require `confirm: true`; never a publish tool). Add a test and the
SPEC CLI line.

## Changing what the LLM writes
Any change to SPEC, scaffold, templates, names or rules that affect app authors must be measured:
`node bench/llm/run.mjs` with the spec and binary frozen by sha, then `analyze.mjs` and `gate.mjs`. Record the
before/after in `bench/llm/README.md`. Fix failures in this order (INTERNALS §21): make the error impossible →
deterministic normalization (info diagnostic) → template that already has the right structure → one SPEC line.

## Definition of done
- `cargo test --workspace`, `npm test --prefix runtime`, `node bench/fixtures-check.mjs` and
  `scripts/check-english.sh` are green.
- Runtime size budget respected; no new dependency.
- INTERNALS updated (section + date) and SPEC updated (planned markers removed only for what now works).
- If it affects app authors: LLM gate does not regress.
- A bug fix covers the class, not the instance: name the defect class, search it in other modules, options, both gateways
  and every channel, and fix what you confirm. Add the gate that would have caught it; it must fail on the unfixed code.
- Commits: semantic, English (`feat(check): add B1305 heavy work inside each`). Push contributions only to your own
  fork or an explicitly configured review branch. Every merge requires the principal author's review; no automatic
  merge or npm publication. See [the evolution workflow](docs/EVOLUTION.md).

## Open contributions and compiler improvements

Human and LLM contributors are welcome under the MIT license. Christian Irack Flores Maldonado (@chrisifm) is
the principal author and required reviewer. Fork the repository, implement a focused change with regression tests,
run `bash scripts/verify.sh`, and open a pull request with the evidence and any performance/compatibility impact.
Do not include private app source, credentials or production data. Use synthetic reproducers.

An LLM using BRAINLLM may correct a discovered compiler defect instead of repeatedly working around it. Follow
[EVOLUTION.md](docs/EVOLUTION.md): isolated worktree, tests, validated binary, reversible activation and optionally
automatic push to a proposal branch. Changing the controller, its gates or dependencies uses a normal reviewed PR.
The live compiler process is never patched in memory. Maintain the app contract and record unsupported behavior.

For React/Angular/Vue/Svelte migrations use [MIGRATION.md](docs/MIGRATION.md), including its read-only inventory
command and per-slice parity ledger. Compilation is one gate, not a substitute for frontend behavior tests.
