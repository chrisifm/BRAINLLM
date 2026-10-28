# Conformance matrix — served SPEC vs. tests

- **Served SPEC:** `./target/debug/brain spec` sha256 `0728f54b98db` · `SPEC.md` sha256 `e1d74f160101` (first 12 chars).
  Previous audit: `SPEC.md` `4dbf7192e53c`, 139 rows (99 ✅ · 0 ❌ · 15 ⚠️ · 25 🔎).
- **Date:** 2026-10-13 (re-measure, task #101)
- **Scope:** only the text outside `<!--planned:#N-->…<!--/planned-->` blocks (the served spec). Rows 1–139 keep their
  ids; rows 140+ are rules the served SPEC gained since (catch-all routes, route files, router, lifecycle helpers, kit,
  forms, data, caching, server env, `brain start`, base path, SEO from `brain.json`).
- **States:** ✅ tested and green · ❌ the test exists and fails · ⚠️ no test, or the claim only partly holds (the
  note says whether it **diverges**: a manual try shows the claim does not hold) · 🔎 verified manually with a command
  only. A bench case (`bench/<name>/run.mjs <case id>`) counts as a test: each bench exits 1 on regression.

## Suite results (2026-10-13)

| Suite | Result |
|---|---|
| `cargo test --workspace --no-fail-fast` | **379 passed, 0 failed** (77 targets, doc-test targets included). The stale `views__dashboard.brain.out` snapshot of the previous audit is green |
| `npm test --prefix runtime` | **206/206** passed |
| `node bench/fixtures-check.mjs` | **44/44** invalid fixtures OK (was 18); valid examples: 0 diagnostics |
| `node bench/scopes/run.mjs` | **PASS**, 12/12 rules in dev and prod (brain `4cca4e74f4c3`) |
| `node bench/precision/run.mjs` | **PASS**, P1–P8 in dev and prod |
| `node bench/http/run.mjs` | **PASS** 51 · FAIL 0 · BLOCKED 0 (S01–S24 dev+prod, C1–C3) |
| `node bench/cache/run.mjs --report-only` | last results (binary `4695ea13a161`): every verdict item PASS, opt-outs 4/4, build signature 5/5 |
| `bench/start/results/summary.md` (not re-run: release build, about 4 min) | 10/10 cases PASS (binary `e5d861f5a9f3`), leftover processes: none |
| `brain doctor --json` on a fresh `brain new` app | `ok: true`, check 0 errors, tests 5/5, `a11y: []` |

## Totals

| State | Meaning | Rows |
|---|---|---|
| ✅ | tested and green | 163 |
| ❌ | test exists and fails | 0 |
| ⚠️ | no test or partly holds (most **diverge**) | 10 |
| 🔎 | verified manually only | 28 |
| | **Total** | **201** |

Former ⚠️ rows now ✅: 22, 23, 24, 37, 93, 99 (`crates/brain-check/tests/conformance.rs`, task #97), 137 (SRI + CSP).
Former ⚠️ rows now 🔎 (they work, no test calls them): 14, 15, 19 (`brain add` view path, layout, lib).
Former 🔎 rows now ✅: 95, 125, 126. Still ⚠️ from the previous audit: 20 (partly), 116, 127 (partly), 135, 136 (partly).

## How to regenerate

```bash
cargo build -p brain-cli
./target/debug/brain spec | shasum -a 256 | cut -c1-12   # served sha (header)
shasum -a 256 SPEC.md | cut -c1-12                    # source sha (header)
cargo test --workspace --no-fail-fast                 # Rust: lex/parse/check/codegen/css/min/pack/serve/seo/mcp/cli
npm test --prefix runtime                             # runtime (node:test)
node bench/fixtures-check.mjs                         # examples/invalid/*.expected.json vs `brain check --json`
node bench/scopes/run.mjs; node bench/precision/run.mjs; node bench/http/run.mjs   # QA gates, exit 1 on regression
node bench/cache/run.mjs --report-only                # last cache results; bench/start/results/summary.md for start
python3 -c "import re,collections as C; print(C.Counter(re.split(r'(?<!\\\\)\\|',l)[5].strip() for l in open('bench/conformance.md') if re.match(r'^\\| \\d+ \\|',l)))"   # totals
# 🔎 and ⚠️ rows are reproduced in a scratch app (e.g. ~/tmp/brainllm-conf101): `brain new app`, write the minimal
# case from the Note column and run `brain check --json <file>`, `brain build`, `brain doctor --json` or `brain add …`.
```

Path convention: `crates/<crate>/tests/x.rs::fn` for integration tests, `crates/<crate>/src/x.rs::fn` for internal
`#[test]`s, `runtime/test/x.test.js::'name'`, `fixture:<case>` = `examples/invalid/**/<case>.brain` +
`.expected.json`, checked by `crates/brain-check/tests/fixtures.rs::invalid_fixtures_match_expected` and by
`bench/fixtures-check.mjs`. `channels-app` = `bench/fixtures-pending/channels-app`. A short path after the first
mention of a file repeats that file.

## Matrix

| # | Section | Claim | Proof (path::test or command) | State | Note |
|---|---|---|---|---|---|
| 1 | Files | `app.brain` entry: `<html lang>`, common `<head>`, layout, `<slot />` | `crates/brain-cli/src/scaffold.rs::new_then_add`, `::templates_pass_check` | ✅ | Scaffold output only; removing `lang` from `<html>` gives no diagnostic (🔎). |
| 2 | Files | No `.html` anywhere | `scaffold.rs::new_then_add` (`!index.html`) | ✅ | |
| 3 | Files | `views/*.brain` routes: `users.brain` → `/users` | `crates/brain-serve/src/route.rs::static_dynamic_layouts_404`, `crates/brain-cli/src/map.rs::routes_from_files` | ✅ | Tests use `users/index.brain`; a flat `views/users.brain` is served at `/users` and prerendered to `dist/users/index.html` (🔎). |
| 4 | Files | `views/users/[id].brain` → `/users/:id` (`ctx.params.id` in `load`) | `route.rs::static_dynamic_layouts_404`, `map.rs::routes_from_files`, `crates/brain-serve/src/load.rs::sidecar_runs_load_with_ctx`, `crates/brain-codegen/tests/llm_gaps.rs::dynamic_route_names_and_load_only_imports` | ✅ | |
| 5 | Files | `views/users/_layout.brain` wraps its folder | `route.rs::static_dynamic_layouts_404`, `crates/brain-check/tests/llm_gaps.rs::form_fields_layouts_and_routes` | ✅ | |
| 6 | Files | `views/404.brain` | `route.rs::static_dynamic_layouts_404`, `map.rs::routes_from_files` (`*`) | ✅ | |
| 7 | Files | `comp/*.brain` stateful · `ui/*.brain` stateless | `crates/brain-check/tests/rules.rs::reactivity_rules` (B0116) | ✅ | |
| 8 | Files | `lib/*.jsbrain` logic, no markup | — | 🔎 | Markup in a `.jsbrain` still gives a generic B0001 ("unclosed regular expression"), not a dedicated rule. |
| 9 | Files | `tokens.cssbrain` design tokens (root) | `crates/brain-check/tests/tokens_file.rs::cssbrain_and_legacy_token_files` | ✅ | |
| 10 | Files | Import only files that exist | `rules.rs::markup_rules` (B0119), `crates/brain-check/tests/llm_gaps.rs::b0119_lists_existing_files`, `crates/brain-check/tests/edits.rs::b0119_edit_points_to_the_existing_file` | ✅ | |
| 11 | Files | Primitives need no import | `brain-check/tests/llm_gaps.rs::primitives_and_ui_kit_need_no_import`, `brain-codegen/tests/llm_gaps.rs::auto_imports_and_field_binds` | ✅ | |
| 12 | Files | `*.test.brain` tests next to the component | `brain doctor` / `brain test` on a scratch app | 🔎 | 5/5 green (`comp/counter`, `comp/user-card`, `views/login`); neither `cargo test` nor `npm test` runs `brain test`. |
| 13 | Files | Name = file in PascalCase (`user-card.brain` → `UserCard`) | `crates/brain-cli/src/names.rs::conversions`, `scaffold.rs::new_then_add`, `brain-codegen/tests/llm_gaps.rs::dynamic_route_names_and_load_only_imports` | ✅ | |
| 14 | Architecture | `brain add view users/[id]` → `views/users/[id].brain` | — | 🔎 | Fixed: `{"file":"views/users/[id].brain"}`; `brain check` `[]`. No test calls it. |
| 15 | Architecture | `brain add layout users` → `views/users/_layout.brain` | — | 🔎 | Fixed: creates `views/users/_layout.brain` with `<slot />`. No test calls it. |
| 16 | Architecture | `brain add comp user-card --props "user:obj"` → `comp/user-card.brain` | `scaffold.rs::new_then_add`, `::templates_pass_check` | ✅ | |
| 17 | Architecture | `brain add ui badge` → `ui/badge.brain` | `scaffold.rs::templates_pass_check` (as `micro`) | 🔎 | Tests use the `micro` alias; `brain add ui badge2` creates `ui/badge2.brain`. |
| 18 | Architecture | `brain add test user-card` → `comp/user-card.test.brain` | — | 🔎 | `brain add comp` already creates it; a second `brain add test` answers "already exists; fix: edit it". No test calls `add test`. |
| 19 | Architecture | `brain add lib format` → `lib/format.jsbrain` | — | 🔎 | Fixed: creates `lib/format.jsbrain` (`export default {}`). No test calls it. |
| 20 | Architecture | Generated files carry contract `<meta>`, typed stubs, a11y-correct markup, a colocated test | `scaffold.rs::templates_pass_check` (all templates pass `brain check`; asserts `comp/user-card.test.brain` after `add comp`) | ⚠️ | **Diverges** partly: only `add comp` creates the colocated test; `add view`, `add ui`, `add layout` and `add lib` create none (the SPEC table says `test` is "for a component or view"). |
| 21 | Architecture | Files kebab-case, components PascalCase | `scaffold.rs::new_then_add` (`UserCard` → `comp/user-card.brain`) | ✅ | |
| 22 | Architecture | One export per `.jsbrain` | `crates/brain-check/tests/conformance.rs::row22_one_export_per_module`, `fixture:b0124-two-exports` | ✅ | B0124 (error). `types.jsbrain` is the documented exception. |
| 23 | Component | Blocks in this order (meta, script, markup, style) | `conformance.rs::row23_block_order`, `fixture:b0125-block-order` | ✅ | B0125 is **info** and the compiler reorders the blocks. |
| 24 | Component | All blocks optional except markup | `conformance.rs::row24_markup_is_required`, `fixture:b0126-no-markup` | ✅ | B0126 (error). |
| 25 | Component | Props `name:type=default` | `crates/brain-parse/tests/errors.rs::expressions_and_blocks` | ✅ | |
| 26 | Component | Props `?` = optional | `rules.rs::markup_rules` | ✅ | |
| 27 | Component | Props without `?` nor default = required | `rules.rs::markup_rules` (B0207) | ✅ | |
| 28 | Component | Types: str num bool list obj fn any (others rejected) | `errors.rs::markup_structure` (B0004), `scaffold.rs::props_errors_have_fix` | ✅ | `n:int` → B0004 (🔎). |
| 29 | Component | `value:str~` bindable prop: the child may write it | `crates/brain-check/tests/channels.rs::model_props_and_component_binds`, `crates/brain-codegen/tests/channels_e2e.rs::eight_channels_run_on_the_runtime` | ✅ | |
| 30 | Component | `emits="select:obj close"` | `channels.rs::each_invalid_case_gives_its_code` (B0802, B0803), `channels_e2e.rs::eight_channels_run_on_the_runtime` | ✅ | |
| 31 | Component | `expose="open"` | `channels.rs::each_invalid_case_gives_its_code` (B0804), `runtime/test/ch.test.js::'expose: the parent receives the frozen API and null when the child unmounts'` | ✅ | |
| 32 | BrainScript | Allowed: let const · arrow/function · if/else · for-of · return · await · objects arrays spread destructuring · template strings | `errors.rs::expressions_and_blocks` | ✅ | |
| 33 | BrainScript | Allowed: try/catch/finally · throw | — | 🔎 | `try { throw … } catch … finally …` in a component → `[]`. |
| 34 | BrainScript | JS built-ins and Web APIs allowed | `rules.rs::script_rules` | ✅ | |
| 35 | BrainScript | `import x from './a.brain'\|'./a.jsbrain'\|'./a.js'` | `crates/brain-codegen/tests/e2e.rs::counter_and_todo_run_on_the_runtime`, `crates/brain-codegen/tests/server.rs::server_code_is_split_from_the_client` | ✅ | |
| 36 | BrainScript | `export … load` only in views | `rules.rs::reactivity_rules` (B0117) | ✅ | |
| 37 | BrainScript | `export default`/`export const\|function` only in `.jsbrain` (and `load` in views) | `conformance.rs::row37_export_only_in_modules`, `fixture:b0120-export-const` | ✅ | B0120 (error). |
| 38 | BrainScript | Forbidden: `class` | `errors.rs::prohibited_constructs` (B0101), `::positions_are_exact` | ✅ | |
| 39 | BrainScript | Forbidden: `this` | `rules.rs::script_rules` (B0102), `crates/brain-cli/src/compiler.rs::real_checker_reports_this` | ✅ | |
| 40 | BrainScript | Forbidden: `var` | `rules.rs::script_rules` (B0103), `edits.rs::mechanical_fixes_apply` | ✅ | |
| 41 | BrainScript | Forbidden: `with` | `errors.rs::prohibited_constructs` (B0104) | ✅ | |
| 42 | BrainScript | Forbidden: `eval` / `new Function` | `rules.rs::script_rules`, `errors.rs::prohibited_constructs` (B0105) | ✅ | |
| 43 | BrainScript | Forbidden: generators | `errors.rs::prohibited_constructs` (B0106) | ✅ | |
| 44 | BrainScript | Forbidden: get/set | `errors.rs::prohibited_constructs` (B0107) | ✅ | |
| 45 | BrainScript | Forbidden: labels | `errors.rs::prohibited_constructs` (B0108) | ✅ | |
| 46 | BrainScript | Forbidden: `for-in` | `errors.rs::prohibited_constructs` (B0109) | ✅ | |
| 47 | BrainScript | Forbidden: `for(;;)` | `errors.rs::prohibited_constructs` (B0110) | ✅ | |
| 48 | BrainScript | Forbidden: `while` | `errors.rs::prohibited_constructs` (B0110) | ✅ | |
| 49 | BrainScript | Forbidden: `switch` | — | 🔎 | B0110 with a useful fix; no test covers it. |
| 50 | BrainScript | Signals are plain variables: `n++`, `items = [...items, x]` | `rules.rs::reactivity_rules`, `runtime/test/compiled.test.js::'compiled counter: props, insert, drv, cls and delegated click'` | ✅ | |
| 51 | BrainScript | Never mutate in place (`items.push` is an error) | `rules.rs::reactivity_rules` (B0113, B0114), `edits.rs::mechanical_fixes_apply` | ✅ | |
| 52 | Primitives | `sig(v)` reactive state | `runtime/test/signals.test.js::'sig is read and written through .v'` | ✅ | |
| 53 | Primitives | `drv(() => e)` derived, read-only | `signals.test.js::'drv is lazy, memoized and read-only'`, `rules.rs::reactivity_rules` (B0112) | ✅ | |
| 54 | Primitives | `fx(() => { …; return cleanup })` | `signals.test.js::'fx cleanup runs before re-running and on dispose'` | ✅ | |
| 55 | Primitives | `export async function load(ctx)` views only, runs on server; result arrives as `props` | `codegen/tests/server.rs::server_code_is_split_from_the_client`, `load.rs::sidecar_runs_load_with_ctx`, `rules.rs::reactivity_rules` (B0117) | ✅ | |
| 56 | Primitives | `emit('ev', data?)` → `on:ev` on the parent | `e2e.rs::counter_and_todo_run_on_the_runtime`, `rules.rs::reactivity_rules` (B0118) | ✅ | |
| 57 | Primitives | `ref()` + `bind:ref={r}` DOM node access | — | 🔎 | Scratch `brain test`: an `fx` reads `r.tagName` → `SPAN`. |
| 58 | Primitives | Never `document`/`window` | `fixture:b0201-document` | ✅ | `window.foo = 1` gives B0201 at level **warning** (🔎). |
| 59 | Primitives | `log(...)` dev log, removed in build | `crates/brain-min/src/mangle.rs::drop_log` (unit) | 🔎 | `brain build` on a scratch app: `fx(() => log('HELLO_LOG_QQ', n))` becomes `f(()=>void 0)`, the string is absent from `dist/`. No end-to-end build test asserts it. |
| 60 | Primitives | `provide(K, v)` · `use(K)` · `key('name')`; `K` exported from channels/ | `ch.test.js::'context: the descendant reads what the ancestor provided, with a reactive signal and key default'`, `::'context: scope isolates siblings and use without provide throws B0807'`, `channels.rs::each_invalid_case_gives_its_code` (B0805, B0807) | ✅ | |
| 61 | Primitives | `store(init, {sync?:'tab'\|'storage'})` · `bus('name')` only in `channels/*.jsbrain` | `ch.test.js::'store sync storage: …'`, `::'store sync tab: …'`, `::'bus: …'`, `channels.rs::each_invalid_case_gives_its_code` (B0801, B0806) | ✅ | `bus()` in `comp/` → B0806 (🔎). |
| 62 | Markup | `{expr}` interpolation | `compiled.test.js::'compiled counter: …'` | ✅ | |
| 63 | Markup | `attr={e}` | `runtime/test/dom.test.js::'attr, cls, style: and properties'` | ✅ | |
| 64 | Markup | `class:x={bool}` | `compiled.test.js::'compiled counter: …'`, `channels_e2e.rs` (`class:dark`) | ✅ | |
| 65 | Markup | `style:prop={v}` | `dom.test.js::'attr, cls, style: and properties'` | ✅ | Runtime API only. |
| 66 | Markup | `on:ev={fnRef \| statement}` | `e2e.rs::counter_and_todo_run_on_the_runtime`, `rules.rs::markup_rules` (B0208) | ✅ | |
| 67 | Markup | `on:submit` prevents default | `crates/brain-codegen/tests/e2e.mjs` (`defaultPrevented`) via `e2e.rs` | ✅ | |
| 68 | Markup | `bind:value\|checked\|ref={x}` | `e2e.rs`, `rules.rs::markup_rules` (B0204, B0205) | ✅ | `bind:ref`: row 57. |
| 69 | Markup | `bind:value\|checked` accepts a field of a sig object | `brain-check/tests/llm_gaps.rs::bind_to_a_field_of_a_sig`, `brain-codegen/tests/llm_gaps.rs::auto_imports_and_field_binds` | ✅ | |
| 70 | Markup | `{#if c}…{:else if d}…{:else}…{/if}` | `errors.rs::expressions_and_blocks`, `dom.test.js::'when: switches branches, …'` | ✅ | |
| 71 | Markup | `{#each list as it (it.id)}` keyed | `dom.test.js::'list: …'` (7 tests), `compiled.test.js::'compiled todo: submit, keyed list, toggle, filters and empty'` | ✅ | |
| 72 | Markup | Each key required | `rules.rs::markup_rules` (B0202), `edits.rs::mechanical_fixes_apply` | ✅ | |
| 73 | Markup | List of primitives → key by value | `rules.rs::each_key_fix_depends_on_item_shape` | ✅ | |
| 74 | Markup | `{:empty}` | `dom.test.js::'list: empty block and anchor between static siblings'` | ✅ | |
| 75 | Markup | `{#await p}…{:then v}…{:catch e}…{/await}` | `errors.rs::expressions_and_blocks` (parse only) | 🔎 | Scratch `brain test`: resolved → `then`, rejected → `catch`. No runtime test. |
| 76 | Markup | `<slot />` | `channels_e2e.rs` (`ui/panel.brain` default slot) | ✅ | |
| 77 | Markup | `<slot name="x" />` | `channels_e2e.rs` (`footer` slot non-empty) | ✅ | Weak assertion (non-empty only); 🔎 `brain build` prerender puts `slot="head"` content only in the named slot. |
| 78 | Markup | `<Comp a={b} on:ev={f} />` | `errors.rs::expressions_and_blocks`, `rules.rs::markup_rules` (B0203, B0206, B0207), `e2e.rs` | ✅ | |
| 79 | Markup | Text interpolation is escaped | — | 🔎 | Scratch `brain test`: `{s}` with `<b>hi</b>` → 0 `b` nodes, literal text. |
| 80 | Markup | Static `id`s in `comp/` and `ui/` get a per-instance suffix (`for`, `aria-*`, `href="#…"` follow) | `crates/brain-codegen/tests/snapshots.rs::static_ids_get_an_instance_suffix`, `crates/brain-codegen/tests/raw_html_svg.rs::svg_references_follow_the_instance_id`, `::svg_references_resolve_on_the_runtime`, `bench/scopes/run.mjs 6` | ✅ | |
| 81 | Communication | 1 props · 2 `emit` + `on:` · 3 slots | `channels_e2e.rs::eight_channels_run_on_the_runtime` | ✅ | |
| 82 | Communication | 4 `~` prop + `bind:value` | `channels.rs::model_props_and_component_binds`, `channels_e2e.rs` | ✅ | |
| 83 | Communication | 5 `expose` + `bind:api={m}` + `m.open()` | `channels_e2e.rs`, `channels.rs::each_invalid_case_gives_its_code` (B0804) | ✅ | |
| 84 | Communication | 6 `provide`/`use` | `channels_e2e.rs`, `ch.test.js::'context: …'` | ✅ | |
| 85 | Communication | 7 `store` in channels/ | `channels_e2e.rs`, `ch.test.js::'store: reactive getter/setter per field, batched $set and $snap'` | ✅ | |
| 86 | Communication | 8 `bus` in channels/ | `channels_e2e.rs`, `ch.test.js::'bus: …'` | ✅ | |
| 87 | Communication | No globals, no `window.x` | — | 🔎 | B0201 on `window.foo = 1`, but only as a **warning**; the fixture covers `document` only. |
| 88 | CSS | Native CSS + nesting | `crates/brain-css/src/lib.rs::tests::parses_nesting_and_decls` | ✅ | |
| 89 | CSS | `@mixin n {}` / `@include n;` | `brain-css/src/lib.rs::tests::mixins_expand_and_unknown_is_b0303`, `rules.rs::css_tokens_and_mixins` | ✅ | |
| 90 | CSS | Scoped to the component automatically | `brain-css/src/lib.rs::tests::scoped_output_hoists_keyframes`, `::root_anchoring_keeps_combinators_and_lists`, `compiler.rs::codegen_dev_output` | ✅ | |
| 91 | CSS | No global selectors | `fixture:b0301-global-selector`, `brain-css/src/lib.rs::tests::global_selectors_are_found` | ✅ | |
| 92 | CSS | Only the declared tokens exist (`var(--x)` unknown → error) | `rules.rs::css_tokens_and_mixins` (B0302), `llm_gaps.rs::b0302_suggests_the_closest_token`, `edits.rs::mechanical_fixes_apply` | ✅ | |
| 93 | CSS | Color properties (color, background, border-color, outline-color, fill, stroke) and spacing (padding, margin, gap, inset) only via tokens; sizes are free | `conformance.rs::row93_colors_and_spacing_from_tokens`, `fixture:b0304-color-literal`, `fixture:b0305-spacing-literal` | ✅ | B0304 (error) for colors; B0305 is **info** and normalizes a literal equal to a token (`4px` → `--space-1`). |
| 94 | CSS | Token set `--space-1..6 --radius --font --fg --muted --bg --surface --line --accent --on-accent --ok --bad` | `scaffold.rs::scaffold_tokens_contrast` | ✅ | |
| 95 | CSS | Tokens `--radius-sm --radius-lg --glass --glass-edge --shadow --scrim` | `scaffold.rs::scaffold_tokens_contrast` (lists all 23 tokens), `crates/brain-check/tests/rules.rs::scrim_token_for_old_apps_and_synonyms` | ✅ | `examples/tokens.cssbrain` still has only 10 tokens (`--space-1..3`, `--radius`, `--fg`, `--muted`, `--bg`, `--accent`, `--ok`, `--bad`). |
| 96 | CSS | Text color pairs (AA) | `scaffold.rs::scaffold_tokens_contrast`, `fixture:b0408-contrast` | ✅ | |
| 97 | CSS | `--line` is for borders only (never a background) | — | ⚠️ | **Diverges**: `background: var(--line)` → `[]`. `color: var(--line)` on `--bg` is only caught by contrast (B0408). |
| 98 | CSS | No `color-mix()` on text color | `fixture:b0408-color-mix`, `rules.rs::contrast_resolves_color_mix_and_rejects_unverifiable` | ✅ | The test accepts a `color-mix` that resolves to AA: looser than the letter of the SPEC. |
| 99 | CSS | No opacity on text color | `conformance.rs::row99_no_opacity_on_text_color`, `fixture:b0408-opacity-text` | ✅ | B0408 "the contrast can no longer be verified". |
| 100 | CSS | Animations inside `prefers-reduced-motion: no-preference` | `fixture:b0409-reduced-motion` | ✅ | `transition` outside the `@media` is not reported (🔎). |
| 101 | A11y | img has alt | `fixture:b0401-img-alt` | ✅ | |
| 102 | A11y | clicks only on button/a | `fixture:b0402-click-noninteractive` | ✅ | |
| 103 | A11y | Use `<Input label>` `<Select label>` from the kit; a raw input/select/textarea needs a `<label>` | `fixture:b0403-input-no-label`, `crates/brain-cli/src/testrun.rs::ax_rules_map_to_spec_codes` | ✅ | `select`/`textarea` only 🔎 (B0403). |
| 104 | A11y | No heading level skips | `fixture:b0404-heading-skip` | ✅ | |
| 105 | A11y | One h1 per view | `fixture:b0405-two-h1` | ✅ | |
| 106 | A11y | One main per view | `fixture:b0405-two-main` | ✅ | |
| 107 | A11y | Buttons have text | `fixture:b0406-button-no-text`, `testrun.rs::ax_rules_map_to_spec_codes` | ✅ | |
| 108 | A11y | No tabindex > 0 | `fixture:b0407-tabindex` | ✅ | |
| 109 | A11y | Build fails otherwise | — | 🔎 | Re-checked: `<img>` without alt in `views/about.brain` → `brain build` exits 2 ("does not compile: B0401"). No test. |
| 110 | SEO | Every view: `<meta seo title description>` | `fixture:b0501-no-seo`, `llm_gaps.rs::form_fields_layouts_and_routes` | ✅ | |
| 111 | SEO | Optional `canonical? og:image? schema? robots?` | `crates/brain-pack/src/prerender.rs::seo_tags`, `::jsonld_and_title_template`, `crates/brain-check/tests/seo_rules.rs::b16xx_metadata` | ✅ | `crates/brain-cli/src/seo.rs` (cited before) was removed; SEO moved to `brain-seo`. |
| 112 | Runtime errors | Errors in `fx`, `drv` and handlers are caught; the app stays alive and the error is reported | `runtime/test/errors.test.js::'failing fx: …'`, `::'failing handler (§42): …'`, `::'no fallback: the error is reported and the app keeps running'`, `runtime/test/guard.test.js::'handler errors (sync and a rejected await) keep the component and show a toast'`, `signals.test.js::'an error in an effect does not leave the system stuck'`, `bench/scopes/run.mjs 9` | ✅ | `drv` throw is covered by the scopes bench ("Boom failed Retry"), not by a node test. |
| 113 | Runtime errors | Effect re-running > 100 times in one update is stopped (B0902) | `errors.test.js::'B0902: a sig→fx→sig cycle is cut, reported once and the other effects keep running'` | ✅ | |
| 114 | Runtime errors | Per-component fallback: `<meta fallback="…">` or `{:catch}` | `brain-codegen/tests/llm_gaps.rs::meta_fallback_wraps_the_component`, `errors.test.js::'failing fx: the component shows its fallback …'`, `::'failing render: …'` | ✅ | 🔎 scratch `brain test`: a throwing `fx` replaces the component with its fallback. |
| 115 | Runtime errors | Captured errors (message, stack, recent actions) listed by `brain_errors` (MCP) | `errors.test.js::'breadcrumbs without values: no inputs, query strings or signals'`, `::'dedupe by fingerprint and sendBeacon send cap'`, `runtime/test/errmap.test.js` (2) | 🔎 | Capture is tested; the `brain_errors` tool (`crates/brain-cli/src/mcp.rs:767`) is registered but no test calls it. |
| 116 | Runtime errors | Captured errors listed by `brain doctor` | — | ⚠️ | **Diverges**: `brain doctor --json` has `check`, `test`, `seo`, `advice`, `heap`; no captured-errors section. |
| 117 | Server | `load` runs only on the server | `codegen/tests/server.rs::server_code_is_split_from_the_client` | ✅ | |
| 118 | Server | Client code may only read env vars prefixed `PUBLIC_` | `crates/brain-check/tests/server.rs::env_vars_b0702`, `fixture:b0702-private-env` | ✅ | |
| 119 | Tests | `test()` · `await mount(Comp, props)` · `await click(sel)` · `await type(sel, text)` | `examples/comp/*.test.brain` via `brain test` | 🔎 | 6/6 green; outside `cargo test`/`npm test`. |
| 120 | Tests | `text(sel)` `attr(sel, n)` `hasClass(sel, c)` `count(sel)` | `examples/comp/*.test.brain` | 🔎 | |
| 121 | Tests | `expect(x).toBe(y)\|toEqual(y)\|toBeTruthy()` | `examples/comp/*.test.brain` (`toBe`) | 🔎 | `toEqual` and `toBeTruthy` only in the scratch probe (green). |
| 122 | CLI | Also MCP: `brain mcp` | `crates/brain-mcp/src/lib.rs::tests::session`, `::protocol_errors`, `::structured_resources_prompts`, `crates/brain-mcp/src/json.rs::tests::roundtrip` | ✅ | |
| 123 | CLI | Every command accepts `--json` | `crates/brain-cli/tests/stdin.rs::commands_exit_with_stdin_open` (`check --json`, `map --json`) | 🔎 | By hand: `add`, `build`, `spec`, `map`, `explain`, `fix`, `doctor`, `check`, `test`, `schema`, `card`, `status`, `seo audit`, `--version` all print JSON. |
| 124 | CLI | `brain new <app>` | `scaffold.rs::new_then_add`, `::templates_pass_check` | ✅ | |
| 125 | CLI | `brain run` (dev server with hot reload, app :5180) | `crates/brain-cli/tests/base.rs::dev_serves_everything_under_the_base` (spawns `brain run`), `runtime/test/hmr.test.js` (3), `compiled.test.js::'compiled counter in dev: …'`, `crates/brain-cli/tests/contract.rs::run_prints_an_impact_record_per_save` | ✅ | Default port 5180 only 🔎 (the test uses `--port 0`). `brain new` now prints "cd app && brain run" (fixed). |
| 126 | CLI | `brain build` (production) | `stdin.rs::commands_exit_with_stdin_open` (runs `brain build`), `crates/brain-pack/tests/*.rs` (base, guards, kit, seo_paths, ssr, workers build real apps) | ✅ | |
| 127 | CLI | `brain doctor` (check + tests + over-engineering + leaks; non-zero on error) | `crates/brain-check/tests/doctor.rs::doctor_app_matches_expected` (over-engineering) | ⚠️ | **Diverges** partly: check, tests, SEO and over-engineering (`advice`, e.g. B1007) are there; leaks print `"heap":{"skipped":"leak detection (--heap) arrives with task #24"}`. |
| 128 | CLI | `brain add view\|comp\|ui <name> --props "…"` | `scaffold.rs::new_then_add`, `::templates_pass_check`, `::props_errors_have_fix` | ✅ | |
| 129 | CLI | `brain check --json` | `rules.rs::diag_json_is_spec_shape`, `fixtures.rs::valid_examples_are_clean`, `::invalid_fixtures_match_expected`, `bench/fixtures-check.mjs` | ✅ | |
| 130 | CLI | `brain test` | `testrun.rs::ax_rules_map_to_spec_codes` (AX mapping only) | 🔎 | No end-to-end test; by hand it runs in headless Chrome and reports JSON + `a11y`. |
| 131 | CLI | `brain spec [--topic x]` | `crates/brain-cli/src/spec.rs::tests::planned_blocks_are_removed`, `::broken_markers_fail_loudly`, `::real_spec_keeps_every_public_section`, `::every_topic_resolves_and_is_smaller` | ✅ | |
| 132 | CLI | `brain fix` applies every `edits` from `brain check` and re-checks | `edits.rs::mechanical_fixes_apply` (edits only) | 🔎 | `brain fix --json` on `var n = 1` rewrites to `let` and reports `remaining: []`. The command has no test. |
| 133 | CLI | `brain map --json`: routes, components, props, channels, tests | `map.rs::routes_from_files`, `::exports_and_tests`, `crates/brain-cli/tests/contract.rs::map_reverse_graph_filters_and_pages` | 🔎 | Output keys: `components env modules reverse routes tests tokens total`; channels appear as `modules` entries, there is no `channels` key. |
| 134 | Build | Minifies and mangles | `brain-min/src/print.rs` and `mangle.rs` unit tests, `brain-pack/src/lib.rs::css_minifier_keeps_meaning`, `::css_shortest_forms` | 🔎 | `brain build` output is minified with short names (`const b=d((g.start??0))`); no end-to-end assertion. |
| 135 | Build | Obfuscates | — | ⚠️ | **Diverges**: only mangling. Class names keep their source names in `dist/` (`class="users"`, `"brand"`); the `--protect=std` layer of INTERNALS is not implemented (no `protect` in `brain-pack`/`brain-min`). |
| 136 | Build | Removes dead code | `crates/brain-min/src/mangle.rs::drops_unused_exports_and_their_helpers`, `::keeps_side_effects_and_transitive_uses` (unit) | ⚠️ | **Diverges** partly: `brain-pack` now tree-shakes modules (`crates/brain-pack/src/lib.rs:533`, `:556`), but an unused local `const deadFnUnusedQq = () => 'DEAD_FN_MARK'` in a component survives in `dist/_b/*.js` (B1009 only warns). |
| 137 | Build | Adds SRI + a strict CSP with the security headers to every page | `crates/brain-pack/src/cache.rs::integrity_and_csp_cover_every_script`, `::headers_one_rule_per_path`, `crates/brain-cli/tests/start.rs::static_mode_applies_headers_and_signature`, `runtime/test/server.test.js::'BUILD #177: the manifest's CSP is used as is, …'` | ✅ | Fixed. 🔎 `dist/`: `integrity="sha384-…"`, CSP with hashes, HSTS, `nosniff`, `X-Frame-Options`, `Permissions-Policy`, COOP in `_headers`. |
| 138 | Loop | Error shape `{"code","file","line","col","msg","fix","edits"}`; `edits` exact replacements; families B00 · B01 · B02 · B03 · B04 · B05 · B07 | `rules.rs::diag_json_is_spec_shape`, `edits.rs::mechanical_fixes_apply`, `::judgment_fixes_have_no_edits`, `errors.rs::markup_structure`, `server.rs::server_imports_b0701`, `::secrets_b0703`, fixtures | ✅ | Diagnostics also carry `level`. Reference component (SPEC §Reference) extracted to a scratch file → `brain check --json` = `[]` (🔎). |
| 139 | Markup | Raw HTML only as `{@html sanitize(x)}` | `crates/brain-codegen/tests/raw_html_svg.rs::raw_html_without_sanitize_is_b0210`, `::raw_html_compiles_to_html_from_brain_data`, `runtime/test/data.test.js::'html(): without sanitize() the value is inserted as text, never parsed'`, `::'fallback sanitizer: drops scripts and handlers, unwraps unknown tags, filters URLs'` | ✅ | `{@html s}` without `sanitize` → B0210 (🔎). |
| 140 | Files | `views/docs/[...rest].brain` matches `/docs/a/b/c`, `ctx.params.rest = 'a/b/c'`, ranked after static and `[id]` | `crates/brain-pack/src/routes.rs::canonical_order_and_resolution`, `::files_to_patterns`, `::regex_mirrors_matches`, `crates/brain-pack/src/lib.rs::catch_all_pages_from_paths`, `runtime/test/server.test.js::'#193 routes come from the manifest (rx, order): …'` | ✅ | |
| 141 | Files | `_guard.jsbrain`: `export default (ctx) => … redirect('/login')`, runs on the server and before client navigation | `crates/brain-codegen/tests/routes.rs::guard_redirect_and_deny_compile_to_plain_objects`, `crates/brain-pack/tests/guards.rs::guarded_routes_run_on_the_server`, `runtime/test/router.test.js::'guards (History): …'`, `::'guards (Navigation API): …'`, `::'#65 client guards share the server shapes: …'`, `runtime/test/server-shared.test.js::'runChain: …'` | ✅ | |
| 142 | Files | `deny('reason')` renders `views/403.brain` | `runtime/test/server-form.test.js::'#65 prod: guard deny → 403 page with the reason; …'`, `routes.rs::guard_redirect_and_deny_compile_to_plain_objects` | ✅ | |
| 143 | Files | `_error.brain` per folder | `server-form.test.js::'#65 prod: … load error → _error page'` | ✅ | |
| 144 | Files | `_middleware.jsbrain` per folder | `server-shared.test.js::'runChain: … middleware onion with headers; …'`, `server-form.test.js::'#65 prod: … allowed → page with the middleware headers …'` | ✅ | |
| 145 | Files | `ctx = { params, query, user, cookies, url }` | `server.test.js::'SSR: params + query reach load, …'`, `::'session: … ctx.user in RPC and load; …'`, `runtime/test/server-base.test.js` (`ctx.url.pathname`) | ✅ | `cookies` is verified only by reading `runtime/server/server.mjs:140` (🔎). |
| 146 | Files | `navigate('/x')` (router, no reload); never `location.assign` | `router.test.js::'#169 navigate()/route(): …'`, `crates/brain-check/tests/rules.rs::navigation_through_location` (B0129) | ✅ | B0129 is a **warning**. |
| 147 | Files | `route()` → `r.path`, `r.params.id`, `r.query.q` (reactive) | `router.test.js::'#169 navigate()/route(): … route() signals follow it'`, `::'#169 route() while the server renders: …'`, `server-base.test.js::'#169 route() during SSR: …'`, `crates/brain-codegen/tests/runtime_imports.rs::navigate_and_route_come_from_the_router` | ✅ | |
| 148 | Files | `brain new` creates `views/ comp/ ui/ channels/ services/ api/ lib/ server/ content/ public/`, `brain.json`, `.brain_env`, a README per folder | `scaffold.rs::new_then_add` (file count only), `::new_app_in_exact_folder` | 🔎 | `brain new app --json` lists every folder, both files and the READMEs; the test asserts only the count (39). |
| 149 | Files | Never create other top-level folders | — | ⚠️ | **Diverges**: no rule; `foo/x.brain` at the app root → `brain check` `[]`. |
| 150 | Markup | Ids in views and layouts stay as written, so `/page#id` links work | `crates/brain-codegen/tests/raw_html_svg.rs::view_ids_are_public_fragments_without_suffix` | ✅ | |
| 151 | Markup | `<pre raw>` and `<code raw>` keep their text literal | `raw_html_svg.rs::raw_code_blocks_are_literal_text`, `::raw_samples_are_opaque_to_block_scanning_and_keep_whitespace` | ✅ | |
| 152 | Lifecycle | `mounted(() => { …; return cleanup })` once after the DOM is in the page | `runtime/test/env.test.js::'mounted: runs after the nodes are in the document; …'` | ✅ | |
| 153 | Lifecycle | `watch(() => q, (now, prev) => …)` only when `q` changes | `env.test.js::'watch: the callback runs on changes only (not the first read), …'` | ✅ | |
| 154 | Lifecycle | `await tick()` after DOM updates | `env.test.js::'watch: … tick waits for the DOM'` | ✅ | |
| 155 | Lifecycle | `every('30s', fn)` timer stopped on unmount | `env.test.js::'every: runs on an interval, pauses while the tab is hidden, stops on unmount'` | ✅ | |
| 156 | Lifecycle | `media()` `visible(ref)` `debounce(v, '300ms')` `online()` are read-only signals | `env.test.js::'media and visible: …'`, `::'debounce: …'`, `runtime/test/http.test.js::'offline: online() follows the events; …'`, `runtime_imports.rs::debounce_takes_a_getter`, `bench/http/run.mjs S22` | ✅ | |
| 157 | Lifecycle | `<component is={C}>` | `env.test.js::'dyn: mounts the current component, …'`, `runtime_imports.rs::portal_component_is_and_transition`, `crates/brain-check/tests/http.rs::portal_component_and_transitions` | ✅ | |
| 158 | Lifecycle | `<Portal>` | `env.test.js::'portal: children render into the target and leave with the component; …'`, `runtime_imports.rs::portal_component_is_and_transition` | ✅ | |
| 159 | Lifecycle | `transition:fade` on elements inside `{#if}`/`{#each}` | `runtime/test/anim.test.js` (2), `runtime_imports.rs::portal_component_is_and_transition`, `http.rs::portal_component_and_transitions` | ✅ | |
| 160 | Lifecycle | Writes refresh every `resource` of that path; `list.refetch()` forces it | `http.test.js::'resource: a write to the same path re-runs it; refetch() forces it; unmount cancels'`, `runtime/test/api.test.js::'resource: a write drops the cached GETs; …'`, `bench/http/run.mjs S16` | ✅ | |
| 161 | Lifecycle | `{:catch e}<ErrorState error={e} />` handles offline, 401, 403, 404, 422, 5xx | `runtime/test/kit.test.js::'#168 kit ErrorState: message and action per kind; …'`, `bench/http/run.mjs S21`, `S19` (401), `http.rs::catch_without_error_states` (B1105) | ✅ | |
| 162 | CSS | Modal and overlay backdrops: `background: var(--scrim)`, never `rgb(0 0 0 / x)` | `rules.rs::scrim_token_for_old_apps_and_synonyms`, `fixture:b0304-color-literal` | ✅ | `background: rgb(0 0 0 / .5)` → B0304 (🔎). |
| 163 | SEO | `<meta seo>` values may be expressions from `load` props | `crates/brain-pack/src/lib.rs::meta_seo_with_expressions_is_removed_whole`, `crates/brain-codegen/tests/seo_tools.rs::seo_and_head_reach_server_and_client` | ✅ | |
| 164 | SEO | `jsonld={{ '@type': … }}` | `prerender.rs::jsonld_and_title_template`, `seo_rules.rs::b16xx_metadata`, `fixture:b1603-jsonld-no-type` | ✅ | |
| 165 | SEO | Site defaults in `app.brain`: `<meta seo titleTemplate og:image twitter>` | `seo_tools.rs::title_template_from_app_brain_is_applied_at_compile_time` | ⚠️ | **Diverges** partly: `titleTemplate` ("About · app") and `og:image` reach every page, but `twitter="@nimbus"` on the `app.brain` `<meta seo>` emits no `twitter:site`; only `brain.json` `social.x` does. |
| 166 | SEO | Any other tag goes in a `<head>` block of the view (merged, deduplicated by name/property) | `prerender.rs::dedupes_head_last_wins`, `seo_tools.rs::head_block_escapes_interpolations_when_run`, `crates/brain-seo/tests/audit.rs::finish_rewrites_heads_and_generates_files` | ✅ | |
| 167 | SEO | `[id]` views list pages with `export async function paths()` → prerendered and in the sitemap | `crates/brain-pack/tests/seo_paths.rs::paths_seo_and_head_reach_every_page`, `seo_rules.rs::paths_only_in_dynamic_views`, `lib.rs::catch_all_pages_from_paths` | ✅ | |
| 168 | SEO | Sitemap date: `<meta seo lastmod>` or the collection's `date`; omitted otherwise | `crates/brain-seo/src/files.rs::robots_and_sitemap`, `audit.rs::finish_with_content_collections` | ✅ | `lastmod="2026-10-13"` on `views/about.brain` → `<lastmod>2026-10-13</lastmod>` in `dist/sitemap.xml` (🔎). |
| 169 | SEO | `brain build` generates robots.txt, sitemap.xml, llms.txt, llms-full.txt, manifest.webmanifest | `files.rs::robots_and_sitemap`, `::llms_files`, `::manifest_from_tokens_and_icons`, `audit.rs::finish_rewrites_heads_and_generates_files` | ✅ | |
| 170 | SEO | Google tags from `brain.json` `google` | `files.rs::google_consent_first` | ✅ | `"analytics": "G-TEST123"` → gtag script in `dist/about/index.html` (🔎). |
| 171 | SEO | `brain.json` `social` → the build writes every missing Open Graph/Twitter tag | `crates/brain-seo/src/social.rs::head_tags_fill_only_what_is_missing`, `audit.rs::finish_social_and_ai_layer` | ✅ | |
| 172 | SEO | `"seo": { "ai": "allow" \| "search-only" \| "block" }` AI crawler policy | `social.rs::robots_policy`, `audit.rs::finish_social_and_ai_layer` | ✅ | `search-only` → `ai-train=no` and `Disallow: /` for GPTBot, ClaudeBot… (🔎). |
| 173 | SEO | Never hand-write `og:*` defaults, robots.txt or llms.txt | — | 🔎 | `public/robots.txt` is shadowed by the built file with a build warning ("rename it"); no check rule. |
| 174 | Kit | Built-in `<Button> <Input> <Select> <Checkbox> <Dialog> <Table> <Tabs> <Toast> <Menu> <Field>`, no import | `crates/brain-check/tests/llm_gaps.rs::primitives_and_ui_kit_need_no_import`, `crates/brain-codegen/tests/kit.rs::every_kit_component_compiles_clean`, `crates/brain-pack/tests/kit.rs::kit_components_are_bundled` | ✅ | All ten exist in `runtime/ui/` (plus `error-state`, `lang-switch`). |
| 175 | Kit | Accessible, token-styled, every state (loading, disabled, error, empty) | `kit.test.js::'kit form: …'`, `::'kit Tabs and Table: …'`, `::'#168 kit ErrorState: …'` | ✅ | States are asserted for the form, Tabs, Table and ErrorState, not per component. |
| 176 | Kit | `form(schema, { submit })` → `f.v.x`, `f.err.x`, `f.busy`, `f.error`, `on:submit={f.submit}` | `runtime/test/form.test.js` (6), `kit.test.js::'kit form: …'`, `crates/brain-codegen/tests/kit.rs::form_reads_are_reactive`, `::form_reads_reach_kit_components_as_getters` | ✅ | |
| 177 | Kit | Schema `{ email: 'email!', age: 'num>=18', name: 'str!<=80' }`, types `str num bool email url date`, `[T]`, nested | `form.test.js::'rule + check: types, required, bounds, lists and messages'`, `::'nested schema, lists, bool, cross-field validate and values()'` | ✅ | |
| 178 | Kit | Server errors: `{ errors: { email: 'Taken' } }` → field; any other failure → `f.error` | `form.test.js::'server errors: returned or thrown {errors} mark fields; other failures set f.error; reset clears'`, `runtime/test/rpc.test.js::'rpc: 422 carries the field errors for form() …'` | ✅ | |
| 179 | Data | `api/*.jsbrain`: `export default api('/users', { get: { out }, post: { in, out } })` | `api.test.js::'api: path params, query, JSON body, input validation, …'`, `crates/brain-check/tests/worker.rs::b1103_unknown_api_type` | ✅ | |
| 180 | Data | `types.jsbrain`: `export const User = { id: 'num!', … }` | `crates/brain-codegen/tests/worker.rs::api_type_names_become_imported_objects` | ✅ | |
| 181 | Data | `resource(api.get)` in `{#await}` must cover loading, error and empty | `crates/brain-codegen/tests/kit.rs::await_on_a_resource_reads_its_value`, `fixture:b1101-await-no-catch`, `http.rs::catch_without_error_states` | ✅ | A `{#each}` without `{:empty}` inside the `{:then}` → B1101 (🔎). |
| 182 | Data | Responses are validated | `api.test.js::'api: a response that breaks its schema is reported with the diff and rejects'`, `bench/http/run.mjs S04` | ✅ | |
| 183 | Caching | Immutable hashed assets | `server.test.js::'static: clean URLs, … immutable /_b/, …'`, `crates/brain-pack/src/cache.rs::headers_one_rule_per_path`, `bench/cache/run.mjs` (repeat visit: 0 B JS/CSS) | ✅ | |
| 184 | Caching | Service worker: visited pages work offline; `views/_offline.brain` optional | `runtime/test/sw.test.js` (9), `cache.rs::worker_manifest_and_opt_out`, `bench/cache/run.mjs` (offline 2/2, 1/1) | ✅ | |
| 185 | Caching | In-memory `resource()` cache | `api.test.js::'resource: memory stale-while-revalidate per URL …'` | ✅ | |
| 186 | Caching | Never write a service worker or cache headers yourself | — | ⚠️ | **Diverges**: a `public/sw.js` silently replaces the generated worker in `dist/sw.js` (no warning; `public/robots.txt` gets one, `crates/brain-pack/src/cache.rs:134`). |
| 187 | Caching | Opt out in `brain.json` `cache`: `sw`, `http`, `offline`, `data` | `bench/cache/run.mjs` (opt-outs, 4 PASS), `cache.rs::worker_manifest_and_opt_out`, `api.test.js::'QA #124 cache.data:false …'`, `sw.test.js::'QA #124: … offline:false …'` | ✅ | |
| 188 | Caching | `api('/posts', { get: { out, cache: '10m' } })` (never for personal data) | `api.test.js::"cache: '10m' keeps the GET in the Cache API …"`, `worker.rs::b1102_api_cache`, `::b1102_when_data_caching_is_off` | ✅ | |
| 189 | Caching | `store(init, { persist: true })` | `runtime/test/persist.test.js` (5), `worker.rs::store_persist_is_accepted` | ✅ | |
| 190 | Caching | `*.worker.jsbrain`: its exports become async functions | `runtime/test/worker.test.js` (2), `crates/brain-codegen/tests/worker.rs::importer_gets_async_stubs`, `::worker_chunk_serves_its_exports`, `crates/brain-pack/tests/workers.rs::worker_modules_are_chunks_with_hashed_urls`, `worker.rs::b0127_no_dom_in_workers` | ✅ | |
| 191 | Server | `.brain_env`: shared lines, then `[development]` `[staging]` `[production]` overrides | `crates/brain-check/tests/env.rs::sections_precedence_and_expansion`, `::b0724_malformed_lines_with_line_numbers` | ✅ | |
| 192 | Server | `brain run` uses development, `brain build` production, `--env x` another | — | 🔎 | `brain build --env staging` puts `staging.example.com` in `dist/`; a plain `brain build` does not. |
| 193 | Server | `process.env.KEY` (server) or `process.env.PUBLIC_KEY` (anywhere) | `crates/brain-codegen/tests/fold.rs::public_env_values_compile_to_constants`, `crates/brain-check/tests/server.rs::env_vars_b0702`, `env.rs::b0720_empty_in_build_env_and_b0723_public_secret` | ✅ | |
| 194 | Server | Never create a `.env` | — | ⚠️ | **Diverges**: no rule; a `.env` in the app root is ignored silently by `check` and `build`. |
| 195 | Server | `.jsbrain` starting with `<meta server>` runs only on the server, used only in `load` or server functions | `server.rs::server_imports_b0701`, `fixture:b0701-server-import-client`, `crates/brain-pack/src/server.rs::rewrites_imports_between_server_modules` | ✅ | |
| 196 | Server | `server: async function save(ctx, data)`; the client calls `await save(data)` | `server.rs::server_functions_b0704`, `fixture:b0704-server-not-async`, `crates/brain-pack/tests/ssr.rs::ssr_and_rpc_through_the_real_server`, `rpc.test.js` (6) | ✅ | |
| 197 | CLI | `brain start` (serves `dist/`, restarts on crash, logs to `logs/app.log`) | `crates/brain-cli/tests/start.rs` (8), `bench/start/results/summary.md` cases 1a, 1b, 2, 4a, 4b, 6 | ✅ | `bench/start` not re-run (about 4 min with a release build); its summary is from 2026-10-13. |
| 198 | CLI | `brain reload` (no downtime) | `start.rs::killed_child_is_back_within_a_second_and_reload_drops_nothing`, `::a_failed_reload_exits_1_and_the_old_instances_keep_serving`, `runtime/test/server-proc.test.js::'two children on the same socket: …'`, `bench/start` cases 3, 3b, 3r | ✅ | 0 failed requests in 287 k under reloads. |
| 199 | CLI | `brain status` · `brain stop` | `bench/start/run.mjs` (every supervisor stopped with `brain stop`, pids from `brain status --json`; "Leftover processes: none") | ✅ | Without a supervisor `brain status --json` answers an error with a `fix` (🔎). |
| 200 | CLI | Never add PM2 or another process manager | — | 🔎 | Author guidance; `brain start` covers the job (rows above). Nothing to enforce. |
| 201 | CLI | Sub-path: `brain.json` `"base": "/app/"`; app-absolute links get the base | `crates/brain-pack/tests/base.rs::every_url_lives_under_the_base`, `lib.rs::base_from_brain_json_and_link_prefixing`, `cache.rs::base_path_moves_every_url_under_it`, `crates/brain-cli/tests/base.rs::dev_serves_everything_under_the_base`, `start.rs::static_mode_serves_under_the_base`, `server-base.test.js` (3) | ✅ | |

## Planned blocks still in SPEC.md

`rg -n '<!--planned' SPEC.md` → 14 blocks. Task state from `equipo.sh task --team brainllm list` (2026-10-13).

| Block | Text (SPEC.md line) | Task #N | Behaviour today (quick check) | Recommendation |
|---|---|---|---|---|
| #65 | `_loading.brain`, `<meta cache="60">`, injectable `services/` + `mount(…, { provide })` (14) | done | `mount(C, el, props, { provide })` and `withProvide` pass (`ch.test.js`); `_loading.brain` exists only in the scaffold (no use in `brain-pack` or the runtime); `<meta cache>` is only accepted by the parser (`crates/brain-codegen/src/lib.rs:946`) | keep planned; the task is closed but two of the three claims do not work (reopen or split) |
| #151 | i18n: `brain.json` `i18n`, `t()`, `fmt`, `<LangSwitch>`, `brain add locale` (19) | pending | B14xx checks and runtime `t()`/`fmt` exist (`crates/brain-check/tests/i18n.rs`, `i18n.test.js`); `brain add locale es` → "unknown kind" | keep planned |
| #154 | HTML first: real `<a href>`/`<form>`, `<meta client>` (27) | in progress | `<meta client>` → B0730 info; posts without JS work (`form.test.js::'#156 …'`, `server-form.test.js`) | keep planned until #154 and QA #157 close |
| #87 | `brain add channel\|service\|api\|type\|guard\|server` (51) | pending | `service`, `guard`, `server` → "unknown kind"; `channel`, `api`, `type` → usage error | keep planned |
| #66 | `<Comp lazy>` / `lazy="idle"\|"interaction"`, `<Img>` (96) | pending | runtime `lazy.test.js` passes, but `<Counter lazy />` → B0206 and `<Img …>` → B0203 | keep planned |
| #169 | Group state `provide(Cart, store())`, `resource(…, { page: true })` + `more()`, `{ every }`, `optimistic` (113) | in progress | all four work: `bench/precision/run.mjs P5`, `http.test.js::'optimistic: …'`, `::'resource page: true → …'`, `crates/brain-check/tests/http.rs::store_inside_provide_and_file_endpoints` | **uncover** (the rest of #169 is not in this block) |
| #106 | Agent tools (WebMCP): `<form tool>`, `<meta tools>` (155) | pending | declarative forms compile and B17xx exist; imperative tools are still reported as planned (`seo_tools.rs::imperative_tools_are_planned_and_declarative_forms_compile`) | keep planned |
| #159 | `brain run --mock` (175) | pending | the flag is accepted and ignored: a normal dev server starts | keep planned |
| #176 | Automatic component isolation: fallback with Retry, toast on a failing handler (191) | pending (QA) | works: `guard.test.js` (8), `errors.test.js::'failing handler (§42): …'`, `bench/scopes/run.mjs 9` | **uncover**; QA #176 fault injection can follow on the served text |
| #90 | Runaway components stopped; B0907 at build (195) | pending | B0907 is not in `crates/brain-lex/src/codes.rs`; B0905/B0906 exist (`crates/brain-check/tests/runaway.rs`) | keep planned |
| #68 | `brain preview`, `brain explain`, `brain doctor --fix`, exit codes (223) | pending | `explain` works; `preview` → "unknown command"; `doctor --fix` runs a plain doctor | keep planned |
| #40 | `brain doctor` loops/leaks/dead code, `brain shot`, `undo`, `migrate`, budgets (228) | pending | `shot`, `undo`, `migrate` → "unknown command"; doctor leaks skipped (#24) | keep planned |
| #37 | `brain edit add-prop\|remove-prop\|rename\|move\|extract` (233) | pending | `brain edit` → "unknown command" (`brain split` exists) | keep planned |
| #82 | `brain gen`, layout primitives, `brain apply`, `read --outline`, `map --focus`, `context` (236) | pending | `gen`, `apply`, `read`, `context` → "unknown command"; `map --focus` prints the whole map | keep planned |

## Gaps by owner

### COMPILER (brain-lex · brain-parse · brain-check · brain-codegen · brain-css)

1. `--line` used as a background (row 97): no rule (`background: var(--line)` → `[]`).
2. Top-level folders outside the layout (row 149) and a `.env` file (row 194): no rule, ignored silently.
3. Markup in a `.jsbrain` (row 8) still gives a generic B0001.

### TOOLCHAIN (brain-cli · brain-serve · brain-mcp)

1. Colocated test only for `brain add comp` (row 20); `add view|ui|layout|lib` create none.
2. `brain doctor`: no captured-errors section (row 116), leaks pending #24 (row 127).
3. Tests that would turn 🔎 into ✅: `brain add view users/[id]|layout|lib|test` (14, 15, 18, 19), `brain fix` (132),
   `brain_errors` (115), `--json` on every command (123), `brain map` has no `channels` key (133).

### BUILD (brain-pack · brain-min)

1. No obfuscation layer (row 135; INTERNALS `--protect=std`).
2. Unused component locals survive in `dist/` (row 136); module exports are shaken.
3. A `public/sw.js` silently replaces the generated service worker (row 186): the shadow check in
   `crates/brain-pack/src/cache.rs:134` runs before `sw.js` is generated (`cache.rs:185`), so the copied public file
   is kept and wins in `dist/`.

### SEO (brain-seo)

1. `twitter="@…"` on the `app.brain` `<meta seo>` is ignored (row 165); only `brain.json` `social.x` writes
   `twitter:site`.

### RUNTIME (runtime/)

No ⚠️ rows. 🔎 rows that deserve a test: `{#await}` (75), text escaping (79), `bind:ref` (57), a `drv` throw in a node
test (112, covered only by `bench/scopes`).
