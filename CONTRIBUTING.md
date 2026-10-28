# Contributing to BRAINLLM (written for LLM agents)

BRAINLLM is a web framework whose primary user is an LLM. Contributors are also expected to be LLMs. This file is
the map; read it fully before changing anything. Three documents, three audiences:

| File | Audience | Content |
|---|---|---|
| `SPEC.md` | the LLM that **writes apps** | normative contract; `brain spec` serves it without `planned` blocks |
| `docs/INTERNALS.md` | the LLM that **changes the framework** | contracts between crates and runtime, numbered §1–§42 with dates |
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
| `crates/brain-pack` | bundler, prerender/SSG, server manifest, cache + service worker, build signature, SEO step, CSP/SRI | `cargo test -p brain-pack` (`tests/seo_paths.rs`, `tests/ssr.rs`, `cache::tests`) |
| `crates/brain-seo` | SEO module: head rewrite, sitemaps, feeds, llms.txt, Markdown copies, audit B18xx, collections (`content::`) | `cargo test -p brain-seo` |
| `crates/brain-serve` | dev server: HTTP/WS by hand, router (`route.rs`), `load` sidecar (`load.rs`), managed `Dev` handle (stop/logs/errors/load), HMR client, `/@brain/ui` kit, test page | `cargo test -p brain-serve` |
| `crates/brain-cli` | commands, scaffold/templates, `spec.rs` (strip `planned`), `map.rs`, CDP client (`cdp.rs`) + test runner + debug browser (`browser.rs`), `mcp.rs` (tools, resources, prompts, undo journal); `build.rs` embeds `runtime/*.js` + `runtime/ui/*.brain` and fails on bad `planned` markers | `cargo test -p brain-cli` |
| `crates/brain-mcp` | MCP protocol (tools + outputSchema, resources, prompts, notifications) + hand-written JSON | `cargo test -p brain-mcp` |
| `runtime/` | `brain.js` core (signals, DOM, list/when, hydrate, rpc, error routing + B0902 cut) · `brain-guard.js` (per-component boundary, §42) · `brain-island.js` (resumable islands) · `brain-data.js` (`api`, `resource`) · `brain-form.js` · `brain-persist.js` · `brain-worker.js` · `brain-router.js` · `brain-lazy.js` · `brain-dev.js` (dev/hmr) · `brain-wc.js` (define) · `brain-inspect.js` (§9 metrics) · `brain-err.js` (rich capture, source maps) · `brain-ch.js` (provide/use/key/scope, store, bus, expose) · `sw/brain-sw.js` · `server/server.mjs` (production server) · `ui/*.brain` (kit) · `shim/dom.js` (test DOM) | `npm test --prefix runtime`; `test/compiled.test.js` runs real codegen output (`test/fixtures/compiled/regen.sh`) |
| `examples/` | valid apps; `examples/invalid/` one case per rule + `.expected.json` | `node bench/fixtures-check.mjs` |
| `bench/llm/` | LLM benchmark (`run.mjs`, `analyze.mjs`, `gate.mjs`, `baseline.json`) | the release gate |

## Commands
```bash
cargo test --workspace                 # all Rust crates
npm test --prefix runtime              # runtime (Node 22+)
node bench/fixtures-check.mjs          # every invalid fixture yields exactly its expected Diag
node bench/llm/gate.mjs                # first-pass rate must not drop vs bench/llm/baseline.json
scripts/check-english.sh               # no Spanish anywhere (accents or common Spanish words)
target/debug/brain doctor              # inside any app
```

## Invariants (a change that breaks one is rejected)
1. **No third-party parsers, bundlers, minifiers or compressors.** Rust deps only for low-level I/O (`notify`).
   Runtime: zero dependencies, no polyfills.
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
