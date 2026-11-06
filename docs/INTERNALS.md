# BRAINLLM INTERNALS — contracts between compiler, toolchain and runtime (not for the LLM that writes apps)

Internal team contracts; same section numbers as SPEC v0. The spec the LLM reads is /SPEC.md. Changes: only the LEAD edits this file; the other roles
propose by message with the exact diff.

## 1. `.brain` file

Fixed block order, all optional except the one marked:

```html
<meta props="name:type=default ...">   <!-- types: str num bool list obj fn any; ? = optional -->
<script> BrainScript </script>
MARKUP (HTML + interpolation)
<style> CSS </style>
```

Props: `name:type=default` · optional `name:type?` · without `=` or `?` = required. `tokens.cssbrain` lives at the app root.

Component name = file name in PascalCase (`user-card.brain` → `UserCard`).
Level by folder: `ui/` micro (no `<script>` with state) · `comp/` component · `views/` view/route.

## 2. BrainScript (closed subset of JS)

Allowed: `let` `const` · arrow `() =>` and `function` · `if/else` · `for (x of y)` · `return` · `await` · `try/catch/finally` · `throw` ·
objects, arrays, spread, destructuring · template strings · JS operators · `import x from './a.brain'|'./a.jsbrain'|'./a.js'`. Exports: `export default expr` · `export const|function` (only in `.jsbrain`, and `load` in `views/`).
Forbidden (error with fix): `class`, `this`, `var`, `with`, `eval`, `new Function`, generators, getters/setters,
labels, `for in`, dynamic `import()` (B0002 names it in statement and expression position, 2026-11-02).

Primitives (the only globals):

| Primitive | Signature | Semantics |
|---|---|---|
| `sig` | `sig(v)` | reactive state; read and written like a normal variable (`n++`, `n = 5`) — the compiler rewrites |
| `drv` | `drv(() => expr)` | memoized derived value, read-only |
| `fx` | `fx(() => { ...; return cleanup? })` | effect; automatic tracking; cleanup on unmount |
| `load` | `export async function load(ctx)` | only in `views/`; runs on the server, its return value becomes props |
| `emit` | `emit('event', data)` | event to the parent (`on:event`) |
| `ref` | `ref()` | node reference (`bind:ref={x}`) |
| `log` | `log(...a)` | console.log that the prod build removes |
| `provide` / `use` / `key` | `provide(K, v)` · `use(K)` · `key('name')` | ancestor → descendants context (§12) |
| `store` / `bus` | `store(init, opts?)` · `bus(name)` | shared channels, only in `channels/*.jsbrain` (§12) |

"Only globals" refers to the framework's: standard JS built-ins and Web APIs (Math, JSON, Date,
Promise, fetch, Array/String/Object methods) are allowed. Touching the DOM with `document`/`window` is
warning B0201 (fix: `ref()` + `bind:ref`). In `views/`, the return value of `load()` arrives as `props`
(`props.users`). `emit('event')` without data is valid.

## 3. Markup

- Interpolation `{expr}` · attributes `attr={expr}` · `class:name={bool}` · `style:prop={v}`
- Events `on:click={…}`: a bare identifier is a function reference and receives the Event (or the `emit`
  data); any other expression is a statement. `on:submit` does an implicit `preventDefault()`.
- Two-way bind: `bind:value={sig}` · `bind:checked={sig}` · `bind:ref={x}`
- Flow: `{#if c}…{:else if d}…{:else}…{/if}` · `{#each list as it (it.id)}…{:empty}…{/each}` (key required)
- Async: `{#await p}…{:then v}…{:catch e}…{/await}`
- Components: `<UserCard user={u} on:select={pick} />` · default slot `<slot />`

## 4. AST (COMPILER → TOOLCHAIN contract)

Every node carries `span: { start, end, line, col }`; the file lives once in `Component.file` and in each Diag. Root:
`Component { file, name, level, meta: MetaAttr[], props: Prop[], script: Stmt[], template: Node[], style: CssSheet | null }`.
`Node = Element | Text | Interp | If | Each | Await | ComponentRef | Slot`.
The exact Rust shape lives in `crates/brain-parse/src/ast.rs` (owner COMPILER) and is mirrored here.

Public API (COMPILER → TOOLCHAIN contract), fixed 2026-10-13:

```rust
// crate brain-codegen
pub fn compile(src: &str, path: &std::path::Path) -> Result<Output, Vec<Diag>>;
pub struct Output { pub js: String, pub css: String, pub deps: Vec<std::path::PathBuf>,
                   pub warnings: Vec<Diag>, pub names: Names }
pub struct Names { pub classes: Vec<String>, pub vars: Vec<String>, pub dynamic: Vec<String> } // "*" = unknown/classList
pub fn compile_with(src: &str, path: &Path, opts: &Options) -> Result<Output, Vec<Diag>>;
// Options { dev, runtime, runtime_dev, runtime_wc, import_suffix, root } · names(&Component) · component_hash(file)
// brain_parse::file_kind → ast::FileKind { Component, App, Module } · Stmt::ExportDefault (only .jsbrain; in .brain B0120)
// brain_check::TOKEN_FILES · templates: line breaks collapse to a single space and are trimmed at the edges
// defined in brain-lex, re-exported as brain_parse::Diag (lex, parse and check emit the same type)
pub struct Diag { pub code: &'static str, pub file: String, pub line: u32, pub col: u32, pub msg: String, pub fix: String }
impl Diag { pub fn to_json(&self) -> String; pub fn list_to_json(d: &[Diag]) -> String; pub fn is_warning(&self) -> bool }
// brain-check: load_ctx(disk_path, &Component) -> (Ctx, deps): nearest tokens.cssbrain + props of the imported .brain files
```

No `serde`: the workspace accepts no serialization dependencies; JSON is written by hand with correct escaping.

## 5. Errors (mandatory format, output of `brain check --json`)

B0749 uses state-like field names, not enum types: ordinary category/filter enums do not establish row publication
state. Optional state fields still warn. This advisory is not a substitute for author-defined read conditions.

```json
{ "code": "B0102", "file": "comp/counter.brain", "line": 7, "col": 12,
  "msg": "`this` does not exist in BrainScript", "fix": "use the variable directly: `n` instead of `this.n`",
  "edits": [{ "line": 7, "col": 12, "endLine": 7, "endCol": 17, "text": "" }] }
```
`edits`: 1-based, exclusive end; empty when the fix requires judgment. `Diag::apply(src, &edits)` is used by `brain fix`.

Every Diag has `level`: `error` (blocks) · `warning` (`is_warning`, does not block) · `info` (`is_info`, deterministic
normalization applied by `compile_with` on its own when no real error remains, §21).

Code catalog (the single source is `crates/brain-lex/src/codes.rs`; a contract names the rule, never the number:
COMPILER assigns the next free code there in the same change that emits it, then this list follows):
- B00xx structure: B0001 lex · B0002 syntax · B0003 malformed markup/CSS · B0004 `<meta props>`.
- B01xx BrainScript: B0101 class · B0102 this · B0103 var · B0104 with · B0105 eval/new Function · B0106 generators ·
  B0107 get/set · B0108 labels · B0109 for-in · B0110 classic for/while/switch · B0111 undeclared name ·
  B0112 write to drv/prop/const · B0113 internal mutation of sig (push, obj.x=) · B0114 mutating an each item ·
  B0115 sig/drv outside the root · B0116 state in ui/ · B0117 load outside views or without async ·
  B0118 emit without a literal name · B0119 nonexistent import · B0120 `export default` in a .brain ·
  B0121 (info) import of primitives or the `brain/ui` kit, removed automatically · B0133 `f.v = …` / `f.err = …`
  on a `form()` (edit `f.v = x` → `f.reset(x)`; the runtime throws on it too) · B0130 `.v` written by the author on a sig, drv or ref
  (warning, edit removes it; `form()`'s `f.v.email` is exempt) · B0129 `location.assign|replace('/x')` or
  `location.href = '/x'` with an app path (warning, edit to `navigate('/x')`) · B0124 more than one export in a
  `.jsbrain` (exempt: `types.jsbrain`, which holds one `export const` per type, and `i18n`/`content` data) · B0127 DOM, UI state or a `.brain`
  import inside a `*.worker.*` module (§35; replaces B0201 there).
- B02xx markup: B0201 document/window (warning) · B0202 each without key · B0203 component not imported ·
  B0204 bind without sig · B0205 unknown bind:x · B0206 unknown prop · B0207 missing required prop ·
  B0208 on: without handler · B0209 repeated attribute · B0210 raw HTML without `sanitize` · B0211 `unload` listener ·
  B0212 (info) ternary in `class=` → `class:x`. · B0216 unknown `transition:` kind (edit) · B0217 `<component>`
  without `is` · B0218 slot props mixed with other content. · B0223 (info, edit) `onclick={f}` on a native element → `on:click` ·
  B0224 (warning) `onclick="…"`, an inline handler the CSP blocks.
- B03xx CSS: B0301 global selector · B0302 nonexistent token (`--space-0` and its synonyms normalize to `0`, info) · B0303 nonexistent mixin. · B0307 dead CSS that `@scope` cancels (html/body/:root not first, `:scope ~`/`+`, a tag the component never writes; warning) · B0309 style rule outside `@layer` in tokens.cssbrain (beats every layered rule; warning)
- B04xx accessibility and B05xx SEO: §8.
- B1010 component over 150 lines or 20 KB (warning; the fix is the exact `brain split` command or the repeated block, §43).
- B16xx source SEO and B17xx WebMCP: §32 (B1705, B1706 and B1707 warnings; B1705, a consequential tool without confirmation, reads names and text, so it is a heuristic; B1708 doctor advice; B1709 cross-file duplicate on one page, `crates/brain-check/src/webmcp.rs`, 2026-11-01). B1895–B1899 asset delivery, read back from `dist` after every build and added to its warnings (`crates/brain-cli/src/asset_check.rs`, 2026-11-01); `brain release-check` (`crates/brain-cli/src/release_check.rs`, 2026-11-01) composes the existing gates, each as its own process, and adds no rule of its own. B18xx built output, `brain seo audit` (§33; `brain_seo::CATALOG` holds
  each code's title, why and example): B1801 no title (error) · B1802 title wider than 600 px · B1803 no description ·
  B1804 description wider than 920 px / 70 chars on mobile · B1805 H1 count · B1806 skipped heading level ·
  B1807 img without alt · B1808 thin content (< 300 words) · B1809 canonical missing or relative · B1810 no lang ·
  B1811 incomplete Open Graph · B1812 no outgoing internal links or internal nofollow · B1813 orphan page ·
  B1814 duplicate title · B1815 duplicate description · B1816 focus keyword (title, H1, first paragraph, description, URL except on `/`, density)
  · B1817 keyword cannibalization · B1818 invalid JSON-LD · B1819 broken internal link (error, fails the build) ·
  B1820 broken anchor (error, fails the build) · B1821 heavy LCP image or image without dimensions · B1822 staging build ·
  B1823 invalid collection frontmatter (error; a missing description is a warning) · B1824 duplicate slug in a
  collection (error) · B1825 invalid redirect (source still a page, broken target, loop, chain, status not
  301/302/307/308; error, fails the build) · B1826 URL published by the previous build that disappeared without a
  redirect (warning, with the suggested rule) · B1827 unusable social image (relative, under 1200×630, not 1.91:1,
  over 1 MB) · B1828 canonical pointing to another locale (error, fails doctor; `<html lang>` that differs from the
  URL locale is a warning) · B1829 entry shown with the default locale's text because its translation is missing
  (warning) · B1830–B1834 AI-answer (GEO) hints, info, never change the score: B1830 answer first, B1831 question
  headings not grouped as `faq`, B1832 article freshness, B1833 author as Person, B1834 no external sources · B1840–B1844 `brain trace` findings (§48): failed request, server exception, CSP violation, cross-origin failure, socket failure ·
  B1835 video without poster or upload date (info: not indexed as a video) · B1836 no `keyword` and the title or
  description does not name the H1 topic (info; `brain seo audit` reads each route's `keyword` from its view, like the
  build, through `brain_seo::cli_with`) · B1837 public dynamic route (no guard, not `noindex`) without `paths()` or
  `sitemap()`: rendered per request but listed nowhere (warning from `brain build`, named with the fix). Collections (`content/<name>/*.md`, §33): server-only `entries` `entry` `tags` `tagged` `pages`
  `page` from `brain:content`, `content.perPage` (default 10), drafts only with `--env staging`.
- B1861–B1866 site focus (§33, `seo.focus`; editorial, never in the score's wording about positions): B1861 more than 3 terms · B1862 missing from the home page · B1863 no main page · B1864 several main pages · B1865 mentions without a link to the main page (info) · B1866 in nearly every title. B1867–B1871 `brain seo audit --url`: B1867 listed URL redirects · B1868 canonical differs from the sitemap · B1869 URL variant answers 200 without the clean canonical · B1870 declared against observed (CDN, cookie, immutable, encoding) · B1871 `noindex` blocked in robots.txt (also static). B1872 later listing page canonical to page 1 (error) · B1873 generic link text · B1874 visible date against `dateModified` · B1875 structured data not on the page · B1876 HTML near Googlebot's 2 MB limit · B1877 URL form · B1878 search view or thin listing marked indexable · B1879 `seo-urls.txt`: registered URL gone with no redirect (error), new page not registered (warning).
- B1850 framework messages left in English (warning, 2026-10-31): a component calling `form()` or `api()` with a schema, or using `<ErrorState>`, in an app whose default language is not English, while that language's file lacks `form.*` or `error_state.*` keys. It names the keys; no edit, because the text must be written in that language. Only the keys the file has reach the page dictionary, so nothing is added to bundles.
- B072x environment file `.brain_env` (§36): B0720 empty value in the build environment (error) · B0721 key used in
  code but missing (warning, edit adds `KEY=`, or `KEY?=` when the file reads it with a default or a condition) · B0722 key never used (info); both see `const { KEY = d, B: alias } = process.env`, aliases of the object (`const e = process.env`, `const { env } = process`) and destructured `ctx.env` (2026-10-31) · B0723 `PUBLIC_*` value shaped like a
  secret (error) · B0724 malformed line or section (error). Values never appear in any diagnostic. `KEY?=` (2026-10-31) declares an optional key: the winning line of an environment decides, an empty one is omitted from the program's environment and a `PUBLIC_*` one compiles to `undefined`. B0727 `.brain_env.example` differs from the union of `.brain_env` and the keys the code reads, compared by key, block and optional flag: doctor only reports it; `brain fix` and `brain doctor --fix` write it.
- B076x backend settings (2026-10-31, `crates/brain-cli/src/startup_config.rs`): the validators of `brain backend serve`, also run per environment by `brain check`/`brain doctor` as warnings on values present in `.brain_env` and the process environment. B0760 mail keys and sender (`sender@domain` or `Name <sender@domain>`) · B0761 object storage · B0762 `DATABASE_URL` (missing, malformed, unreachable, unverified TLS in production; the `BDB_*` tokens stay in the message) · B0763 pool sizes, process count, port · B0764 `PUBLIC_SITE_URL` unusable in mail links. Serve stops on the fatal ones before its first database connection; messages name the variable, never its value.
- B0771 anonymous API catalog (warning, `backend.rs` `anonymous_catalog_warning`, printed at `backend serve`, silent for an explicit `--env development`) · B0772 handler without a default export (warning in `check`, `structure.rs`; refused at boot) · B0773 secret-shaped literal in a route, job or hook handler (error; `read_source_limit` is the boot refusal)
- B0784 trusted proxy on public addresses (warning, 2026-10-31, `crates/brain-cli/src/platform_config.rs` `proxy_findings`, fatal for `brain backend serve --env production`; `trustPublicProxy` declares a CDN).
- B0790 backend secret not set (warning, 2026-10-31, `crates/brain-cli/src/route_executor.rs` `secret_problems`, `platform_config.rs` `captcha_secret_problem`, `webhook_config.rs` `secret_problem`): `brain check` reads every route contract twice, with a stand-in (B0735 owns a bad contract) and with the development environment, and names what serve would disable or refuse (2026-11-01): an absent secret sets `Route.disabled`, which answers 503 at the top of `routes_http::serve` so the stand-in secret can never verify anything, and `webhook_config::load` leaves out endpoints without their keys (`Platform.disabled_webhooks` answers 503); too short, or endpoints left without the encryption key, exits 78. Warning because a secret may live only on the host. Names only, never values. `backend::startup_contract_errors` adds to B0735 what only serve reads: `brain.json` analytics and missing or oversized handler and before-hook sources; it stays out of `inspect` so schema, docs and migrate keep working while a handler is being written.
- B0787 resource contract changed (error, 2026-10-31, `crates/brain-backend/src/contract_review.rs`, `db::migrate_approving`): `migrate` and `verify` refuse a recorded contract that differs from the code until `brain backend approve-contract` records it.
- B0786 own session with a native backend (warning, 2026-10-31, `crates/brain-cli/src/env.rs` `own_session`): `ctx.session.set` in a file that does not branch on the backend, in an app whose `.brain_env` sets `BRAIN_BACKEND_URL` without `BRAIN_BACKEND_SESSION=app`.
- B0785 backend environment not stated (error, 2026-10-31, `crates/brain-cli/src/backend.rs` `unstated_environment`): every `brain backend`, `brain jobs`, `brain media` and `brain analytics` operation that connects stops without `--env` on a remote database or NODE_ENV=production; NODE_ENV is checked before the database lookup, so B0762 cannot hide it. Outside production `/_brain/api` also needs admin for any proxied request.
- B0795 no `csp.connectSrc` in brain.json (warning, `crates/brain-cli/src/origin_check.rs`) · B0796 invalid CSP source entry (error; the entry stays out of the policy). `brain_pack::cache::app_csp` is the one place the connect list replaces `https: wss:`; the build's own sources (sockets, analytics tags, Google sign-in) stay as `extra`. The `_headers`, nginx, Caddy, `dist/server.mjs` manifest and the `brain run` report-only policy all come from it. `CSP0` in `runtime/server/server.mjs` only serves a manifest with no `csp`, which no current build writes.
- Explicit `csp.formAction` (2026-11-04) adds exact HTTPS document origins or local HTTP loopback origins to the existing own-origin form-action. `form_action_problem` rejects wildcard hosts/ports, paths, sockets, blobs and header injection; `read_form_action` shares bounded-list validation and B0796 with check/doctor. `cache::app_csp` adds only this directive, so static host configurations, SSR manifests and dev report-only policy stay aligned without widening connect/script/framing permissions.
- Explicit `csp.mediaSrc` / `csp.workerSrc` (2026-11-04) accept `blob:` and exact TLS origins, plus plain HTTP loopback origins. Each is independently optional: omission preserves its CSP fallback, while a declared list emits `'self'` plus validated sources (an empty or wholly invalid list remains own-origin only). They do not add blob permission to script-src. Validation, the schema and both check/doctor use B0796; no wildcard host or port, data URL, path or header injection is accepted.
- B0794 unique value any user can claim (warning, `backend.rs` `claimable_unique_warnings`): a `unique` field with no `access` on a resource whose `create` or `update` grants `user` or `public`; an explicit `access`, including `{ write: ['user'] }`, states who may and silences it.
- B0797 internal-looking field returned to every reader (warning, `backend.rs` `public_field_warnings`): whole-word name heuristic; `private`, `writeonly` or any `visible` (including `['public']`) states the exposure and silences it; `api: false` resources have no readers. Never an error: a name is only a hint.
- B0792 a job or route handler whose imports need the browser runtime (error, `brain-cli` `backend::handler_bundle_problems`, the same bundling the worker does at start)
- B0814 browser navigation to a URL built at runtime (warning, `brain-check` `security::client_redirect`): `location.href = x`, `location = x`, `location.assign(x)`, `location.replace(x)`, `window.open(x)`. Accepted: a literal, a value whose host cannot grow, a same-origin path or helper (the B0710 rules) and a value inside `if (new URL(x, location.origin).origin === location.origin)` or after the `!==` form that returns. External destinations (2026-11-03, a payment checkout on another origin): the same two guard forms against a literal host (`new URL(x).host` or `.hostname === 'checkout.example.com'`, `.origin === 'https://checkout.example.com'`) pass when `brain.json` → `"redirects"` (the B0710 list) names that host; an unlisted one keeps B0814 and the message names it. A value the app's own server answered also passes, but only where the checker sees it: a `const` (or `const { url }`) bound to `await f(…)` with `f` a `server:` function of the same file, `await r.json()` of a `const r = await fetch('/fixed/path')`, `(await f()).url`, inside the block of that binding (`origin_guarded`, `server_answers`). A `let`, a fetch of another origin or a built path, and any other call keep B0814.
- B0815 literal option of a native `list()` call that the resource does not declare (warning, `brain-cli` `list_options`): a `filter` key or operator, a `sort` field or an `include` path missing from `query.jsbrain`, which the backend answers 422 on every call (the rule is `brain-backend` `query::filter_problem`, `query::sort_problem` and `relations::include_path_declared`, the same `parse` applies), and a literal `limit` outside 1..100 or `offset` outside 0..10000, which the native client refuses with 400. The resource is the literal `backend.resource('name')` of the call or of a `const` bound to exactly one resource in the file. Skipped: a call with a spread, a computed key, an unknown resource; a computed operator still reports an undeclared field.
- B0816 native route handler reads a literal header absent from its sibling route contract (warning, `brain-check::route_headers` and conservative native inference, 2026-11-04). Context/header aliases and literal destructuring are tracked; dynamic keys, mutations and reference escapes remain unknown. Contract reads are bounded and dependency-tracked, including absent files. No header is forwarded automatically; authentication uses ctx.user.
- B0811 literal iframe or script outside `csp.frameSrc` / `csp.scriptSrc` (warning, `brain-check` `origins::check_embeds`); invalid entries are B0796.
- B0812 server route environment outside `ctx.env` (warning, `route_executor::env_warnings`): a global `process.env` in any `server/routes/**/*.jsbrain` (`brain_check::env::process_env_offsets`, so comments and strings never count), and a key a route reads that `.brain_env` declares only in `[production]`. Names only, never values.
- B0813 operation debits a balance the caller chooses (warning, `backend.rs` `debit_subject_warnings`): a `debit` step whose `subject` is a `$` reference other than `$user.*` (`$input.*`, `$<step>.*`). `debit` is the only credit step operations have (no credit, refund or transfer step exists), and the contract only accepts `$` references as subjects.
- B0810 answer built but not thrown (error, `brain-check` statement walk): a bare `notFound()`, `gone()`, `redirect()` or `deny()`; a local function of the same name is the author's own.
- B0808 update granted without read (warning, `backend.rs` `blind_update_warnings`): `public` and conditioned read grants count as reading. B0809 operation step cannot reach rows its role cannot read (warning, `operation_reach_warnings`): update, delete, action and transition steps with ids from `$input`, and upsert steps, without `owned: true`, for a role without an unconditional read. Every keyed step applies the resource read policy (`Engine::may_read`): a hidden row answers 404 like a missing one, and an upsert whose key matches one answers 403 and rolls the operation back.
- B0799 backend session controls do not reach an app-owned session (warning, 2026-11-01, `crates/brain-cli/src/env.rs` `doctor`): `BRAIN_BACKEND_SESSION=app` keeps the app's own session alive past a revoke or idle timeout of the backend one; the recipe is `docs/backend/sessions.md`.
- B0798 import refused (error, `brain backend import`, `brain_backend::import` §54): one file runs in one transaction, so a bad row leaves everything as it was and the report names each row.
- B0770 file bucket without declared types (error since 2026-10-31, `crates/brain-cli/src/backend.rs` `bucket_type_errors`; `brain backend serve` refuses to start, and `Bucket::accepts` answers false for an empty list so no library path stays open): `types: '*'` is the explicit opt-out; no auto-edit, because choosing types is a judgment call. The `bucket-types-required` manifest gives existing apps the one-line path that keeps their behaviour.
- B0941 `brain doctor` (warning): a non-English locale with the auth email keys unset
- B0940 `app.brain` without `<head>` (warning): the build and the server write titles and tags before `</head>`, so they would vanish
- B0749 unconditioned public read of a resource with a state field (warning; silenced by a `where` or `expose_all_rows: true`) · B0734 missing local MCP registration (doctor warning; configuration only) · B0735 invalid backend resource/job contract (error; schema, policy or hook reference) · B0736 placeholder site URL (build and doctor warning): a reserved origin (example.com/org/net, `*.example`, `*.test`, `*.invalid`, localhost, loopback) from `.brain_env` or brain.json is no site outside development, so no sitemap, robots `Sitemap:` line or absolute URL is published; the build names the `sitemap()` routes it skips. An explicit `--site` is used as given. · B0738 role grant admits nobody: an undeclared role, or a signup role no grant names (warning) · B0739 implicit legacy roles kept by an app without a roles file (warning)

Invalid-policy guidance (2026-11-05): contract validation supplies an actionable fix naming declared-role source
and conditional owner.<ref> alternatives drawn only from the resource's reference fields. The CLI contracts boundary
adds actual explicitly configured roles on policy errors; implicit legacy discovery reads contracts itself and
must never be called from this error path. Invalid role configuration never masks the original schema/policy error. Validation changes no grant and retains the existing fail-closed authorization semantics.

- B0740 origin denied by the native backend · B0741 cross-origin RPC or form refused · B0742 native call needs a same-origin POST · B0743 live socket origin refused · B0744 invalid origin declaration · B0745 socket origin outside the HTTP origins · B0746 client call to another origin (warning; info for https) · B0747 browser blocked a cross-origin call (dev console) · B0748 auth public URL outside the allowed origins. Table by code: `docs/backend/proxy-cors.md`. Details go to logs and development answers; production answers carry only the code.
- B0762 connection failures name the host, database and user tried, never the password, plus where the URL came from (`Boot.source`) and the cause from SQLSTATE and server text (`boot::refusal`, 2026-11-01): 28000 with `pg_hba` is a missing pg_hba.conf line, not a role to create; other 28000 is a missing role or a peer/ident refusal; 28P01 a refused password; a URL without a user uses `PGUSER`, then the system account, as `psql` does (sqlx alone would try `anonymous`). · B0765 queue with no worker reporting to this database in 45 s (read from `_brain_workers`, so another process counts; another database does not) · B0766 queue workers not checked (database unreachable or not migrated) · B0767 schedules with no scheduler · B0768 active jobs in an undeclared queue (all warnings from `brain doctor` and backend startup, liveness from `_brain_workers`, 45 s window) · B0769 job without a handler (warning from `brain check`). · B0788 member read on a `key()` channel · B0789 raw `pg` in a route handler (warnings from `brain check`, 2026-10-31).
- B0750 declared route hides something (warning): a `server/routes/*` path that overlaps a view, a literal `public/` file or a framework prefix (`/_r`, `/_b`, `/_server`, `/_health`, `/@brain`), or that starts with a parameter; the gateways proxy declared routes on the site origin, so the overlap would hide the other one; a route that starts with a parameter is not proxied.
- B0780 stray `}` in markup (warning): a `}` in text or right after a value in an attribute closes nothing; `<pre>`, `<code>`, `<samp>`, `<kbd>` and `<textarea>` may show one, unless it follows a value. `{{x}}` is the same mistake, with the exact edit. · B0781 `await` with no enclosing `async` function (error, the emitted JavaScript does not parse); the top level of a `.jsbrain` and of a test file may await. · B0782 `BRAIN_SECRET` missing when a request opens a session (runtime). · B0783 credential in a readable place (warning): `console.*`, `localStorage`/`sessionStorage.setItem`, and a stored password in what a `server:` function, `load` or a route handler returns.
- B0737 credential in the signed session cookie (warning): every credential-shaped key, name or member passed to `ctx.session.set`; the fix moves it to `ctx.session.setSecret`. · B0751 backend setting missing or invalid (warning in check and doctor, startup error): see §31.
- B0920–B0923 server LLM module (§51): B0920 `ctx.llm` or a `PUBLIC_` provider key in browser code or `.brain_env` · B0921 call not configured (runtime) · B0922 provider failure (runtime) · B0923 `ctx.llm` with no provider key declared (warning).
- B07xx server and secrets (§11.1): B0701 server module used outside load/server: · B0702 env without `PUBLIC_` on the client · B0703 literal shaped like a key · B0704 `server:` function badly declared (not async, nested, in .jsbrain) or reading component state · B0707 invalid brain.json `server` setting (error; unknown key or a value outside its range).

## 6. Runtime (CODEGEN → RUNTIME contract), module `runtime/brain.js`

Fixed URLs: in dev, `/@brain/brain.js`, `/@brain/brain-dev.js` and `/@brain/brain-wc.js` (a single prefix, a
single module instance). In build, each island is served at `/_b/<hash>.js`.

Exports: `sig(v, name?)`, `drv(fn)`, `fx(fn)`, `batch(fn)`, `untrack(fn)`, `tpl(html) → () => Node`,
`insert(parent, fn|value, anchor?)`, `attr(el, name, fn|value)`, `cls(el, name, fn|value)`, `on(el, type, handler)`,
`list(parent, fn, keyFn, render, anchor?, empty?)`, `when(parent, condFn, a, b?, anchor?)`,
`mount(Comp, target, props) → dispose`, `hydrate(root = document)`.

- `sig` and `drv` expose `.v`; codegen rewrites `n` → `n.v` and `n = x` → `n.v = x`. Writing to `drv.v` throws.
- Writes are batched per microtask; `batch` applies synchronously when it finishes; every `on` handler runs inside `batch`.
- Native non-bubbling handlers (2026-11-05) use element listeners owned by an fx cleanup. Dialog close/cancel,
  toggle/beforetoggle, invalid, pointer enter/leave and media events do not depend on document bubbling. A shared
  eventCall preserves this/error/rejection handling for both paths; direct currentTarget stays native and does not
  invoke ancestors. Native default cancellation remains subject to the browser's cancelable flag and CloseWatcher
  activation rules. References: [HTML events](https://html.spec.whatwg.org/multipage/indices.html#events-2),
  [Pointer Events](https://www.w3.org/TR/pointerevents3/).
- Owners run first: before a queued effect runs, its owner chain is brought up to date from the top. A `when`/`{#if}` re-evaluates its condition and disposes its block before an effect inside the block runs, so `{#if data.file}{data.file.name}{/if}` never reads `name` of a `file` that became `null`, whatever the subscription order (guard-first scheduling, as in Solid and Svelte).
- `anchor` = template comment (`<!>` in `tpl`); everything is inserted BEFORE the anchor. Without an anchor, the runtime creates one at the end of `parent`.
- `fn` of insert/a/b/empty/render may return Node, DocumentFragment, string, number, array or `null`/`false` (empty).
- `render(item, i)` receives SIGNALS (`item.v`, `i.v`): same key with a new object updates the row in place. `keyFn(item, i)` receives the raw value. Duplicate key = error.
- `attr`: `value`/`checked`/`selected` are assigned as properties; `style:prop` → `el.style.setProperty`; `null`/`false` remove; `true` sets the empty attribute.
- `on`: the handler receives the native Event; `e.currentTarget` = the handler's element; one listener per type on `document`. `focus blur mouseenter mouseleave load error scroll` are registered directly.
- Component = `function Comp($p) → Node`; `$p` carries getters if the parent passes reactive values. A component called inside `insert(() => …)` is wrapped in `untrack`.
- `Comp.h = '<hash>'` (the same as `data-b` and `@scope`, §7). Only in dev, codegen emits `sig(v, 'name')`.

In build, `brain-min` renames runtime exports to 1 character with its single `RUNTIME_MAP` table and rewrites
imports and uses in every chunk; an export without a short key breaks the build. Codegen always emits readable names.

### 6.1 Hydration markup (emitted by SSR, consumed by `hydrate`)
`<button data-b="k3x9">Clicks<!--b:0-->: 1<!--b:1--></button><script type="b/state">{"c":"/_b/k3x9.js","p":{"start":1}}</script>`
- Island = root element with `data-b` whose IMMEDIATE SIBLING is `<script type="b/state">`. A `data-b` without that script is only a CSS scope.
- JSON per island: `c` = chunk URL · `x` = export (default `default`) · `p` = initial props (includes the return value of `load`).
- `<!--b:N-->` goes AFTER the content of hole N, in the order of the `<!>` in `tpl`.
- Resume: `hydrate()` installs capture listeners (`click submit input change keydown focusin`) without walking the tree. On the first event inside an island: preventDefault + stopPropagation, `import(c)`, `mount` in place of the island and replays the event on the equivalent node (`focusin` → `.focus()`).
- v0 = re-render on the first event (0 JS on load). v1 = claim nodes via `<!--b:N-->`, with the same markup.

### 6.2 Dev and Web Components (separate modules; the core stays ≤4 KB gz)
- `brain-dev.js` → `hmr(hash, newModule)`: re-mounts in place the instances with `Comp.h === hash` and keeps the values of `sig(v, 'name')`. In prod the cost is an `if (name && reg)`.
- `brain-wc.js` → `define(tag, Comp, css)`: `attachShadow({mode:'open'})` or reuses the DSD `shadowRoot`; one `CSSStyleSheet` per tag via `adoptedStyleSheets`; `Comp.p` → `observedAttributes`; `Comp.form = true` → `formAssociated` + `attachInternals()` in `$p.$internals`; `emit` → `CustomEvent(name, {detail, bubbles:true, composed:true})`.

## 7. CSS

Native CSS + `@mixin name { }` / `@include name;` + tokens from `tokens.cssbrain` (a nonexistent `var(--x)`
is an error). Output: `@layer reset, tokens, ui, comp, view;` and each component wrapped in `@scope ([data-b="hash"])`.
**Do not simplify the output CSS (measured in Chrome 154):** inside `@scope`, a selector without `:scope`/`&` carries an
implicit descendant `:scope ` and never touches the component root. That is why `brain_css::scoped` emits each top-level
selector twice, one anchored to the root: `:where(:scope):is(.todo) li, .todo li` (`:where` adds no
specificity). `to ([data-b])` works and does not exclude the root.
**Slot content is styled by its author** (decided 2026-10-13, like Svelte, Vue and Shadow DOM): markup a component
passes into another's `<slot>` keeps the scope (`data-b`) of the component that wrote it, so the host's CSS never
reaches it and a host cannot break the look of what it receives. The host styles only its own wrapper around the
slot. Verified by `bench/scopes` rule 8 (COMPILER #194).

## 8. Components: encapsulation, HMR, accessibility, SEO and tests (fixed 2026-10-13)

**Default output: compiled light DOM, not Web Components.** Each component compiles to direct DOM
functions (§6). Encapsulation in 3 layers:
- Styles: `@scope ([data-b="hash"]) to ([data-b])`. The `to` cuts at the child, so a style does not enter
  another component. Global selectors are forbidden in `<style>` (B0301); global styles live only in the
  `reset` and `tokens` layers.
- Logic: each `.brain` is an ES module with its own scope; there is no implicit global state.
- DOM: a component only touches its own nodes (`ref`). `document.querySelector` outward gives B0201. Toward the
  parent only `emit` is used; toward children, props and `<slot>`.

**Native Web Component, opt-in:** `<meta export="wc" tag="brain-counter">` additionally emits a Custom Element with
Shadow DOM, Declarative Shadow DOM for SSR, `adoptedStyleSheets` shared across instances and
`ElementInternals` if it is a form control. Used for widgets embedded outside BRAINLLM.

**HMR with preserved state:** the dev server sends the new module; the runtime re-mounts the instances of that
`data-b` keeping `sig` values by name. A CSS-only change swaps the sheet without re-mounting. The HMR client emits on `window` the `brain:hmr` event `{kind:'css'|'js'|'reload', file, receivedAt, appliedAt}` (ms epoch = `performance.timeOrigin + performance.now()`); `appliedAt` is taken after the sheet loads (css), after re-mounting (js) or right before reloading (reload).

Dev watcher admission (2026-11-05): access notifications are discarded in the callback before the bounded
64-event mutation queue. Linux source reads may otherwise fill that queue while compilation runs and falsely
trigger lost-mutation recovery. Create/modify/remove/unknown events and watcher errors retain admission;
real mutation overflow still clears caches and reloads clients. The focus navigation regression also retains
an identity outside the mounted view, so unexpected full document loads cannot pass as client navigation.

**Accessibility, verified by `brain check`** (B04xx errors with fix):
- B0401 `<img>` without `alt`.
- B0402 `on:click` on a non-interactive element: fix with `<button>`.
- B0403 `<input>`/`<select>`/`<textarea>` without `<label>` or `aria-label`. planned:#234: a raw control with a
  `placeholder` or `name` is normalized to the kit's `<Input label=…>` (info with edit); with neither it stays an error.
- B0404 Skipped heading levels.
- B0405 More than one `<main>` per view (error). More than one `<h1>`: info, normalized by `compile_with` (each `<h1>`
  after the first becomes `<h2>`, §21; #133). planned:#234: when the layout already provides `<main>`, the view's
  `<main>` is normalized to `<section>` the same way (info with edit).
- B0406 `<button>` without accessible text.
- B0407 `tabindex` > 0.
- B0408 Contrast of color token pairs below 4.5:1, computed at build.
- B0409 Animation without `prefers-reduced-motion`.
- B0411 `<a href>` without an accessible name: no text, no `aria-label`/`aria-labelledby`/`title`, no `<img>` with a
  non-empty `alt` (an expression, component or slot child counts as text).
- B0412 (warning) Link text that is only a generic phrase (`GENERIC_LINK_TEXT`: "click here", "read more"…) and no
  `aria-label`; only fully static text is compared.
- B0413 (warning) `<img alt>` that is a file name (`.png`, `.jpg`, `.svg`…) or a lone generic word (`poor_alt`);
  `alt=""` stays the way to mark a decorative image.
- B0414 `<iframe>` without `title` or `aria-label`.
- B0415 (warning) `<video>` with `controls` or without `muted` and no `<track kind="captions|subtitles">`; a muted
  autoplay loop without `controls` is exempt.

**SEO per view:**
```html
<meta seo keyword="…" title="…" description="…" canonical="/path" og:image="/img.png" schema="Article">
```
The build emits `<title>`, the meta and Open Graph tags, JSON-LD, `sitemap.xml` and `robots.txt`, and prerenders to static
HTML. A view that does not declare `seo` is error B0501. An indexable view without `keyword` (the search query it targets,
audit-only, never rendered) is warning B0502; exempt: `robots="noindex"`, `_x.brain`, `[param]` views, 404/500/error views
and views under a folder with `_guard.jsbrain` (up to the app root, `Ctx.guarded`, the guard file is a dependency of
the check cache).

**Tests:** `comp/counter.test.brain` (BrainScript + `test()`/`expect()`/`click()`/`type()`) and `brain test`
runs them in real headless Chrome via CDP, with its own Rust client. The same command audits the
accessibility tree with `Accessibility.getFullAXTree`. No jsdom or Playwright.
Headless navigation isolation (2026-11-05): a destroyed pending __brainStep context becomes a failing TestResult
with the file, registered test name and component-test-specific repair. It is not retried or converted to success.
The runner closes the owned tab, reloads the same file in a new tab and checks its registered names match before
continuing later tests. Recovery navigation or a changed test list makes remaining tests explicitly unavailable;
other files continue. Interrupted runtime/heap diagnostics cannot be complete. Restored pages reinstall inspector
and error capture; completed/load-failed/recovery tabs close through their own attached target identity only.
The test API snapshots scenario registration and returns an opaque document/registration identity. Both runners
bind each step, accessibility observation and final feedback to that identity; trusted unload invalidates it before
a new document can answer. Interrupted observations are discarded. Navigation between steps fails the preceding
scenario and restores only the not-yet-started next one, once. A changed registry cannot execute a stale index.
Transport failures retain the existing one-retry policy and do not become ordinary assertion failures.

`brain-cli::testrun::preflight` runs before test-server reuse/start. It compiles current application sources with dev
codegen and follows explicit local Brain imports plus resolved automatic components, rather than broad watcher
inventories. Test scripts are parsed for imports and retain browser-provided globals. CSS imports use the CSS checker.
The bounded graph refuses sources outside the app and stops on current compiler errors before browser assertions.

Test API in `*.test.brain`: `test(name, async () => {…})` · `await mount(Comp, props, options?)` · `await click(sel)` ·
`await type(sel, text)` · synchronous readers `text(sel)`, `attr(sel, name)`, `hasClass(sel, cls)`, `count(sel)` ·
`expect(x).toBe(y)`, `.toEqual(y)`, `.toBeTruthy()`, `.toBeFalsy()`, `.toContain(y)` (substring, or an array element by `Object.is`; anything else fails), each negated with `expect(x).not.…` · `await waitFor(fn, { timeout })` (default 3000 ms) calls `fn`, then waits 20 ms plus a render flush and calls it again until it neither throws nor returns `false`; on timeout the last failure becomes the test's error, prefixed `waitFor: still failing after N ms` · `stub(target, answer)`, `reply(status, body)`: `stub` overrides `fetch` on the test page only (the runtime has no test code), so it covers `server:` functions (`/_r/<hash>/<name>`, by name, the answer function receives the call's arguments) and `api()` endpoints (`'GET /path'`, by method and path, with the query if the target has one), not an XHR upload. It lives for one test, a stub nobody called fails it, and the page turns the data cache off so answers do not leak between tests. Selectors are CSS within the mounted component; `#id` also finds that id's `~N` form, because a component's ids stay unique across the mounts of one file. `options`:
`slot` (text of the default `<slot />`), `slots` (`{ name: text }` for `<slot name="…" />`) and `provide` (service
swaps); slot text is real DOM, so the accessibility audit sees it. Unknown options and non-text slot values fail the test.

**Encapsulated IDs:** every static `id` of a component and its references (`for`, `aria-labelledby`,
`aria-describedby`, `aria-controls`, `popovertarget`) are rewritten with a per-instance suffix: the first instance keeps
the id as written (`new`), later ones get `~2`, `~3` (`new~2`), so `/page#id` anchors work (#268). Computed ids
(`id={'faq-' + it.id}`, `id="faq-{it.id}"`) get the same suffix at run time through the module helper `$sx`, and so do
computed references; with computed ids every token of a computed reference is treated as local (#308).
Direct prop IDs/references are caller-owned and bypass suffixing (2026-11-05); lexical each locals that shadow a
prop remain component-owned. Derived expressions remain local; callers supply unique public IDs.
Prop defaults selected by omitted/nullish input remain local and receive the suffix, including references to them.
Stateless inlined UI owns no IDs: its references do not inherit the parent's dynamic-ID scope or defaulted props;
slot content and subsequent parent nodes restore the parent's ID scope.
String-literal expression attributes share static-ID rewriting, including SVG fragment references. ARIA IDREF/IDREFS, HTML form,
table, microdata and popover/command references follow local IDs.
Local instance IDs do not collide in light DOM. B0410 pairs a computed reference with its
target by source shape and warns when the target sits in a conditional block the reference is not in.

## 9. Dev-mode memory inspector (`brain-inspect`, fixed 2026-10-13)

Exists only with `brain run`; not a single byte reaches `brain build`. Module `/@brain/brain-inspect.js` (owner RUNTIME),
served and wired by the dev server (owner TOOLCHAIN).

**What it measures, per sample every 1 s and on every event:**
- JS heap: `performance.measureUserAgentSpecificMemory()`. For that the dev server responds with
  `Cross-Origin-Opener-Policy: same-origin` + `Cross-Origin-Embedder-Policy: credentialless` (does not break cross-origin
  resources without CORP; `brain run --coep require-corp` is optional). Fallback: `performance.memory`.
- Live per component (`Comp.h`): instances, `sig`, `drv`, `fx`, delegated listeners and DOM nodes, counted by the
  runtime dev registry (`_dev`).
- Events with heap delta: module load (`/_b/*`, `/@brain/*`, via resource `PerformanceObserver`), mount,
  dispose, hmr and hydrate of an island.
- **Leaks:** each unmounted instance is registered in a `FinalizationRegistry` with `WeakRef`; if after 2 GC cycles
  it is still alive, it is a suspected leak B0601 with the component and the cleanup-less `fx` retaining it. A request in
  flight at the unmount defers the verdict until it settles (60 s at most) and is named in the fix. `brain test --heap` and
  `brain doctor` wait 3 s after unmounting, then abort what remains (dev inspector only) and collect. `brain test --heap`
  fails on B0601; doctor warns unless `doctor.failOnSuspectedLeaks`, and lists its failed gates in `failed` (2026-10-31). An `fx` that adds
  global listeners or timers without returning cleanup is B0602.

### 9.1 Standalone panel `brain devtools` (own port)

`brain run` serves the app on `:P` (default 5180) and the debugger on `:P+1` (default 5181; `--inspect-port N`, `--no-inspect`).
The debugger is a BRAINLLM app written in `.brain` (built with the framework itself) in `devtools/`, shaped
as a SaaS: sidebar, KPIs and dense views. It survives app reloads and supports multiple tabs (one session per tab).

| View | What it shows |
|---|---|
| Load log | chronological record of each component since navigation: order of appearance, cause (route, import, island activated by event, `{#if}` condition), parent, and per phase chunk download → evaluation → mount → first paint → hydration, with start relative to navigation, duration, gz/raw weight and dependencies it triggered. Shown as a waterfall and as a filterable list; click on a row = component detail |
| Tree | live mounted components (hierarchy, current props, sig with its value, hydrated or inert islands) |
| Load | per component: chunk download, parse, mount and first paint (ms), gz size, who imported it |
| Memory | total heap over time with event marks; per component: instances, sig/drv/fx, nodes, estimated bytes; leaks B0601/B0602 |
| Latency | per update: signal written → effects re-run → nodes touched, with duration; the worst 20 |
| Network | every `fetch`/`XMLHttpRequest`/`WebSocket` attributed to the component and to the `load`/handler/`fx` that originated it: URL, method, status, ms, bytes |
| Events | handlers executed per component, count and duration; emits between components |
| Graph | sig → drv → fx dependencies of a component; highlights what is recomputed unnecessarily |
| Timeline | flamechart-style timeline of mount, update, hmr, hydrate and network |
| State | all live `sig`/`drv` per component with current value, who last wrote them and how many times; received props |
| Storage | cookies (includes HttpOnly, read from the `Cookie` header the dev server receives; Secure/SameSite/expiry flags), localStorage, sessionStorage, IndexedDB (databases, stores, count and sample of records), Cache Storage and service workers |
| Connections | open WebSocket, EventSource, BroadcastChannel and Workers: URL, state, messages in/out, bytes and owner component |
| Timers | active `setTimeout`/`setInterval`/`requestAnimationFrame` with the component and the line that created them |
| Routes & env | `views/` routes with their `load`, `<meta seo>` and prerender state; public environment variables used by the build (never the private ones) |

**Debug one by one:**
- *Isolate:* `/@brain/isolate/<hash>?props=<snapshot>` mounts a single component with the props captured in the load log,
  without the rest of the app; retry, change props and measure it alone.
- *Step by step:* with "Pause on each load", the dev runtime holds each deferred chunk or island until the
  developer presses "Next"; the load log marks what changed in memory, DOM and network with each step.
- Each phase is recorded with `performance.mark`/`measure` (`brain:<hash>:<phase>`), so it also appears in Chrome's
  Performance panel.

**Only the project's own data:** each item is shown if the app's code touched it. The dev wrappers of `document.cookie`,
`Storage`, `indexedDB.open`, `caches`, `WebSocket`, `EventSource`, `BroadcastChannel` and timers record key,
component and line; what extensions, third-party scripts or the browser created goes into a separate "Foreign" filter,
hidden by default. Cookie and token values are shown masked until the developer reveals them.

**Attribution:** in dev, the runtime keeps the "current owner" (component + origin: load, handler, fx) on a stack; the
dev wrappers of `fetch`/XHR/WS and the timers capture that owner when called, so async calls stay
attributed even if they finish later.

**Discovery:** `brain run` writes `<app>/.brain/dev.json` with `{port, inspectPort, pid}` and deletes it on exit;
`brain_inspect` (MCP) reads it. Without a live dev server, the error carries the fix "run `brain run`". The scaffold ignores `.brain/`.

**Transport:** app → dev server WebSocket (`/@brain/ws`, `{t:"m", …}` messages batched every 100 ms) → server
→ debugger over WebSocket. The server keeps a 5-min circular buffer per session. The same data is available at
`GET /@brain/metrics?session=…` and through the MCP tool `brain_inspect` so an LLM can diagnose without a screen.

**Also inside the app:**
- Floating mini panel (Alt+Shift+B) with heap, leaks and a link to the full debugger.
- The same data goes over the HMR WebSocket to the server: `GET /@brain/metrics` (JSON) and the MCP tool
  `brain_inspect`, so the LLM can diagnose without looking at the screen.
- `brain test --heap`: takes `HeapProfiler.takeHeapSnapshot` via CDP before and after each test and fails if a
  test leaves live instances after dispose.

## 10. Own extensions: `.brain` · `.jsbrain` · `.cssbrain` (fixed 2026-10-13)

Own brand: the project source never uses `.html`, `.js` or `.css`. `dist/` ships web extensions (`.html`/`.js`/`.css`)
by default, so any host serves it unconfigured (decided by Chris 2026-10-13); the own extensions are opt-in with
`"ext": "brain"` in `brain.json` or `brain build --ext brain`. The browser decides by `Content-Type`, not by extension,
so in that mode it is enough for the server to map each one to its type.

| Extension | In the source | In `dist/` | Content-Type |
|---|---|---|---|
| `.brain` | component, view and `app.brain` (HTML + BrainScript + CSS) | prerendered page `<route>/index.brain` (`--ext brain`) | `text/html; charset=utf-8` |
| `.jsbrain` | shared logic module in BrainScript (`lib/date.jsbrain`), no markup | compiled and minified chunk `/_b/<hash>.jsbrain` | `text/javascript; charset=utf-8` |
| `.cssbrain` | shared styles: `tokens.cssbrain` (app root), mixins, reset | per-view sheet `/_b/<hash>.cssbrain` | `text/css; charset=utf-8` |

- **Zero `.html` in the project.** The entry is `app.brain`: it defines `<html lang>`, the common `<head>`, the layout and
  `<slot />` where each view (`views/*.brain`) goes. The compiler assembles the document; nobody writes `index.html`.
- Clean URLs for SEO: `/users` resolves to `dist/users/index.html` (`index.brain` with `--ext brain`).
- Precompression at build: each text file in `dist/` carries its `.gz` alongside (optimal deflate). `.br` is planned:#231
  (own encoder, enabled only if it beats that gzip in measured bytes); build-time Brotli is supplied by the maintained Rust codec (§26, updated 2026-10-29).
- Brand: `<meta name="generator" content="BRAINLLM <version>">` and header `X-Generated-By` on documents.
- `brain build --ext brain` needs a host where MIME can be configured: unconfigured hosts (GitHub Pages, S3 without
  metadata, Netlify/Cloudflare Pages without `_headers`) serve `application/octet-stream` and the app does not start.
- A plain `.js` `import` is still allowed as an escape hatch (§2) and is published as `.js` (`.jsbrain` with `--ext brain`).

**Who understands it:**
- `brain run` / `brain-serve`: internal MIME map.
- `brain build` emits `dist/brain.nginx.conf`, ready to `include` (the `types`/`index` lines only with `--ext brain`):
  `types { text/html brain; text/javascript jsbrain; text/css cssbrain; }` · `index index.brain;` ·
  `try_files $uri $uri/ =404;` · `gzip_static on;` (`brotli_static on;` enabled by the host when ngx_brotli is installed; .br files are emitted by the build) · `Cache-Control: public, max-age=31536000, immutable`
  on `/_b/` · security headers. Also `_headers` (Netlify/Cloudflare Pages) and a Caddy block.
- **Cloudflare:** its default cache decides by extension and these three are not on its list; the snippet documents the
  Cache Rule for `/_b/*` (or "Respect origin" with the `Cache-Control`). Without that rule they are not cached at the edge.
- Installing the snippet on Chris's droplet goes through `/ci-cd-manager`, never by hand.

## 11. Code protection in production (fixed 2026-10-13)

Physical limit: the browser has to execute the code, so the key to decrypt it travels with it. Encrypting the
client JS only obfuscates it, and it also forces `eval`/`new Function` (CSP with `unsafe-eval`), breaks V8's code cache,
preload and streaming compile. BRAINLLM protects in layers, with the cost of each one visible:

**Always in `brain build` (zero or negative cost):**
1. **What is valuable never ships:** `load()`, `.jsbrain` files marked `<meta server>` and `server:` handlers run only on
   the server; the client receives data, not logic. An import of server code from the client is B0701.
2. **Secrets:** an environment variable without the `PUBLIC_` prefix that reaches the client bundle breaks the build (B0702).
   Literals shaped like keys (sk-, AKIA, JWT, `-----BEGIN`) are scanned in the output (B0703).
3. **Total mangling:** identifiers, runtime exports (`RUNTIME_MAP`), internal prop names, `data-b` in
   base36; comments, `log()` and file names removed. No public source maps (`--sourcemap private` keeps them
   outside `dist/` for uploading to an error server).
4. **Integrity, not secrecy:** Subresource Integrity (`integrity="sha384-…"`) on every `.jsbrain`/`.cssbrain` and strict
   nonce-based CSP without `unsafe-eval`. Nobody can inject or alter the served code.

**Obfuscation by default in `brain build` (`--protect=std`, done by the compiler itself; `--protect=off` turns it off):**
5. **JS:** internal data prop names shortened, function order shuffled
   deterministically by build hash, and no readable component names (`Comp.h` in base36).
6. **CSS:** classes and custom properties renamed to 1-3 characters (`.user-row--done` → `.a3`, `--sp-2` → `--b`), with
   the same map applied to the markup and the JS; selectors rewritten, comments removed, rules merged.
7. **Markup:** internal attributes (`data-b`, `<!--b:N-->` anchors) in their minimal form; no source name
   survives in the HTML.
The cost of the std layer is low and the build prints it (bytes and startup ms against `--protect=off`).

**`brain build --protect=max` (cost measured and reported in the build):** additionally, encoded string table (the
literals move to an encoded array read by index; it breaks compression and adds decoding at startup, which is
why it is not in std until measurements say otherwise — Chris's decision 2026-10-13: measure and decide), control-flow flattening and
opaque predicates on `.jsbrain` files marked `<meta protect>`, never on the runtime. It is obfuscation,
not encryption: it stops casual reading, costs size and speed, and the build prints how much.

## 12. Isolation and communication channels (fixed 2026-10-13)

**Rule:** a component is a closed box. It reads or writes nothing of another component except through one of these 8
channels, all declared in code and visible in the debugger (Graph view). Any other coupling is an error.

| # | Channel | Direction | Declared by the child / emitter | Used by the parent / receiver | When to use it |
|---|---|---|---|---|---|
| 1 | **Props** | parent → child | `<meta props="user:obj">` | `<UserCard user={u} />` | data the child displays and does not change |
| 2 | **Model** (bind) | parent ⇄ child | `<meta props="value:str~">` (`~` = bindable; the child may write `value = x`) | `<Input bind:value={name} />` | editable control (input, toggle, selector) |
| 3 | **Events** | child → parent | `<meta emits="select:obj close">` + `emit('select', it)` | `on:select={pick}` | signal that something happened; the parent decides |
| 4 | **Slots** | parent → child (content) | `<slot />` · `<slot name="footer" />` | content between tags · `<div slot="footer">` | the child is a container/layout |
| 5 | **Exposed API** | parent → child (imperative) | `<meta expose="open close">` | `<Modal bind:api={m} />` and `m.open()` | one-off actions: focus, open, reset |
| 6 | **Context** | ancestor → descendants | `provide(Theme, sig('light'))` | `let theme = use(Theme)` | data that crosses many levels (theme, language, session) |
| 7 | **Store** | any ⇄ any | `channels/cart.jsbrain`: `export default store({ items: [], total: 0 })` | `import cart from '../channels/cart.jsbrain'` | state shared between unrelated branches |
| 8 | **Bus** | any → subscribers | `channels/toast.jsbrain`: `export default bus('toast:show')` | `toast.send(msg)` · `toast.on(fn)` | decoupled, stateless notification (toasts, analytics) |

Context keys: `Theme` is an object exported from `channels/*.jsbrain` with `key('theme')`, never a loose string
(no collisions). `store()` and `bus()` only exist in `channels/`. A `store` can sync outside the tab with
`store(init, { sync: 'tab' | 'storage' })` (BroadcastChannel or localStorage), always explicit.

**New primitives:** `provide`, `use`, `key` (§6, module `brain-ch.js` if the core has no headroom), `store`, `bus`.

**What the compiler prevents (B08xx):**
- B0801 mutable module-level state in a `.jsbrain` outside `store()` (hidden global state).
- B0802 `emit` of an event not declared in `<meta emits>`; B0803 `on:x` on an event the child does not declare.
- B0804 calling through `bind:api` a method not declared in `<meta expose>`.
- B0805 importing a `.brain` to read something other than the component (its internal variables do not exist outside).
- B0806 `store()`/`bus()` outside `channels/`; B0807 `use(X)` without any `provide(X)` in the ancestors (warning at
  build if static; error in dev if it does not appear at runtime).
- B0112 (already existing) covers writing a prop that is not a `~` model.

## 13. Performance through compilation (fixed 2026-10-13)

Principle: everything that can be decided at build is not decided in the browser. The source is for the AI; the output is
machine-JS code with no abstraction layers.

COMPILER:
- Signal folding: `sig` never written → constant; `drv` of constants → literal value; effects without
  reactive dependencies → a direct call on mount. No reactive node, no subscription.
- Precomputed DOM paths: codegen emits exact `firstChild`/`nextSibling` per hole (no `querySelector`).
  Release 0.5.0 constructs each access expression without repeatedly formatting its growing prefix; emitted
  expressions are unchanged. Sharing additional parent references is a separate optimization, not established here.
- Text: direct `node.data = v`; attributes with a minimal setter; equality comparison before touching the DOM.
- (Dropped 2026-10-13) Lazy handlers per event: almost every handler closes over component state, so it cannot leave the component chunk; islands (§6.1) and `lazy` components (§22) already defer code at a useful granularity.
- Zero runtime checks in prod: types, keys and contracts are validated at build; the prod runtime carries no asserts.
- Stateless `ui/` components are inlined into the parent (no function or instance).

TOOLCHAIN:
- Module graph → exact `<link rel="modulepreload">` per view and `103 Early Hints` in the server snippet.
  Header selection (2026-11-04) retains styles first within 2048 bytes, then module graph order; HTML and the manifest
  keep the full graph. Node SSR uses the same selection for final Link and optional 103 hints, logs omitted counts
  once per route, and gives declared font preloads the same priority as styles.
- Predictive prefetch of the probable next route by the router (links in view while idle, hover/focus first; §19).
- Streaming SSR: the `<head>` and the layout go out before `load()` finishes.
- Critical CSS per view inline; the rest deferred. Views without islands = 0 bytes of JS.

RUNTIME:
- Keep monomorphic shapes and lazy arrays; no closure per node when a shared function is enough.

**Only vanilla JS primitives (runtime and codegen output, hot path):** `for (let i = 0; i < n; i++)` instead of
`map/forEach/filter/reduce`; direct `node.data`/`textContent`, `cloneNode`, `insertBefore`, `removeChild`, `addEventListener`;
no `Proxy`, `Reflect`, `arguments`, `delete`, spread or new closures per iteration; `try/catch` outside
loops; fixed-shape objects created with all their fields. Zero dependencies, zero polyfills: the target is
evergreen browsers (Baseline).

## 14. ui/ kit and forms (approved by Chris 2026-10-13)
`brain/ui` comes embedded in the binary as `.brain`: Button, Input, Select, Checkbox, Dialog (native `<dialog>` +
focus trap), Table (sortable, virtualized >200 rows), Tabs, Toast (bus), Menu, Field (label + error + help).
Each one complies with §8 and covers loading/disabled/error/empty. `form(schema, {submit})` in the runtime (`brain-form.js`):
validates on blur and on submit, automatic `aria-invalid` + `aria-describedby`, blocks double submit (`busy`).
Schema mini-language: `str num bool email url date tel file` + `!` required + `<=N >=N` (bytes for `file`) + `file(image/*,.pdf)` accepted types + `[T]` list + type name (2026-10-31: `tel`, `file`).
2026-10-29: `f.v` and `f.err` are accessors whose setters throw (replacing them left the inputs bound to signals the
form no longer read, so submit saw empty fields); B0133 catches it at compile time. Validation messages go through
`msg(key, english, vars)` in `brain-schema.js`, which reads `globalThis.__brainDict` directly (no brain-i18n.js
import): `form.*` keys of the page's locale, then the default locale, then English. brain-pack ships the `form.*`
keys in the dictionary chunk of every page whose graph reaches `brain-schema.js`, even without a `t()` call.
A form posted without JS is validated in the page's locale too: `callForm({ locale })` sets `__brainLocale` only
around the synchronous validation (concurrent requests in other languages never see it), and the generic
"Something went wrong" message is `form.error` of that locale. `dist/server.mjs` loads the page's dictionary chunk
first; `brain run` strips the `/<locale>` prefix before routing the POST and hands the sidecar the dictionaries.

## 15. Data with contract
`api(path, methods)` and types in `.jsbrain`. The compiler generates the client (native fetch) and a minimal validator per
type; a response that does not comply → captured error with the shape diff. `resource(fn)` = reactive `{state, value, error}`;
B1101 if an `{#await}` on a resource does not cover `{:catch}` or the empty list. B1102 `cache` outside `get` or not
`<n>s|m|h` or the explicit `no` opt-out in an `api()` schema (edit when the fix is unambiguous, e.g. `10 minutes` → `10m`; §35). `brain run --mock` responds
with deterministic synthetic data derived from the schema.

2026-10-30: GET `cache: 'no'` matches the existing runtime contract and remains valid even with application-wide
data caching disabled. The checker never removes that endpoint privacy intent; writes still reject cache options.

## 15b. One SEO story with or without a server (2026-10-31)
`brain_seo::Built.server_head` writes the site's request-independent head into the templates of pages rendered per request,
through the same `social::head` and `inject_head` as prerendered pages. `head()` in server.mjs binds only request values:
title-derived `og:*`, the absolute `og:image` and the card type. The template also carries the WebSite + WebPage JSON-LD
graph with the route pattern as URL and without the page name, description, image or breadcrumb; `head()` writes the request
path into its `@id`/`url` like the canonical and fills the WebPage name from the final `<title>` and the description from
the view. Not covered yet: hreflang, the breadcrumb and image dimensions of a per-request page. A route with folder guards answers `x-robots-tag: noindex` even when it renders, and its template skips the site head: it is never indexed and the tags would only spend its view budget. `keyword` or `canonical` on such a view is warning B0505, because a guard cannot be read to learn whether it admits anonymous visitors. The optional credit line and the shared tokens sheet do not spend `budget.viewKB`, as when the sheet was a separate file; `htmlGz` in the report stays the exact emitted size.
`query` is read by the build (`brain-query` marker in the template, removed per request). Canonical stays path-only unless a
parameter is listed; `tests/seo_parity.rs` holds the parity and identity contract. A failed build is typed by
`build_error.rs`: `phase` and `diagnostics` with `brain check` fields, `generated` on locations outside the app, no edits there.

## 16. Visual verification and budgets (2026-10-28)
`brain shot <route>` owns a temporary dev server and Chrome session; both close after the operation.
Static preview/start reject symlinks in requested files, directories, gzip variants, headers and fallback pages.
The served `dist/` must remain trusted; these checks are not a filesystem sandbox against concurrent replacement.
Default viewports are 390×844 and 1280×800. PNGs and accessibility text go to `.brain/shots/`.
`--baseline <name>` compares against `baselines/<name>-<width>x<height>.png`; only `--update-baseline` changes
that tracked baseline. `--max-diff` is a changed-pixel fraction in [0,1], default exact. Different dimensions fail.
Animation/transition/caret rendering is disabled for capture, but dynamic data and external fonts can still vary.
Pin browser, OS, fonts and app data for stable comparisons. The accessibility outline includes roles and names;
it is not an AX-tree diff or a complete layout model. MCP `brain_shot` offers the same comparison for one viewport.
PNG decoding is bounded by bytes, dimensions and a deadline. Output/baseline paths reject traversal and symlinks.
Build `budget.viewKB` measures emitted JS + CSS + HTML per route (KiB=1024 bytes), including dynamic dependencies
conservatively, excluding media/fonts. It uses gzip sizes when emitted and raw bytes otherwise; a page rendered per
request (`_server/pages`) counts gzip-compressed, as the server compresses its response (2026-11-02). Every budget
message names the document it measured: the environment, its `.brain_env` values on this machine, and whether pages
carry absolute URLs (`brain doctor` builds with a placeholder site so SEO can score them, `brain build` without one,
which is why the two can differ by the length of those URLs). A route within max(2% of the limit, 256 bytes) passes
with a warning in the build and in doctor's `build.budget.warnings`: the HTML carries environment values, so the same
commit can fail on another machine (2026-11-02). Invalid or unknown
budget fields fail instead of silently disabling enforcement. `mountMs` p95 and `heapMB` remain planned and are
rejected explicitly until a real browser measurement contract is implemented.

**CDP disconnections (2026-11-04):** call/send writes and closed receive channels use one transport error formatter.
Broken-pipe/EOF/reset failures identify Chrome and suggest reducing concurrent browser work under load before retry.
No mutation is automatically replayed. Failed writes still stop only the owned browser session; watchdog deadlines
and profile cleanup are unchanged. Closed-pipe regressions cover both write APIs and receive EOF with owned helpers.

## 17. AST editing and migrations (2026-10-28)
`brain edit` applies checked source-span edits: add-prop, remove-prop, component-file rename, move and extract.
Rename/move update resolved imports while preserving local aliases. Extract delegates to the AST-backed card splitter.
Ambiguous dynamic references and relative CSS URLs in moved components are rejected rather than guessed.
The preflight snapshot is bounded to 2,048 files, 32 MiB total, 2 MiB per file and depth 32. Symlinks and overwrites
are rejected. The checker runs before/after; normal I/O errors roll back changed files. This is not a durable
transaction against power loss or SIGKILL. A kernel lock coordinates AST edits and MCP text writers; source
checks before each write and conditional rollback preserve detected external changes. This is optimistic detection,
not an atomic compare-and-swap with unrelated editors. MCP records one undo entry per changed file; undo that many steps to
revert a multi-file operation. CLI `brain undo` and version-to-version `brain migrate` remain planned.
Framework migration from React/Vue/Angular uses the separate staged workflow in `docs/MIGRATION.md`.

## 18. Protection: loops, leaks, overengineering and native error capture
**Loops (B09xx):** B0901 at build: an `fx` writes a signal it reads itself (direct cycle) or a sig→fx→sig cycle
detected in the static graph. Runtime: an effect re-run >100 times in the same flush is cut off,
captured as B0902 with the full cycle and the app stays alive (never hangs the tab). B0903 component
recursion without a base case (depth >64). B0904 list that grows without bound (inspector warning).
**Leaks:** §9 (B0601/B0602) + `brain test --heap`.
**Overengineering (B10xx, `brain doctor` and warnings in check):** B1001 `drv` that only returns another signal · B1002
`store` used by a single component (→ local `sig`) · B1003 context with a single direct consumer (→ prop) ·
B1004 `fx` that only writes another signal (→ `drv`) · B1005 unused props/emits/expose · B1006 component used
once and <10 lines (→ inline) · B1007 nearly identical components (normalized AST hash) · B1008 prop drilling >3
levels (→ context) · B1009 dead code (unused exports, functions, CSS).
**Native error capture (no third parties):** the runtime wraps `fx`, handlers, `load` and render; plus
`window.onerror` and `unhandledrejection`. Each error carries: fingerprint (type + component + line), component and
parent chain, stack with lines from the `.brain` source (dev: direct; prod: private source maps resolved by the
server), breadcrumbs (last 30 actions: navigation, clicks, fetch, signal writes without sensitive values),
build version and browser. Dedupe by fingerprint + send limit. The failing component shows its
`fallback` and the rest keeps running. Dev: "Errors" view of the debugger + MCP `brain_errors`. Prod: `brain build
--report=/path` sends via `navigator.sendBeacon` to the app's own endpoint; `brain errors serve` is a minimal
collector (SQLite) with the same UI as the debugger. Inputs and cookies are never included.

**Source-map marker ownership (0.4.1 development):** `finish` consumes framework-emitted JavaScript, never
arbitrary JavaScript or foreign ESM. The BrainScript parser discards author JavaScript comments; its AST has no
raw-code or comment escape hatch. The emitter's exact `/*@L<positive canonical u32>*/` comments are therefore
owned mapping directives. The framework's own lexer identifies comment trivia while keeping strings, regular
expressions and template text opaque; template substitutions are scanned normally, including nested function
bodies. Regex-leading expression statements are parenthesized by the emitter so inserted `if`/`for` blocks
do not change lexical context. `Lexer::next_span` uses the same scanners and validation without materializing
discarded token values or successful-token columns. Its empty span denotes EOF after trailing trivia; normal
`next_token`/`next_token_bytes` keep their complete payloads and saved lexer context remains interchangeable.
Errors retain exact generated coordinates; the generated line index is built only when a scan fails, and the
failing scan is repeated with it. Before HMR wiring becomes quoted metadata, the same scan strips compiler
comments from that code, preserving author literals; the existing 64 KiB wiring eligibility bound is unchanged.
This additional scan is limited to eligible development metadata, with no second generation or extra map/base64.
Marker-shaped quoted HTML/CSS, inlined child text and public environment constants remain data. Author
comment lookalikes cannot change source lines. Generation remains one pass; maps remain line-level, not precise
columns. If generated syntax cannot be lexed, compilation fails with the real lexer diagnostic at a virtual
`generated:<component>` location, explicit compiler-defect feedback and no author-source edits.

## 19. Native Chrome features leveraged (fixed 2026-10-13)
Rule: always feature detection; in browsers without the feature the classic path is used and nothing breaks.
The `.brain` author writes nothing different: the compiler or the runtime applies it.

| Feature | Support (verified 2026-10-13, sources in bench/support-19.md) | Where | What for |
|---|---|---|---|
| `Element.moveBefore()` | Chrome 133+, Firefox 144+; Safari no | RUNTIME `list` | move rows without losing focus, iframes, animations or state; fallback `insertBefore` |
| `scheduler.yield()` / `postTask` | Chrome 129+, Firefox 142+; Safari no | RUNTIME | split mounts and large lists into ≤16 ms slices (one frame; 5 ms slices added frames to a 1 000-row list); offscreen islands with `background` priority; fallback `MessageChannel` |
| `Element.setHTML()` (Sanitizer) | Firefox 148+, Chrome 146+ | RUNTIME `{@html}` | native sanitizing; fallback own minimal sanitizer |
| Navigation API | Baseline 2026: Chrome 102+, Firefox 147+, Safari 26.2+ | RUNTIME router | intercept navigations, scroll and focus restored; fallback History |
| View Transitions (same document) | Baseline 2025: Chrome 111+, Firefox 144+, Safari 18+ | RUNTIME | animated transition between views in the router |
| `@view-transition` (cross-document) | Chrome 126+, Safari 18.2+; Firefox no | COMPILER | transition between prerendered pages |
| Speculation Rules | Chrome 109+; Safari 26.2 behind flag (prefetch only); Firefox no | prefetch on document navigations only (§53, 2026-10-31) | the router intercepts other clicks, so a prerender was never used there (removed 2026-10-29); the router's predictive prefetch replaces it |
| Predictive prefetch (IntersectionObserver + `requestIdleCallback` + `fetch priority: 'low'`) | Baseline (idle callback: Safari no, `setTimeout` fallback) | RUNTIME router | links in view are fetched while idle (2 at a time, 12 per view), hover/focus/touch first; the next navigation swaps from memory and its chunks get `modulepreload`. Off: `router({ prefetch: false })`, `<meta name="brain-prefetch" content="off">`, `data-prefetch="false"`; skips Save-Data/2G, files, `/api/`, `/auth/`, log-out, unsubscribe and confirm paths and URLs with a token; any write (form submit, `api()` write, server function) forgets every fetched page; private/no-store and dynamic no-cache HTML is never reused; static HTML (`X-Brain-Static: 1`) and explicitly cacheable public HTML stay at most 5 min (bounded by max-age); requests carry `x-brain-prefetch: 1` (`ctx.prefetch`) (2026-10-29) |
| Built-in AI: Prompt API, Summarizer, Translator, LanguageDetector, Writer, Rewriter, Proofreader | Chrome 138+ (Summarizer, Translator, LanguageDetector stable; Prompt API origin trial on the web, stable in extensions; the rest trials); other browsers no | RUNTIME `brain-ai.js` (`ai` primitive) | on-device Gemini Nano; `ai.ready()` answers 'unavailable' where absent and calls throw `code: 'unavailable'` (`brain guide ai`, docs checked 2026-10-29) |
| `103 Early Hints` | Chrome 103+, Firefox 120+, Safari 17+ (HTTP/2+) | TOOLCHAIN | preload before the response |
| Explicit compile hints `//# allFunctionsCalledOnLoad` | Chrome 136+ | COMPILER | only in the initial chunk of each view; never in deferred chunks |
| Compression Dictionary Transport (`dcz`, RFC 9842) | Chrome 130+ | TOOLCHAIN | the previous build serves as dictionary: a returning user downloads only the delta; fallback gz/br |
| Popover API | Baseline 2025: Chrome 114+, Firefox 125+, Safari 17+ | ui kit | Dialog, Menu and Toast without JS |
| Invoker commands (`commandfor`) | Baseline 2025: Chrome 135+, Firefox 144+, Safari 26.2+ | ui kit | open/close without JS; minimal fallback |
| `<dialog>` | Broad Baseline: Chrome 37+, Firefox 98+, Safari 15.4+ | ui kit | native modal |
| CSS anchor positioning | Chrome 125+, Firefox 147+, Safari 26+ (complete in Safari 27); Limited | ui kit | menus and tooltips without measuring JS; feature detection |
| `appearance: base-select` | Chrome 135+, Safari 27+; Firefox behind flag | ui kit `Select` | native stylable select; feature detection |
| `content-visibility: auto` | Baseline 2025: Chrome 85+, Firefox 125+, Safari 18+ | COMPILER | offscreen content is not painted |
| `contain-intrinsic-size` | Broad Baseline | COMPILER | avoids layout shifts with content-visibility |
| `@starting-style` | Baseline 2024: Chrome 117+, Firefox 129+, Safari 17.5+ | COMPILER | entry animations without JS |
| Scroll-driven animations | Chrome 115+, Safari 26+; Firefox no | COMPILER | scroll animations without JS; feature detection |
| `fetchpriority` / `loading="lazy"` / `decoding="async"` | Baseline | COMPILER | the LCP candidate of each view is decided at build: first image with `width*height >= 50000` or an unknown size (§22) |
| Long Animation Frames API | Chrome 123+; Firefox and Safari no | RUNTIME inspector | "Jank" view: which component and script caused each long frame |
| Event Timing | Baseline 2025: Chrome 76+, Firefox 89+, Safari 26.2+ | RUNTIME inspector | INP per component |
| bfcache | all | COMPILER | `unload` forbidden (B0211); no `no-store` on public pages (session pages are `no-store`, §35) |
| Document Picture-in-Picture | Chrome 116+, Firefox 151+ (desktop); Safari no | devtools | mini panel in a floating window |

## 20. Native linter always on (fixed 2026-10-13)
A single engine (`brain-check`) runs in `brain check`, on every `brain run` recompilation, in `brain build` and in the
MCP. There is no separate linter to install. Levels: error (blocks build), warning (does not block), info.

New families in addition to B00–B11:
- **B12xx compatibility:** the binary embeds a Baseline table of web and CSS APIs (generated at binary build time
  from web-features data, dated version). B1201 API/property not supported in the target (`brain.json`
  → `"targets": "baseline-widely"` by default) · B1202 experimental or limited API without feature detection
  (`if ('moveBefore' in Element.prototype)`) · B1203 obsolete/deprecated API (with replacement in the fix).
- **B13xx bad practices:** B1301 `await` inside a `for-of` over independent data (→ `Promise.all`) ·
  B1302 `fetch` without error handling · B1303 nested loop over the same list (O(n²)) when a `Map` fits ·
  B1304 `fx` without reactive dependencies (→ mount code) · B1305 heavy computation inside `{#each}` (→ `drv`) ·
  B1306 timers without cleanup (in addition to B0602 at runtime) · B1307 magic time/size number outside tokens or
  constants · B1308 `console.*` except `error` and `warn` (→ `log`).
- Loops: B0901–B0904 (§18).

**Where it shows up:**
- `brain run` terminal: each save prints the file's diagnostics with `file:line:col` and the fix.
- Screen, in dev: overlay over the app (errors block, warnings as a chip in a corner that opens); the overlay
  shows the code snippet, the fix and a "copy for the LLM" button.
- Debugger: "Problems" view with the whole project, filterable by family and component.
- `brain build`: errors break the build with a summary; warnings are listed with a count.
- `brain check --json` / MCP `brain_check`: same output the AI reads.
`brain.json` → `"lint": { "B1307": "off" }` allows lowering or turning off specific rules; errors from §8, §11 and §12 cannot
be turned off.

### 11.1 Server code and RPC (fixed 2026-10-13, COMPILER proposal approved)
- Typed domain inputs (2026-11-05): domain::input_identifier separates input keys from resource column names.
  id, owner, created_at, updated_at and search are ordinary typed values for actions, transitions and operations;
  shape_fields and $input reference parsing share the same bounded admission. Resource identifiers, $user/$steps,
  row ownership and bound SQL remain separate. Unknown fields, invalid types and prototype names still fail.
- Native shutdown (2026-11-05): serve_full records one absolute ready-delay/drain deadline on the signal. It drops
  the owned HTTP future before cleanup and bounds socket, webhook worker and pool cleanup by that same deadline.
  db::close_until refuses to wait forever on checked-out connections. CLI-owned CommandRuntime allows at most one
  additional second for blocking-task teardown; this is not a library-wide claim that Rust can cancel arbitrary CPU
  work. Supervised job children retain their own separately bounded queue drain. PM2 backend/queue previews derive
  kill_timeout from their declared budgets plus a margin instead of reusing the frontend's twenty-second value.
- Migration boundaries (2026-11-05): db::migration_plan reads and validates the existing migration ledger without
  executing SQL migrations. approve-contract includes it before apply; migrate includes the pre-apply plan in its
  report. The phase is explicitly before-apply; CLI output separately reports observed framework applied,
  installedAfter and pendingAfter, preserving the legacy resource-only top-level applied meaning. The transactional
  migrator logs a fixed migration-rollback-boundary warning before advancing an existing
  framework ledger. Additive columns do not exempt an unknown migration from older-binary startup refusal. No
  published SQL/checksum or drift guard is changed.
- Handler native helpers (2026-11-05): backend::server_module admits the pure list/text/date/hash modules and the
  bounded outbound module, alongside the existing core/budget dependencies. Job and route graphs share this list,
  including indirect server imports. Browser-only imports still fail B0792. Runtime execution tests cover every
  documented pure helper; actual CLI/ledger acceptance keeps list/text modules out of routes that do not use them.
  The server emitter distinguishes existing server output from a browser module missing metadata. CLI B0792 keeps
  its message and actionable fix separate; an already server-only source is never told to add the same metadata.
- Server `.jsbrain`: first line `<meta server>` (or `<meta protect>`) → `Component.meta`.
- Server function in a `.brain`: `server: async function save(ctx, data) { … }` (reserved label; normal labels
  remain forbidden, B0108). Must be async (B0704). `ctx` is injected by the server (`ctx.req`, `ctx.cookies`,
  `ctx.user` if there is auth, `ctx.env`); the client calls `save(data)` without `ctx`. On the client a stub remains:
  `(...a) => rpc("<hash>/save", a)`.
- B0701: a `.jsbrain` with `<meta server>` can only be used inside `load()` or `server:` functions. Codegen removes
  those imports, `load` and the server bodies from the client.
- B0702: `process.env.X` / `import.meta.env.X` (including dynamic access) in client code with X without `PUBLIC_`.
  TOOLCHAIN substitutes `PUBLIC_*` at build.
- B0703: literals shaped like keys in client code; `brain_check::scan_secrets(&str)` also scans the final output
  in brain-pack.
- `Output` gains `server_js: String` (ESM with `load` + server functions + their imports) and `server_fns: Vec<String>`.
- **Callable names (2026-11-02):** only `server:` functions answer RPC and form posts. The server module exports
  `$server`, a frozen array of their names, and `serverFn(module, name)` in server.mjs is the one check: `rpc()`,
  `callForm` and the dev sidecar's RPC go through it. `load`, `paths`, `sitemap`, `seo`, `head`, `$forms` and any other
  export answer exactly as an unknown name (RPC 404 `Unknown function`, form 400) and never create a rate-limit key, so
  a view's `load()` data stays behind its guards and middleware. A module without `$server` exposes nothing.
- **RPC contract:** `POST /_r/<hash>/<name>`, JSON body = array of arguments, JSON response. Default defenses:
  same origin only (`Origin` + header `X-Brain-RPC: 1`, rejects CSRF), body limit (`server.maxBodyKB`), rate limit per IP
  and function (`server.rpcPerMinute`; counted only for a `server:` function, keyed by the exact `<hash>/<name>`, which is
  also the only accepted shape of a form's `_brain` field; the key map holds at most 50 000 windows), call deadline, unexpected error → generic 500 to the client and full log on the
  server. Public failures (2026-10-31, `publicError` in server.mjs): a plain `{ message }` without `status` → 400
  `{error: message}`, an Error subclass outside the built-ins with an integer 4xx `status` → that status and message
  (≤500 characters). `callRpc` and `callForm` share it, so `f.error` reads the same with and without JS. Plain
  `{ status, message }`, bare `Error` and built-in errors keep the message private. The runtime exports `rpc(id, args)` (RUNTIME); brain-serve mounts the endpoint in dev (TOOLCHAIN).
- **Server limits (2026-10-30):** brain.json `"server": { "maxBodyKB", "timeoutMs", "rpcPerMinute" }` (defaults 1024,
  10000, 60; read by `brain_pack::read_server_limits`). The build writes the resolved object into
  `_server/manifest.json` as `server`; server.mjs applies it after `createServer` options and `BRAIN_MAX_BODY_KB` /
  `BRAIN_TIMEOUT_MS` / `BRAIN_RPC_LIMIT`. `brain run` reads brain.json per request with the same overrides, sizes the
  request reader, the sidecar request frame (6× the body for JSON escaping) and the sidecar deadline from it, and
  passes the deadline and rate to `callRpc`/`callForm`. `maxBodyKB` is capped at 65536, the dev server's global
  in-flight body reservation. Both servers read bodies bounded (never buffering past the limit); a 413 names
  `brain.json server.maxBodyKB`, and server.mjs logs `body too large` with the limit and the fix. The dev server
  answers `Expect: 100-continue` with `100 Continue` once the declared length fits and with 413 otherwise; other
  expectations get 417. An invalid value is B0707 in `brain check`; the build warns and keeps the defaults.
- **Production server:** if the app has `load` or `server:` functions, `brain build` emits `dist/server.mjs`
  (Node, zero dependencies, `node:http`): SSR of the views with `load`, RPC at `/_r/`, and leaves the rest static. The
  nginx snippet does `proxy_pass` only for those routes and `/_r/`. Without `load` or `server:`, the output is 100 %
  static. It runs under `brain start` (§34), the framework's own process manager. Deployment tools also emit
  optional PM2/nginx configurations; an external process manager is not required.
- **Server file URLs (2026-10-30):** use `url::Url::from_file_path` for platform-native drive/verbatim/UNC paths,
  byte-safe escaping and the modification-time cache key. Preserve literal Unix backslashes instead of treating
  them as separators. Node ESM rejects those filenames; a rejection must never import a different nested file.

## 21. Goal: 100 % error-free (fixed 2026-10-13, requested by Chris)

Two metrics, both at 100 %:
- **Delivery:** code that comes out of `brain` never has errors. Already met with `check` + `fix` and locked by the QA
  gate (#59): nothing is delivered or published with an error-level diagnostic.
- **First pass:** the view the model writes compiles without errors on the first attempt. Measured on a bench of
  100 views × 3 models (Haiku, Sonnet, local qwen), with the spec frozen by sha.

How to reach the second: each bench failure is classified and receives a **structural** fix, never just one more line
of documentation. In order of preference:
1. **Make the error impossible:** the name the model predicts becomes the canonical one (like `--space-N`); what the
   model writes naturally becomes valid (like `bind:checked={obj.field}`); what is not implemented is not shown to it.
2. **Deterministic normalization:** when an error has a single possible correction with no loss of meaning, the
   compiler applies it itself and emits an `info`-level diagnostic (does not count as an error):
   `var` → `let` · `console.*` → `log` · missing key in `{#each}` over objects with `.id` → `(x.id)` · over
   primitives → by value · import of `brain/ui` or primitives → removed · exact token synonym
   (`--spacing-2`, `--space2`, `--gap-2` → `--space-2`; `--border` → `--line`; `--text` → `--fg`) · `class=` with
   ternary conditional → `class:x`.
   What requires judgment is never normalized: the text of an `alt`, the heading hierarchy, a missing `<label>`, a
   nonexistent import or dubious logic. Those remain errors with `fix`.
3. **The scaffold and `brain add` already bring the correct structure** (h1 and main in the view, `Field` with label), so the
   model completes instead of inventing.
4. Only if none of the above applies: one line in the served SPEC.

Each fix is validated with the same run (before/after, same bench sha) and recorded in bench/llm/README.md.

Codex source-generation transport distinguishes message-only non-terminal diagnostic items from tool calls and
terminal failures. Diagnostic text is discarded; success still requires a completed turn, valid token usage,
exit 0 and bounded owned output. Infrastructure failure stops queued Codex generations, retains incomplete
evidence and exits nonzero; it never establishes a first-pass result for unattempted briefs.
The gate and matched comparison share frozen-run validation: at least twenty complete unique generation rows,
summary/count agreement and valid matching source hashes. Codex baselines include provider/model/effort/mode;
legacy non-Codex keys are preserved, and a partial smoke cannot be accepted as a release baseline.
An uninitialized cohort fails the release gate until a complete reference is reviewed and explicitly accepted.

## 22. Modern router, loading and services features (fixed 2026-10-13)

RPC authorization boundary (documented 2026-11-04): `rpc()` prepares authoritative identity and calls the function directly; page guards/middleware are not inherited. Each function authorizes ctx.user and native resource policy independently.
Popup document policy (2026-11-04): `router()` captures whether the initial document head has the Google or Apple
sign-in opt-in (`content="1"`). After parsing a navigation response, before disposing or swapping the view, a change
in that opt-in triggers a full navigation in both the Navigation API and History paths. This also covers redirected,
dynamic and prefetched pages without enumerating static paths. Comments, inert templates and body markers do not
opt in. Pages with the same policy retain client navigation. `server.mjs` calculates opener and cache headers before
the HTML 304 branch; that branch preserves the stored CSP nonce by omitting a replacement CSP header.
Streamed SSR derives the policy from the head it will send, after applying SEO head metadata, for both GET and HEAD.

Coverage against React/Next, Vue/Nuxt, Angular, Svelte/Kit and Astro; all with a single pattern per need.

| Need | BRAINLLM | Status |
|---|---|---|
| Dynamic routes and nested layouts | `[id].brain`, `_layout.brain`, `404.brain` | #34 + TOOLCHAIN dev router |
| Protected routes | `_guard.jsbrain` per folder: runs on the server (SSR/RPC) and in the client router before navigating; `redirect(url)` or `deny(reason)` (renders `403.brain` with status 403) | #65 |
| Middleware | `_middleware.jsbrain` (headers, auth, logging) on the server | #65 |
| Route states | `_loading.brain` while `load` runs; `_error.brain` as the folder's error boundary | #65 · `_loading` #235 (rendered in SSR and prerender; not yet in streamed SSR, QA #253 → RUNTIME) |
| Request context | `ctx = { params, query, user, cookies, url }`; `ctx.user` is filled by a signed session (HttpOnly cookie, HMAC with `BRAIN_SECRET`) | #65 |
| SSG / SSR / revalidation | views without `load` → static; with `load` → SSR; `<meta cache="N">` → regenerate every N s (ISR-like) | #65 · `<meta cache>` #235 (applied, with stale-while-revalidate; verified in prod by QA #253) |
| Isolated reactivity | signals + 8 channels (§12) | done / #30 #31 |
| Lazy loading | per view (chunk), per island (event) and per component: `lazy` (visible, IntersectionObserver), `lazy="idle"` (requestIdleCallback/postTask), `lazy="interaction"` | #66 |
| Dependency injection | `services/*.jsbrain` + `provide`/`use` (context §12); `mount(..., {provide})` replaces services in tests | #65 + #31 |
| Forms and actions | `form()` + `server:` functions (RPC) | #38 #51 |
| Real time | `store(init, {sync:'ws', url})` over WebSocket with reconnection | #31 |
| i18n | §37: `i18n/<locale>.json`, `t(key, vars)`, prefix routing for non-default locales, `brain add locale` | #149–#153 |
| Images | `<Img>`: `width/height` required (no CLS). 2026-11-01: `fetchpriority="high"` goes to the first image of a view with `width*height >= 50000` or an unknown size; smaller images before it stay eager (up to 3), the rest get `lazy`. `{#if}` branches and `{#await}` states are alternatives with one budget each. In `{#each}` the decision is per iteration through the index: priority for the first, the first 3 eager; a nested list and any list after the candidate are lazy. A component cannot see its page: its first image with large literal dimensions is not lazy and takes no priority; `loading="lazy"` on it restores lazy for a card repeated in a list. Markdown images (`markdown::optimize_images`) follow the same rule and take `width`/`height` from `public/`; a remote or unreadable image gets no invented size. Responsive variants only if a system tool is available | #66 |
| Head per view | `<meta seo>` | done |
| Transitions | View Transitions (§19) | #43 |
| Offline/PWA | on by default, opt-out in `brain.json` (§35) | #121–#124 |

2026-10-29 fixes from a real app:
- Client guards get `ctx.user` as a getter with no value (the session cookie is HttpOnly). A guard that read it and
  would cancel, redirect or deny returns `null` from `check()`: a full load, where server.mjs runs the same guard with
  the session. Guards that never read `user` keep deciding on the client.
- `notFound()` (codegen intrinsic → `{ notFound: true, status: 404 }`, server code only) thrown or returned from `load`:
  server.mjs answers 404 with the app's 404 page; the dev sidecar answers `OK {"notFound":true…}` and brain-serve
  renders `views/404.brain` with 404. A streamed route holds its head up to 200 ms (`createServer({ streamHoldMs })`)
  for `load()`: a redirect, `notFound()` (also from an imported helper), deliberate 4xx or failure in that time gets
  the same 303/404/4xx/500 as a non-streamed route (2026-10-30, real app: they answered 200). A slower `load()` has
  its 200 head out already: a redirect becomes `location.replace`, not found a noindex notice.
- server.mjs `head(page, seo, path)`: an SSR template's canonical/og:url carry the route pattern (`/posts/:slug`);
  they are rewritten to the request path, og:url follows a `seo(props).canonical` unless the view sets `og:url`, and a
  tag in the view's `<head>` block replaces the generated tag with the same `name`/`property` (or canonical).
- `<option value={x}>` is an attribute in the runtime (`attr`), so SSR emits it, including `value=""`; SSR also
  reflects `selected` on options and marks the option matching `<select value={x}>`.

## 23. CLI and distribution with professional standards (fixed 2026-10-13)
Option value handling (2026-11-04): known valued flags, including global `--progress`, reject missing values before
command dispatch. Shared positional selection consumes spaced progress values, ignores the `-q` alias and respects
`--` termination. Explicit empty/invalid values still belong to each command's validation.
Check target update (2026-11-04): all positional targets are checked. Explicit files must end in `.brain`, `.jsbrain`
or `.cssbrain`; other extensions produce actionable usage errors (exit 2), never component-parser diagnostics.
No targets still means the current app; JSON combines diagnostics from every selected target.
Upgrade update (2026-10-31): `brain upgrade` returns a migration report from the app's recorded `brainVersion`
and applies the safe codemods with `--apply` (see Support commands below).
Identity update (2026-10-28): `brand::Session` adds a circuit-brain ASCII mark and task-specific start/completion
messages to interactive `check`, `build` and `run`. Its duration is measured with `Instant`; no comparative speed
claim is inferred. Both output streams must be terminals. JSON, profile, quiet/plain/off progress, CI, MCP and dumb
terminals suppress branding even when rich progress is requested. `NO_COLOR` presence disables banner color.
Branding writes only stderr, starts no background work, and leaves machine schemas unchanged. See
[CLI-EXPERIENCE.md](CLI-EXPERIENCE.md).

Browser helper reliability (2026-11-04): `bench/lib/cdp.mjs` probes the read-only debugging endpoint before one target
creation, attaches domains before navigation, and bounds connect/protocol waits plus pending commands. Closing a
connection rejects its pending requests. The SSR DOM's Event extends the native Node Event brand, preserving native
EventTarget/WebSocket interoperability even after the shim installs globals; custom-node dispatch defines targets
and keeps its own composed path without replacing native EventTarget paths.

Public route precedence (2026-11-04): Node already checks public build output before unguarded parameterized SSR matching.
A matching route with guards or middleware keeps its security chain ahead of every static/form/twin shortcut, even
when legacy output collides. Private-output checks and reserved status handling still run; POST uses its transport.
Dev checks explicit public files before extensionless app-page fallback, including well-known verification files,
while keeping a matching view's guard/middleware chain authoritative. Rust
static production already resolves public output before its fallback. Build sweeping removes old non-produced HTML,
so a route changed from static to guarded SSR does not retain a stale public page.

Local verification update (2026-10-27): `bash scripts/verify.sh` builds the CLI with the lockfile, then runs the
workspace, runtime, fixtures and English checks. Building first is required because fixtures execute
`target/debug/brain`; `cargo test` alone can leave that binary stale. The branch/PR verification workflow uses the
same entry point. Frozen LLM generation runs are checked separately with `bench/llm/gate.mjs <run.json>`.

Patterns from Vite, Next, Biome, Turborepo, Cargo and Deno applied to `brain`:

- **Version pinned per project:** `brain new` adds `@christianirack/brainllm` as a production dependency with an exact version and
  `run/build/doctor` scripts. A global `brain` **delegates** to the project's local version if it exists (Turborepo/ESLint
  pattern): two projects with different versions coexist and builds are reproducible.
- **A single config file:** `brain.json` with `"$schema"` (JSON Schema included in the package → autocomplete and
  validation in the editor). Precedence: flag > `BRAIN_*` variable > `brain.json` > defaults.
- **Stable output:** `--json` with `"schemaVersion"` in all machine output; fixed exit codes (0 ok · 1 project
  errors · 2 incorrect usage · 3 internal failure); respects `NO_COLOR`, detects CI (no color, no prompts, no
  version notifier); `--quiet` and `--verbose`.
- **Rich terminal output** (Chris 2026-10-13: "it must look nice while compiling"): on an interactive terminal,
  `brain build` and `brain run` show a colored, animated progress view; everything else gets plain lines.
  - `brain build`: a header (`◆ brain` + version + target), one live progress bar with the current phase (check →
    compile → pack → minify → prerender → SEO → cache/SW → compress) and `files done/total`, a check mark per phase with
    its time, then a summary: a colored bar chart of JS/CSS/HTML gz bytes per view (budget line in red when over),
    total size, page count, SEO score range, cache result (`N files cached, M changed`), build signature and total time.
  - `brain run`: the same header, a short boot bar, then a status panel (URL, LAN URL, view count, last rebuild in ms
    with a colored sparkline of the last 20 rebuild times, error/warning counters); each save prints one colored line
    (`✓ views/home.brain 0.6 ms · css` or `✗ B0302 …` with the fix).
  - Palette: the framework's accent for bars, green ok, amber warning, red error, dim gray for paths; Unicode block and
    braille characters with an ASCII fallback when the locale is not UTF-8. Redraws at most every 50 ms and never
    adds work to the build itself (rendering on its own thread; measured build time must not change by more than 2 %).
  - Plain mode is automatic and exact: not a TTY, `CI`, `NO_COLOR`, `TERM=dumb`, `--json`, `--quiet`, or running under
    MCP. Plain mode prints one line per phase and the same summary as text, so agents and logs keep stable output.
    `--progress=off|plain|rich` forces a mode.
- **Support commands:** `brain preview` (serves `dist/` exactly as in production) · `brain explain B0302` (what it is,
  why, correct example) · `brain info` (OS, brain/runtime/spec versions, Node, Chrome; for reporting bugs) ·
  `brain upgrade` (migration report and safe codemods, below) · `brain completions zsh|bash|fish` ·
  `brain new --template blank|landing|saas` · `brain build --analyze` (weight per view and component) ·
  `brain doctor --fix` (applies the mechanical corrections and checks again) · `brain run --host --open --https`.
- **`brain upgrade` (2026-10-31):** `migrate.rs` reads the embedded change manifests (`releases.rs`) in `(from, to]`.
  `from` = `--from`, else brain.json `brainVersion` (the scaffold writes it), else the package.json pin, else
  package-lock.json; `to` = `--to`, else the installed compiler (a prerelease build targets the newest manifest above
  it). Downgrades are refused. Codemods (ids in `releases::CODEMODS`): `package-pin` (the revision-checked
  `upgrade::preview` operations), `record-version` (`with_brain_version` rewrites only the value, or inserts the key
  first with the file's indentation and newline) and `brain-fix` (`run_fix_only` limited to the manifests' codes).
  Entries become `manual` (breaking/deprecation/change with steps and `finds` or `codes` matches, each with
  `file:line`), `adopt` (capabilities whose `uses` probes match nothing), `alreadyUsing` or `changes`. Gates: `check`
  always; `doctor` only with `--apply`, no check errors and without `--no-doctor`. Dry run writes nothing; `--json`
  returns the report (`schemaVersion` 1), otherwise a markdown checklist. Exit 0 = `ok` (gates pass, no manual
  steps), 1 = not ok, 2 = usage or input error. `brain upgrade <exact-version>` stays the package-pin preview. MCP
  `brain_upgrade` takes `{version}` for that preview or `{from?, to?, apply?, doctor?}` for the report; applied writes
  go to the MCP journal, so `brain_undo` reverts them.
- **Reproducible builds:** same source + same version = same bytes and same hashes (no dates or absolute paths
  in the output); QA verifies it by compiling twice.
- **Versioning:** lockstep semver (binary, runtime and spec with the same version); `CHANGELOG.md` generated from
  semantic commits; `brain --version` prints version + sha of the served spec.
- **Supply chain:** publishing with `npm publish --provenance` from GitHub Actions, SHA-256 checksums for each
  binary, zero runtime dependencies, no telemetry. New-version notice: at most one query per day,
  in the background, never in CI.

## 24. `brain mcp`: full project control over MCP (fixed 2026-10-13)
Update (2026-10-29): new apps register stdio in `.mcp.json` (Claude Code) and `.cursor/mcp.json` (Cursor), preserving
existing servers. CLAUDE.md imports AGENTS.md. `brain agent setup --hooks` installs optional bounded PostToolUse feedback;
B0734 warns about missing local registration without implying connection status. Profiles: full (default), six-tool
minimal, or focused backend. See [BACKEND.md](BACKEND.md) for supported tools and lifecycle limits.

The MCP server is the AI's main interface: everything the CLI does and more, with typed JSON input and output
(`inputSchema` and `outputSchema` on every tool). A single process per project; `brain run` shares it.

| Group | Tools |
|---|---|
| Project | `brain_info` · `brain_spec {topic?}` · `brain_guide {topic?}` · `brain_map` · `brain_new {name, template}` |
| Modules and files | `brain_add {kind: view\|comp\|ui\|channel\|service\|api\|test, name, props?}` · `brain_edit {op: add-prop\|remove-prop\|rename\|move\|extract, …}` · `brain_remove {path, confirm}` · `brain_undo {steps}` · `brain_read {path}` (with summarized AST) |
| Quality | `brain_check {path?}` · `brain_fix` · `brain_doctor {fix?}` · `brain_test {path?, heap?}` · `brain_shot {route, viewport?, baseline?}` · `brain_explain {code}` |
| APIs and data | `brain_api_add {path, methods, types}` · `brain_api_call {path, method, body?}` (against the dev server, with the test session) · `brain_load {route}` (runs `load` and returns the props) · `brain_rpc {component, fn, args}` · `brain_mock {on}` |
| Execution and debug | `brain_run_start` · `brain_run_status` · `brain_run_stop` (the dev server as a managed process) · `brain_trace {since?, failed?, module?, requestId?, issues?, limit?}` (§49) · `brain_logs {since?, level?}` · `brain_errors {since?}` · `brain_inspect {view: memory\|log\|latency\|network\|state\|storage\|connections\|timers}` · `brain_eval {expr, component?}` (dev only, in the live page, without access to HttpOnly cookies) · `brain_navigate {route}` · `brain_click\|type {selector}` (via CDP, to reproduce a bug) |
| Build and delivery | `brain_build {analyze?}` · `brain_preview_start\|stop` · `brain_pack` (dry-run of the npm package). **There is no `brain_publish`:** publishing is Chris's action. |

**Generated app docs (2026-10-31).** `brain docs` (`src/docs.rs`) builds one JSON model from `map::build`,
`brain_check::contract::card`, `backend::inspect` (rendered generically: new contract keys appear without code changes)
and the env key names of `.brain_env.example`, then renders seven sorted Markdown pages with a "generated" banner and
merges `docs/app/notes/<page>.md`. No timestamps or absolute paths (invariant 8). `cmd_build` rewrites changed pages
when `docs::enabled`; `run_doctor` adds a `docs` warning advice for stale pages and never fails on it. Golden output:
`crates/brain-cli/tests/fixtures/docs-app/docs/app` (`BRAIN_UPDATE_GOLDEN=1 cargo test -p brain-cli --test docs`).

**Guides (2026-10-29).** `brain guide [topic] [--json]` and `brain_guide {topic?}` serve task playbooks embedded from
`crates/brain-cli/guides/*.md` (`src/guide.rs`: start, views, data, forms, server, seo, a11y, webmcp, tests, i18n,
performance). SPEC says what exists; a guide says what to do, in order, how to verify it and which code catches each
mistake. Unit tests fail when a guide cites a diagnostic code `brain explain` does not know, a `brain <command>` that
`brain --help` does not list, or text the English check rejects; planned features are named as planned in prose.

- **MCP resources** (read without calling tools): `brain://spec`, `brain://map`, `brain://errors`, `brain://doctor`.
- **MCP prompts:** `new-view`, `fix-all`, `debug-error {fingerprint}`, `add-api`: guided flows that chain tools.
- **Notifications:** new errors, HMR results and diagnostic changes are emitted as MCP notifications while
  `brain run` is alive, so the agent reacts without polling.
- **Security:** destructive tools (`remove`, `undo`, overwrite) require `confirm: true`; `brain_eval` and
  CDP automation only exist in dev and on `127.0.0.1`; no tool reads `.env` or returns secrets.

## 25. Agent autonomy contract (fixed 2026-10-13, Chris: "a solid LLM experience that does not depend on humans")
Browser readiness correction (2026-10-28): `Page.navigate` acknowledgment is not proof that the new document is
ready. The MCP browser marks the old context and waits for its replacement plus complete readiness under one
10 s polling budget; same-document navigation checks the destination URL. Dev route pages also wait for
`#brain-view[data-brain-ready=true]`, set only after dynamic imports and mounting finish, not merely document load. Redirects may legitimately change the
final URL. Browser regressions assert successful navigation, typing and dirty form state before testing prompts.

Reliability update (2026-10-28): private CLI Chrome instances run in their own process group. Dropping an
instance cancels and joins its watchdog before reaping the browser, kills only that owned group (including workers),
and removes its temporary profile; teardown never waits for a stalled CDP `Browser.close`. CDP input is bounded to
two queued frames, each at most 32 MiB. Load sidecars likewise retain at most two frames (16 MiB per frame); load,
RPC, guard/middleware and form responses share one absolute 30 s deadline and kill/reap Node on timeout or protocol
closure. Irrelevant response frames cannot renew the deadline. These limits bound protocol retention, not the
whole process heap or detached descendants of application server code.

Priority order for every decision: (1) an agent can finish the task alone, (2) correctness, (3) speed, (4) human
ergonomics. A feature that only a human can use is never on the critical path.

**The tool never blocks an agent**
- No interactive prompts, no TTY assumptions, no pagers; every command terminates (default timeouts, `--timeout`).
- Every failure is machine-readable (`--json`, stable `schemaVersion`) and actionable (`fix`, `edits`, or the exact
  next command). Unknown internal failures print what to run next (`brain info --json`), never a bare stack trace.
- Commands are idempotent and recoverable: re-running is safe; every write goes to the undo journal.
- MCP outputs are bounded: large results are paginated (`cursor`) or summarized with a path to the full artifact,
  so an agent's context never overflows.

**Everything a human would check, the agent can check**
- Visual result: `brain_shot` returns the PNG (multimodal models read it) plus the accessibility tree as text.
- Behaviour: `brain_navigate/click/type/eval` reproduce any interaction; `brain_errors` and `brain_logs` show what
  happened. Performance and memory: `brain_inspect` (JSON). The devtools UI is a convenience on top of the same data.
- "Judgment" errors (alt text, heading structure, labels) are decided by the agent; the tool states the rule and
  the location, never waits for a person.

**Stuck-loop protection**
- `brain fix`/`brain_check` track repeated diagnostics: the same code at the same place after 2 fix rounds returns
  `stuck: true` with `brain explain <code>` output and a minimal correct example, instead of the same message again.
- `brain doctor` exit 0 is the agent's definition of done; `AGENTS.md` generated by `brain new` says so.

**Measured, not assumed**
QA's autonomy benchmark: complete multi-view apps from a one-paragraph brief, through MCP only, no human turns.
Metrics: tasks finished with `doctor` = 0, tool calls, tokens, stuck loops, and human-intervention count (target 0).

## 26. Stronger compression and minification — production only (fixed 2026-10-13)
`brain run` stays fast and readable; everything below runs only in `brain build`.

**Minifier (`brain-min`), using what the compiler knows:**
- Constant folding and dead-branch removal (`if (false)`, `PUBLIC_*` substituted values).
- Inline single-use functions and constants; drop unused exports across the whole module graph (tree shaking).
- Property mangling of compiler-generated objects (props bags, row records, internal state): safe because codegen
  owns those shapes; user objects and anything crossing JSON/`fetch`/`postMessage` keep their names.
- Template minification inside `tpl()`: collapse whitespace, drop optional quotes and optional closing tags.
- CSS: merge identical rules across components, shorthand merging, shortest color/number forms, remove unused
  selectors (the compiler knows every class a component can produce, `Output.names`).
**Bundler (`brain-pack`):** extract chunks shared by ≥2 views; inline chunks < 1 KB into their importer; inline
critical CSS per view; exact `modulepreload` graph.
**Compression:** gzip with optimal parsing (Zopfli-style iterations; slower, ~4–8 % smaller, build-time only),
zstd dictionary deltas for returning visitors (§19), and build-time Brotli (2026-10-29): latest stable Rust brotli 9.1.1,
quality 11/window 22, content cache keyed by codec/settings, two compression workers, emit only when smaller than raw.
Static Rust and Node servers negotiate br/gzip weights and exclusions; Caddy enables both, nginx documents ngx_brotli.
`brain build --report` prints, per view, raw → minified → gz → dcz bytes and what each pass saved. `--strict` turns the warnings that mean an incomplete `dist/` into a failure (`degrades_output`, `main.rs`); a throwing prerender is the case reached in practice, because a missing Node already stops the build.

**Portable static policy (2026-11-04):** `brain new --static` selects a static scaffold in memory and saves
brain.json output:static. `static_export.rs` resolves the persistent policy for CLI, doctor and MCP; an explicit
build --static/MCP static:true strengthens it without global state. Backend declarations are checked before the
build writes dist/. `BuildOptions.static_export` rejects gateway policies, folder guards/middleware, reachable
server functions, live sitemap/server forms and SSR plans. Build-time paths/load/head/seo remain available.
Invalid paths parameters and degraded prerenders fail before writing publishable output; a final invariant rejects
server.mjs/_server artifacts. HTML, Markdown and generated OG destinations use the shared brain-lex URL-path decoder once per segment;
URLs stay encoded for SEO and links. Encoded separators, dot-segments, invalid UTF-8 and controls fail before output.
Destinations are checked for file/directory collisions and symlinks; stale cleanup never follows symlinks.
Static mode forces web extensions and disables the stale-chunk grace on successful
export, so old SSR output cannot survive. It uses the same SEO, minification and raw-plus-precompressed pipeline;
optional host configurations are hints, not a portable guarantee of HTTP headers or redirects.

## 27. AI-driven development: closing the gaps (fixed 2026-10-13)
Goal: the agent composes verified building blocks and applies changes transactionally; it writes free-form code
only where the product is unique.

| Gap | Feature | Why it helps an LLM |
|---|---|---|
| Rewriting common structures from scratch | **Recipes:** `brain gen <recipe> --params '{…}'` · `brain gen --list --json` (each with a params schema). v1 set: app shell with sidebar + topbar, navbar/menu (responsive, keyboard accessible), auth pages + `_guard`, CRUD from an `api/*.jsbrain` schema (list with filters + detail + form), dashboard KPIs + table, settings page, pricing, landing sections (hero, features, FAQ, CTA), wizard, modal flow, empty/error/loading states | every recipe passes `brain doctor` by construction: 100 % first pass on the common 80 % of an app |
| CSS mistakes in layout | **Layout primitives** in `brain/ui`: `Stack`, `Row`, `Grid`, `Center`, `Sidebar`, `Container`, `Spacer` with token-only props (`gap="3"`) · **themes:** `brain theme <preset>` writes `tokens.cssbrain` with AA-verified pairs | structure without writing CSS |
| Multi-file changes that half-fail | **Transactional plans:** `brain apply plan.json` — a list of operations (gen, add, edit, write, remove) executed atomically; if `brain doctor` fails afterwards, everything rolls back and the report says which step broke | one call, all-or-nothing |
| Reading too much code | `brain read <file> --outline` (props, emits, state, handlers, children, imports — no bodies) · `brain map --focus <file\|route>` (subgraph + neighbours) · `brain context "<task>" --budget 8000` (minimal file set for a task, ranked by the graph, within a token budget) | understand a project in hundreds of tokens, not thousands |
| Screenshots cost tokens / need vision | `brain shot --text`: semantic outline of the rendered page (landmarks, headings, controls with labels and states, visible text, layout boxes) | text-only models can "see" the UI |
| Reviewing its own change | `brain diff --semantic`: components, props, routes, channels and tokens added/removed/changed since the last journal entry | self-review before finishing |
All of it is exposed by MCP (`brain_gen`, `brain_apply`, `brain_read {outline}`, `brain_context`, `brain_shot {text}`,
`brain_diff`) with input/output schemas.

## 28. Incremental compilation: near-instant dev (fixed 2026-10-13)
Build work audit (2026-10-31): `brain-metrics` holds one `Build` per run, attached to each worker thread, so concurrent
builds never mix. It reports wall time, the CPU of that build's threads (child processes excluded) and executed-work
counters, in quiet mode too. It also memoizes facts per source revision: dependency hashes are keyed by a stamp of length,
mtime, ctime and inode, and a final snapshot warns when a source changed during the build. `cache_budget.rs` amortizes
maintenance: an entry reservation only scans the directory when the budget runs out, the budget halves against the real
free space (O(log N) scans), and eviction goes to a 7/8 low-water mark while protecting entries the running build used.
Reads verify SHA-256, so writes no longer fsync. A parsed-AST memo was measured and rejected: cloning cost what parsing
cost. An `.nextSibling` chain helper too: at most -0.08 % gzip and +0.2 % Brotli on a new app.
Cache audit (2026-10-28): persistent lookup now hashes exact source bytes directly; it does not parse an AST just
to look for cached output. HMR still uses parsed block/contract classification. Both key APIs include adoption,
lint, root, runtime imports, locale and public environment options with unambiguous field boundaries. Compiler
fingerprints include runtime primitive exports/kit inputs, build scripts and manifests as well as compiler Rust.
Context dependencies include absent nearer token files, configs, layouts, shared types and locale files. Provider
and canonical checks additionally track graph membership. Missing dependency metadata is a cache miss, not an
empty dependency list. Negative dependencies distinguish absence from unreadable input. Hashes remain noncryptographic
cache identifiers; they are not security signatures. The compiler cache is local, trusted and never a remote artifact store.
The CLI's block/HMR reuse layer also validates dependency fingerprints; unchanged component text alone is not a
cache hit after a token/config/schema change. That optional layer retains at most 256 entries / 64 MiB of output
(1 MiB per entry); the dev server map retains at most 1024 entries / 128 MiB (16 MiB per entry). Those in-memory
layers clear on budget pressure rather than implementing a second LRU list. Returned outputs still belong to their
request owners. Graph dependencies match newly created descendants as well as directly imported files.

Reliability update (2026-10-28): the CLI compiler cache caps complete entries at 16 MiB each, 128 MiB total and
1024 entries per project, evicting the oldest writes. Reads reject oversized/corrupt entries; dependency hashes stream
in 64 KiB chunks. Concurrent writers use process ID plus an atomic sequence for temporary names and publish by
rename. Temporary files from other writers are not evicted. Dev cache misses are serialized to prevent duplicate
compilation; this is a single-flight correctness guard, not parallel compilation. The watcher queue holds 64 events,
batches for at most 50 ms, and coalesces overflow into cache/environment/sidecar invalidation plus reload. Dev admits
at most 64 active connections (including WebSockets and sidecar waiters); sidecar queue waits expire after 5 s,
independently of the running request's 30 s response deadline. Excess connections receive 503 with
Retry-After. Request reads have one total 5 s budget so trickling bytes cannot monopolize a slot. Stopping dev shuts
down owned WebSocket sockets to release their reader threads. Imported-module and token edits invalidate cached parents even if their own mtimes are unchanged. Server-code edits
restart the sidecar to discard Node's old module versions. Existing cache architecture and remaining work follow.

Server capability ownership (2026-11-04): codegen emits `brain_lex::ServerMetadata` from top-level exported
function ASTs and the same namespace that generates RPC stubs. CLI compilation, pack units and dev compiled units
carry load/paths/sitemap/forms flags and exact RPC IDs; source examples, comments and nested functions grant nothing.
Build route classification, production RPC manifests and dev load/sitemap/RPC dispatch consume these facts, never
search emitted strings. Compiler cache format -3 requires all metadata fields and its budget evicts both old -2 and
new -3 entries. No-op/style-only HMR preserves the same metadata; script changes invalidate it. Dev refuses duplicate
RPC owners and incomplete inventories (20,000 entries, 4,096 candidates, 32 directory levels); links and special
files are not followed. This is module discovery, not arbitrary source evaluation.

Dev cache correctness (2026-11-04): `server.rs::compile_cached` reads and SHA-256 hashes exact source bytes before
returning an in-memory hit. Equal-length in-place writes with restored mtimes cannot serve stale output, and a
metadata-only save reuses compiled output. Reads and compiles share the single-flight lock; watchers still invalidate
dependants. The cache dies with the process and its granularity remains the whole file.

**Dev is never minified, mangled or compressed.** Speed comes from doing less work, not from doing it faster:
- **Content-hash keys:** every stage result is keyed by a hash of its exact input (source bytes + compiler version +
  options). Saving a file without changes costs one hash.
- **Stage granularity inside a file:** a `.brain` is split into `meta`, `script`, `markup` and `style` blocks, each
  hashed separately. Style-only change → recompile CSS only and hot-swap the sheet; markup/script change → re-run
  codegen for that component only; `meta props/emits` change → also re-check the direct importers (their contract
  changed), nothing else.
- **Persistent cache** in `.brain/cache/` (AST, diagnostics, codegen output per block hash): a restart of `brain run`
  or `brain build` starts warm. Invalidated wholesale when the compiler version changes.
- **Persisted dependency graph:** imports, component usage, channels and tokens; a token change re-checks only the
  components that use that token.
- **Parallel cold start:** files compile on `std::thread` workers (no new dependencies).
- **Build reuses dev work:** `brain build` takes cached codegen outputs and re-runs only minify/pack/compress for the
  chunks whose content hash changed (content-addressed names make the rest no-ops).
**Targets, measured by `brain run --profile` and reported per edit:** ≤ 5 ms from file event to compiled output for a
component edit, ≤ 20 ms to the browser (excluding the OS file-event latency), 0 ms for a no-op save; cold start of a
200-component app ≤ 300 ms with a warm cache.

## 29. Opinionated architecture and generators (fixed 2026-10-13, Chris: "like the Angular CLI, but for LLMs")

GitHub Pages static host preset (2026-11-05): AssetLayout chooses host defaults independent of field order, requires
a valid HTTPS origin separate from base, and emits root .nojekyll plus a fallback404 only when absent. Pages never
move below the URL base on disk. CLI/MCP deployment preview proposes a preserved/CAS-checked workflow under deploy/,
then requires review before installing it in .github/workflows/. The workflow validates configure-pages base metadata
and uploads dist only; it does not mutate Pages configuration or publish during preview.

`scaffold::AddOptions.blank` selects minimal noninteractive component markup with a slot, typed props and the normal
colocated test. CLI `brain add comp --blank` and MCP `brain_add` share this path; all other kinds reject the option
before mutation, and ordinary component generation retains its existing example template.

Build attribution (2026-11-04): `brain-lex::branding` is the shared landing/comment authority. Own graph JS, token/view
CSS, dictionaries, locale links and analytics bootstrap add code comments after minification and before content
hashes. Framework documents and private fixed-name server modules use format-safe attribution before cache/SRI.
Robots and normal/news/index/dynamic/dev sitemaps are covered; XML attribution follows the declaration and counts
against sitemap header reservations. Dynamic runtime split headers use the same signature and size accounting.
`brain-build.json` carries deterministic generator/version/home metadata for all output, including original binary
and protocol assets whose bytes/schemas stay unchanged; it is reserved and inserted before build-signature/SW work.
Host hints and cleanup workers retain the signature. Page credit/version-label options do not remove build attribution.
Explicit project opt-out (2026-11-05): brain-lex::branding::enabled validates root compilerBranding as a unique
boolean, default true. Only false disables attribution; SEO credit/version labels are independent. The shared policy
reaches code emission before hashes, SEO text/metadata, SW/offline, protocol metadata, host hints and both HTTP
gateways. Node's owned emitted source omits its attribution URL literals; dev HTML/XML and deployment workflow
previews also honor the policy. Copied public/foreign bytes are never filtered. Ordinary build grace is disabled
on policy transitions so obsolete branded chunks are removed, never rewritten under an existing content hash.
LLM-facing adoption examples preserve true; false requires an explicit end-user request.
Generator discovery copy (2026-11-05) appends branding::DESCRIPTION to the escaped HTML generator version label.
It names the Rust frontend/full-stack compiler without adding site-topic keywords or changing authored metadata.
The entire tag still follows compilerBranding. npm descriptions/keywords separately name actual JavaScript,
Node.js, SSR, static-site and SEO capabilities; those search terms do not change application output.


Living brandbooks (2026-11-04): `brain-cli::brandbook` owns a bounded declarative section structure and the
explicit `design.brain.json.brandbook` editorial contract. A structural AST marker in the real brandbook route is
expanded by compiler::check and compile_hash before parse/codegen/cache identity; generated markup occupies one
source line and its diagnostics map to the owned marker with automatic edits removed. Authorial lines stay stable.
Component-scope collision planning uses the same expanded input. Design/token files and UI directory/file membership
are dependencies; current facts read live tokens rather than the stored snapshot. Rendering only escapes data and
lists filenames, never evaluates app modules. Source JSON and authored pages without the marker are preserved.
New static/ordinary scaffolds include the route and navigation; design-enabled older apps get warning B2202.
B2203 rejects malformed explicit managed content. Six generic groups cover strategy, voice, identity/product,
assistant behavior, channels/governance and research; drafts and source observations never imply approval.
Multilingual managed content (2026-11-05): generated literals reuse registered default-dictionary keys through the
same B1403 allocator. Only the owned line is transformed. `brandbook.messages` persists original text/key identities
so default translations can change; explicit fix/add-locale applies the bounded mechanical B1403 design edit.
Check/build only read design and locale dependencies. Untouched draft prompts stay in the editable contract while
the public page shows a brief pending notice; all section IDs/titles, authored content, statuses and evidence remain.
Nested checks allocate new keys with the same reported file scope as the checker, then persist those identities.
Token category IDs and CSS contexts are code, like token names/values and file paths; locale extraction never
translates these source facts. Human headings and editorial content still use the normal dictionaries.

Scaffold component tests (2026-11-04): `write_test` parses the component's actual prop AST before creating a test.
Required strings/any receive example text, numbers zero, booleans false, lists/objects empty containers and functions
no-op callbacks. Optional/defaulted props remain omitted. Nested application shapes still require authored fixtures.
Missing/invalid components fail before creating an unusable test file. CLI add and MCP share this implementation.

Scaffold package installation (2026-11-04): the exact compiler pin lives in dependencies because generated start/backend/worker commands execute it in production. Existing devDependency pins remain readable by upgrade/delegation; no existing package manifest is silently moved. Static output still deploys as dist/ only.
- One folder per kind, flat by kind (predictable beats clever for a model): `views/` (routes, layouts, guards),
  `comp/`, `ui/`, `channels/`, `services/`, `api/`, `types.jsbrain`, `lib/`, `server/`, `tokens.cssbrain`, `app.brain`.
  Large apps may group by feature with the same sub-structure (`features/billing/{views,comp,channels,api}`);
  `brain map` and the router treat both the same.
- **Initial structure (`brain new`, Chris 2026-10-13: "the organization the framework expects, adopted by default"):**
  every project starts with the full layout, so the model never invents folders and every generator has its place:
  ```
  app.brain            static document shell: <head>, site <meta seo>, skip link
  tokens.cssbrain      design tokens (white glass theme)
  brain.json           name, site URL, cache, seo.minScore, budgets
  .brain_env           variables per environment (§36), gitignored · .brain_env.example committed
  .gitignore · APP.md (phases) · AGENTS.md (how an agent works here)
  views/               home.brain (/), about.brain, 404.brain, 403.brain, _error.brain, _loading.brain,
                       _offline.brain, login.brain, posts/[id].brain, account/_guard.jsbrain, account/index.brain
  comp/                counter.brain + counter.test.brain
  ui/                  badge.brain
  channels/ services/ api/ lib/ server/ content/   one README.md each (plus the example files below): what goes there + the `brain add` command
  public/              favicon.svg, icon-192.png, icon-512.png (manifest), og.png
  ```
  **Basic features already solved** (Chris 2026-10-13): the scaffold is a small working app that shows each core
  feature once, the way the framework expects it, so the model copies a correct example instead of inventing one:
  | Feature | Files | What it shows |
  |---|---|---|
  | Routing | `home`, `about`, nav in `views/_layout.brain` | static routes, active link, View Transitions |
  | Dynamic route | `posts/[id].brain` + `lib/posts.jsbrain` | `paths()` + `load(ctx)` with `ctx.params`, SEO per page |
  | Not found | `404.brain` | returned for unknown URLs (status 404), link back home |
  | Permission denied | `403.brain` | a guard's `deny('reason')` renders it with status 403 and names the missing permission; never a blank page |
  | Error screen | `_error.brain` | error boundary: what happened, a retry button, the request id; no stack in production |
  | Loading | `_loading.brain` | shown while `load` runs (skeleton, no layout shift) |
  | Offline | `_offline.brain` | the service worker's fallback (§35) |
  | Protected view | `account/_guard.jsbrain` + `account/index.brain` | `ctx.user ? true : redirect('/login?next=' + ctx.url.pathname)`; shows the user and a logout |
  | Login and session | `login.brain` + `server/auth.jsbrain` + `channels/session.jsbrain` | `form()` with validation and server errors, a `server:` function, the signed session cookie, the nav reading the session store |
  | Tests | `comp/counter.test.brain`, `views/login.test.brain` | a component test and a form test |
  The demo login accepts the password in `DEMO_PASSWORD`, which exists only in the `[development]` section of
  `.brain_env`; in production the key is absent, so `login.brain` says auth is not configured and how to wire a real
  provider, and no demo credential can ever reach a deployed app.
  Folders without code yet hold only that README (git keeps them, `brain doctor` ignores `.md`), never placeholder code
  that would be dead. The generated full app passes `brain doctor`, may include SSR for protected/server routes, and scores ≥ 80 on every page with
  no edit (nav links every view, no orphans). `types.jsbrain` is created by the first `brain add type`.
- `brain add <kind>` is the only sanctioned way to create a file. Every generator: writes the contract first
  (`<meta props/emits/expose>`, typed stubs), uses `brain/ui` and layout primitives, passes `brain doctor` as
  generated, colocates a test, records the operation in the undo journal, and updates nothing else silently (it
  prints which importers/routes it touched). Generators are data (templates + params schema) so recipes (§27) reuse
  the same engine.
- `brain check` warns (B1501) on a file outside the layout or a name that breaks the convention, with an `edits` fix
  that moves/renames it via `brain edit move`.
- The same table is in SPEC.md («Architecture»); the planned rows unhide as each generator ships.

## 30. Runaway protection: loops and growing memory (fixed 2026-10-13, Chris: "the compiler must fail to protect the browser")
Three gates, so a looping or ever-growing component never reaches a user's browser, and if it does, it cannot take
the tab down.

**1. Compile time (`brain check`, errors that block `brain build`) — COMPILER**
- B0901 (exists) static signal cycles; B0903 (exists) recursion without base case.
- B0905 an `fx` (or a timer/interval/subscription callback inside it) that appends to a list it also reads, with no
  bound (`items = [...items, x]` in `setInterval`, in `bus.on`, in an `fx` reading `items`): unbounded growth.
- B0906 a timer, interval, listener or subscription created outside `fx` or without cleanup inside a component.
- B0904 (exists, warning) a list fed by user or network input without `slice`/limit: stays a warning.

**2. Build-time runaway probe (`brain build`, `brain doctor`, `brain test`) — TOOLCHAIN**
After prerender, every view and every island is mounted in the Node DOM shim with fake timers and stepped for
N simulated seconds (default 30 s of virtual time, ~200 ms real) with no user input. The probe samples per
component: DOM nodes, live signals/effects, list lengths and effect runs per tick. Linear growth with no input, or a
requestAnimationFrame loop that re-arms itself on every frame for the whole run, fails the build with B0907 naming
the component, the signal and the line (a timer that legitimately ticks, such as a clock, is not a runaway),
and the measured growth (e.g. "+40 nodes/s, +1 item/tick in `items`"). `brain.json` → `"probe": { "seconds": 30 }`.

**3. Runtime circuit breaker (prod and dev) — RUNTIME**
- Timing correction (2026-10-28): the first runs window begins at component creation, not clock zero. A controlled
  clock regression verifies that crossing the process second boundary cannot shorten a new instance's budget.
- B0902 (exists): an effect that re-runs > 100 times in one flush is stopped.
- Per component instance budgets: DOM nodes (default 200 000) and list rows (default 100 000), measured once per flush
  with a native count, runs of one effect per second (default 1 000), and continuous work per flush (default 200 ms).
  Crossing one trips
  the breaker for THAT instance: its effects and timers are disposed, its fallback (`<meta fallback>` or a neutral
  "This part stopped to protect the page") replaces it, the error is captured (B0908 with the budget, value and
  component), and the rest of the app keeps running. Budgets are overridable per component with
  `<meta budget="rows:200000">` for legitimate large lists (the kit `Table` virtualizes instead).
- Dev: the breaker also opens the error overlay and fails `brain doctor`.

## 31. Proactive protections — full inventory (fixed 2026-10-13)
Everything the compiler/runtime guards by default, so an agent cannot ship the classic web failures. ✓ = already
specified elsewhere; ➕ = new in this section.

| Area | Protection | Where | Status |
|---|---|---|---|
| Loops & growth | static cycles, recursion, unbounded append, timers without cleanup, build-time probe, runtime breaker | §18, §30 | ✓ |
| Memory | leak detection on unmount, `brain test --heap` | §9 | ✓ |
| XSS | escaped interpolation; raw HTML only via `sanitize` | §3, SPEC «Markup» | ✓ |
| XSS via URLs | every URL attribute (`href`, `src`, `action`, `formaction`, `poster`, `xlink:href`, `cite`, `data` on `<object>`, `background`, `longdesc`, `manifest`, `codebase`, `icon`, each `srcset`/`imagesrcset` candidate, each `ping` URL) bound to an expression is checked at runtime, in the browser and in SSR: `javascript:`, `vbscript:` and `data:` (except `data:image/*`) are dropped and reported (B0213); a static one is a build error. The runtime (`runtime/brain.js` `URL_ATTRS`) and the checker keep one list, compared by a test | COMPILER + RUNTIME | ➕ |
| XSS via `srcdoc` | a bound iframe `srcdoc` needs `sandbox` without `allow-same-origin` (B0210): otherwise the value renders as a same-origin document | COMPILER | ➕ |
| Server code on a host | one list of private dist/ paths (`brain-pack` `private.rs`: `_server`, `_brain`, `server.mjs`, `_headers`, `_redirects`, dot files but `.well-known`) feeds the Rust static handler, `_brain/nginx.conf`, `_brain/Caddyfile` and `brain deploy config nginx`; the Node server keeps the same list (a test compares them). `_headers` cannot refuse a path, so a server build's `_headers` says not to upload it to a static host | TOOLCHAIN + RUNTIME | ➕ |
| Tabnabbing | `target="_blank"` gets `rel="noopener noreferrer"` automatically (info diag) | COMPILER | ➕ |
| Prototype pollution | `api()` validators and `store` `sync` drop `__proto__`, `constructor`, `prototype` keys from untrusted JSON | RUNTIME | ➕ |
| Open redirect | `redirect(url)` in guards/RPC only to same origin unless the host is listed in `brain.json` `"redirects"` (B0710); browser navigation to a runtime URL only after a check against this origin or a listed host, or to a URL the app's own server answered (B0814) | COMPILER + TOOLCHAIN | ➕ |
| SSRF | server `fetch` whose URL is built from `ctx.params`/`ctx.query`/RPC args without an allowlist: warning B0711 | COMPILER | ➕ |
| CSRF / abuse | RPC same-origin + header, body limit (brain.json `server.maxBodyKB`, 1 MB default), rate limit | §11.1 | ✓ |
| Secrets | env and key-shaped literals never reach the client | §11 | ✓ |
| Cookies | session cookie forced `HttpOnly; Secure; SameSite=Lax`; `document.cookie` writes of names starting with `__Host-`/`session` are a build error | TOOLCHAIN + COMPILER | ➕ |
| Headers | CSP with nonce, SRI, plus `Strict-Transport-Security`, `X-Content-Type-Options: nosniff`, `Referrer-Policy: strict-origin-when-cross-origin`, `frame-ancestors 'none'` (clickjacking), `Permissions-Policy` minimal | TOOLCHAIN (server config + server.mjs) | ➕ (CSP/SRI ✓) |
| Supply chain | imports by URL (`http(s)://`) are a build error (B0122); any third-party `<script src>` in `app.brain` requires `integrity` (B0123) | COMPILER | ➕ |
| ReDoS | `new RegExp(<user input>)` without escaping: warning B1311 | COMPILER | ➕ |
| Hanging work | `fetch`/`api()` get a default timeout (10 s, AbortSignal); server `load` and RPC time out (10 s → 504); slow ones are reported | RUNTIME + TOOLCHAIN | ➕ |
| Races | `resource()` and `load` navigation cancel stale requests; only the latest response is applied | RUNTIME | ➕ |
| Double submit | `on:submit` handlers that return a promise are not re-entered until it settles (button gets `aria-busy`) | RUNTIME | ➕ |
| Payload size | `load` result serialized into the page > 128 KB: warning B0712 (paginate or fetch on demand) | TOOLCHAIN | ➕ |
| Large state | a `sig` holding > 10 000 items rendered without virtualization: warning B1310 (use `Table` or paginate) | COMPILER + RUNTIME | ➕ |
| Deploy skew | a chunk that 404s after a new deploy triggers one full reload instead of a broken page | RUNTIME | ➕ |
| Storage | `store` `sync:'storage'` writes are size-checked and quota errors are caught and reported, never thrown | RUNTIME | ➕ |
| Hydration | dev compares prerendered HTML with the first client render and reports mismatches (B0214) | RUNTIME | ➕ |
| Error boundary | every view has a fallback even if the author wrote none | RUNTIME | ➕ |
| Privacy | breadcrumbs/errors never include input values, cookies or query strings | §18 | ✓ |
| A11y / SEO | build-blocking rules | §8 | ✓ |

### 8.1 Literal colors and spacing (fixed 2026-10-13)
- Color properties (`color`, `background(-color)`, `border(-*)-color`, `outline-color`, `fill`, `stroke`, `caret-color`)
  with a literal color: B0304 error; `edits` replace it only with the token whose value matches exactly (an opaque
  literal; a translucent background becomes `--scrim`). Otherwise the fix names the nearest color token and its value
  without writing it, because it changes the color (2026-11-02). `filter`/`box-shadow` colors and SVG attributes in
  markup are free.
- Spacing properties (`padding*`, `margin*`, `gap`, `row-gap`, `column-gap`, `inset*`) with a literal length equal to a
  token value: B0305 info, normalized to the token (§21); any other literal: B0305 warning. `0`, `auto` and
  percentages are always allowed. Sizes (`width`, `height`, `max-*`, `font-size`, `border-radius`, `line-height`) are free.
- Implementation notes (#97, accepted; narrowed 2026-11-02): B0304's edit is the exact-value token only; the
  nearest by RGB distance is a suggestion in the fix, so no write changes a color the author chose. B0125 (markup required) counts `<meta>` and
  `<script src|type>` in `app.brain`'s head as markup. B0126 exempts `*.test.brain`.

## 32. Dynamic metadata, generated SEO files and WebMCP (fixed 2026-10-13, dynamic sitemap 2026-10-29, imperative tools 2026-10-30)

Evaluated discovery metadata (2026-11-05): pack Page.lastmod carries the merged static/evaluated lastmod value
from each rendered route to the CLI SEO post step. This keeps load-derived per-page dates consistent with the
head/JSON-LD instead of rereading only source-text meta attributes. Empty values are omitted; static source values
remain a compatibility fallback. This does not derive a content-change date or run per-request SSR for a sitemap.

Static layout byte accounting (2026-11-04): local emitted paths are independent of _b. The route graph resolves
JS/CSS from text and binary maps, including public modules and their relative imports, with hosting-base confinement.
Service-worker registration is not a page module import: ledger.serviceWorker and kind:service-worker expose its
actual bytes separately, outside the existing view budget. The default 30 KiB budget and its tests stay unchanged.

Public asset containment and relative graphs (2026-11-04): pack validates the input public tree before graph work
in static and ordinary modes. Symlinks/special files are refused; depth and entry count are bounded. Cache walks also
skip non-regular entries rather than following aliases. Raw public JS/MJS/CSS keep authored URLs and dependency
resolution, like streaming manifests; compiler-owned modules/styles still rewrite before hashing. Planned immutable
public destinations enter the pre-cache SEO inventory, so download links are not falsely reported absent.

Zero-JS static artifact cleanup (2026-11-04): when every emitted page passes the existing zero-route proof,
remove compiler-generated JS not reachable from explicit document references and their literal module/URL imports
before SEO, cache inventories and precompression. This is static-mode-only. Public files are copied afterward and
remain available to dynamic/external/download references; the disabled-cache cleanup worker is still emitted.

Static artifact doctor (2026-11-04): `brain-cli::static_audit` runs only after a fresh successful static build.
Doctor compiles into a uniquely reserved private temporary directory and removes it through an owned guard after
success or failure. Build, payload, SEO and artifact gates inspect that exact candidate; none reads or replaces the
app's deployment dist/. Old-chunk grace remains owned by explicit deployment builds, not isolated validation.
Compiler caches and diagnostic feedback may refresh. `build.outputScope` and `outputRetained` describe the evidence;
an explicit `brain build` still writes deployment output. Release-check retains its environment-specific build.
It reuses `brain-seo::html::elements` and the packer's literal module/URL reference scanner. The bounded owned CSS
scan skips comments and strings, handles compact/comment-separated imports, and fails incomplete on escaped URLs.
Resolution uses ordinary directory indexes, the hosting base and strict percent-decoded confinement, with no app
server extensionless fallback. Local targets, anchors/sprite IDs, duplicate document IDs, ARIA/HTML ID associations
and final SHA-384 SRI are checked. ID associations resolve in the same document, only against real IDs (not named
anchors), and share the bounded reference inventory. The scanner marks nested template contents as outside the
document ID scope: inert/declarative-shadow fragment IDs cannot satisfy outer references or create false document
duplicates. Resource declarations inside templates are still inventoried; instantiated fragments/shadow roots need
browser tests. Stronger unknown
SRI is incomplete, never a success. Digests are reused per target. External services, dynamic URLs and JS execution
are explicitly outside this declaration gate. CLI and MCP expose `staticArtifact` and include it in failed gates.

Static build customization (2026-11-04): `brain-pack::asset_layout` validates staticOutput before graph emission.
JS/CSS/image/other destinations are resolved before imports and content hashes; inlineCss controls stylesheet
inlining, and fast/small compression uses independent setting-qualified caches. Public originals and foreign SDK
relative trees retain their identity. Prerender resolves exact emitted URL mappings, not a hardcoded _b prefix.
An immutable generated-file inventory drives custom cache/SW hints; public files in the same directories are not
made immutable by a directory-wide rule. SRI, preloads and route ledgers resolve actual local emitted paths.

**Metadata (COMPILER + TOOLCHAIN + RUNTIME)**
- `<meta seo>` attributes accept expressions over `props` (the `load` result); `jsonld={obj}` serializes to
  `<script type="application/ld+json">` (escaped `</`), and `schema="Article"` stays a shortcut: one node of that
  type from the page's title (`headline` for article types, else `name`), description, canonical, `og:image` and
  `lastmod`, written by prerender `seo_head` and, for SSR pages, by server.mjs `head()`, which replaces the template's
  node; never a `<meta name="schema">` (fixed 2026-10-29). `app.brain`'s
  `<meta seo>` holds site defaults (`titleTemplate`, `og:image`, `twitter`, `locale`); views override per page.
- A view may contain a `<head>` block for anything else (alternate hreflang, preload, custom meta). Merged into the
  document head at SSR/prerender; deduplicated by `name`/`property`/`rel+hreflang`; on client navigation the router
  swaps every route-owned tag: title, description, canonical, hreflang and Markdown alternates, `prev`/`next`, og:*,
  twitter:*, robots and JSON-LD. The page's own tags replace the previous page's, and none stay when it has none (S04);
  icons, stylesheets and the RSS feed are site tags and stay.
- Dynamic routes export `paths()` → prerendered per param and listed in the sitemap; without `paths()` they are SSR.
- A dynamic route without `paths()` (live content: SSR) may export `sitemap()` instead: it returns the params of its pages
  (`[{ slug: 'a', lastmod: '2026-10-01' }]`) and gets `{ env }`. The manifest marks the route `sitemap: true`, and
  server.mjs serves /sitemap.xml as the build's file plus those URLs (a sitemap index gets one more child,
  `/sitemap-dynamic.xml`), with the origin of the build's own sitemap. One file holds at most 50 000 URLs and 50 MB of
  UTF-8 XML (escaping included; S01): when the build's URLs plus the dynamic ones pass either limit, /sitemap.xml
  becomes an index of `/sitemap-static.xml` (the build's file) and `/sitemap-dynamic.xml`, `/sitemap-dynamic-2.xml`, …
  filled in `sitemap()` order. Dynamic URLs past 50 000 are dropped with a warning, and a URL over 2 048 characters is
  left out with a warning. The build splits its own sitemap the same way (`sitemap-N.xml`, by count and bytes) and
  warns about each URL it leaves out. The list is kept 5 min
  (`max-age=300`; requests after expiry share one run); a throwing `sitemap()` is logged and skipped. B0117 outside a
  dynamic view, like `paths()`, and in a view that also exports `paths()` (those pages are prerendered and already
  in the build's sitemap). The dev server (`brain run`) answers /robots.txt, /sitemap.xml (+ sitemap-N.xml) and
  /llms.txt (+ /<locale>/llms.txt) itself, with no build step: the origin is the request's Host, the pages are every
  static route plus what `paths()` or `sitemap({ env })` returns (sidecar), in every locale, without noindex and error
  views; a file in `public/` wins. Each page (up to 300) is rendered once through the dev page path, and the files come
  from the same `brain_seo::files` generators as the build: titles and descriptions in llms.txt, hreflang alternates
  with x-default, and image entries for the view's literal `<img src>` (images bound to props need SSR, so only the
  build lists them). Static hosting has no
  server, so the route needs `paths()` there. Not in llms.txt (built before any request).
- B1601 view without description; B1602 title longer than 60 or description longer than 160 chars (warning);
  B1603 invalid JSON-LD (missing `@type`); B1604 duplicate canonical across views.
**Generated files (TOOLCHAIN, `brain build`)**: robots.txt (with sitemap URL; `noindex` views stay crawlable so
Google can read their `noindex`, and are left out of the sitemap, like the not-found page `*`; staging disallows everything), sitemap.xml (static +
`paths()`; `lastmod` = the collection frontmatter date or `<meta seo lastmod="YYYY-MM-DD">`, omitted when there is no
date, never the file mtime: invariant 8, deterministic builds), llms.txt (site name, description, every page with title, URL and
description; a section is headed by its own page's title, `/cases` → «Customer stories», else the folder name), llms-full.txt (each page's prerendered main text as Markdown), manifest.webmanifest (name, colors from
tokens, icons the author provides in `public/`; sizes are checked, not generated), rss.xml (the site feed, like
WordPress `/feed`: every indexable page, newest `lastmod` first, 50 items, `<generator>`) and `<section>/rss.xml` for each
section without a content collection (a collection keeps its richer RSS + Atom at the same path), sections.json (the
manifest: site, generator, discovery files and every section with title, description, home page, feed and its pages
with URL, Markdown copy and lastmod; `crates/brain-seo/src/sections.rs`). Every page gets `<link rel="alternate">` to
`/rss.xml` and `/sections.json`; `seo.feed: false` / `seo.sections: false` turn them off. **Sitemap of a per-request site:**
`_server/manifest.json` carries `site`, so `/sitemap.xml` knows the origin even when the build lists no URL; public pages
rendered per request join the sitemap and llms.txt (llms-full.txt leaves them out: they have no prerendered text); no URL at
all is B1900. `sitemap()` of every view runs at once, an expired list is served while the next is prepared, and a view that
fails keeps its last good list. **Credit:** `seo.credit` (default true) closes every page, prerendered or a `_server/`
template, with «By BRAINLLM x.y.z» before `</body>`, outside `#brain-view` so client navigation keeps it; its inline padding includes `env(safe-area-inset-bottom)` and does not override app margins (2026-11-04); it is added after
the SEO audit and the Markdown copies, and its link is `nofollow` because a sitewide footer link reads as a paid widget to search engines. Attribution: next to
`<meta name="generator">` each page carries `<!-- by BRAINLLM https://chrisifm.github.io/BRAINLLM/ -->`, llms.txt and
llms-full.txt end with `## Built with`, and documents carry `X-Generated-By: BRAINLLM (<home>; <repo>)` from `brain start` and the generated `_headers`, nginx and Caddy configs, without a version.
All configurable in `brain.json`.
**Google tags**: from `brain.json` `"google"` only. Analytics/Tag Manager are injected with the CSP nonce and Consent
Mode v2 defaulting to denied until the app calls `consent('analytics')`; site verification becomes the meta tag.
**WebMCP (agent tools)** — support verified 2026-10-13: Chrome origin trial (149+), API name moving from
`navigator.modelContext` to `document.modelContext`; not a standard yet. Therefore:
- Status (2026-10-30): both paths ship. Declarative `<form tool>` is browser-native HTML; imperative `<meta tools>`
  (#106) compiles to `registerTools([...])` from `brain-mcp.js`, imported only by components that declare tools (no
  byte on other routes; B0128 no longer fires for it).
- RUNTIME `brain-mcp.js` (separate module): feature-detects either object; registration and lifetime are one helper
  (`own`) shared by `tool()` and `registerTools()`: `registerTool(def)` inside one effect of the calling component,
  `unregister()` (or `unregisterTool(name)`) on dispose; no-op when unsupported, during SSR (`__brainSSR`) and route
  prerender (`__brainRoute`). A registration the browser refuses (a name another mounted instance holds) is skipped,
  so the page keeps rendering and that instance's unmount leaves the other's tool in place. `execute` validates the
  arguments first (a bad one answers `field: message` and never runs the function) and answers with the result as
  text, or `Error: message` when it throws.
  `tool(name, description, params, run, { title, readOnly, consequential, untrusted })` adds `title` and
  `annotations: { readOnlyHint, consequentialHint, untrustedContentHint }` (WebMCP draft 2026-10-27), only the keys
  given (2026-10-29). `registerTools([{ name, description, inputSchema, execute, title?, readOnly?, consequential?,
  untrusted? }])` applies the same keys the same way; the `<meta tools>` mini-language has no syntax for them yet, so
  compiled entries carry none. Its arguments are checked against the compiled schema: required = present (`false` and
  `0` count), `string` with `format` email/uri/date and `number`/`boolean` through `brain-schema.js` `check`, `array`
  and `object` by shape, `{}` (`any`) unchecked.
- COMPILER: `<meta tools="name: description (param:type!, …)">` (tools separated by `;`, types `str num bool list obj
  email url date any`) + a same-named function → `{ name, description, inputSchema, execute: <function> }` per tool,
  the JSON Schema built at compile time (`tool_js` in brain-codegen); declarative `<form tool=… tool-description=…>`
  is emitted with the attribute names the current spec uses (one table in the codegen, updated when the spec
  settles). B1701 tool without a matching function, B1702 name not snake_case or duplicated on the page, B1703
  parameter without a type, B1704 (2026-10-30) the function destructures a key the tool does not declare (agents
  never send it; reported at the key) or the tool declares one the function never destructures (reported at the
  parameter list; a function without parameters counts). A whole-object parameter, a rest element, a computed key or
  a `server:` function are trusted (the checker cannot tell). No edits: either side may be the one to change.
- Tools run with the page's permissions only; anything destructive should ask for confirmation in the UI.

## 33. `brain-seo`: the SEO module (fixed 2026-10-13, Chris: "much more powerful than WordPress")
Owner: SEO role. Crate `crates/brain-seo` (no third-party deps), wired into `brain build`, `brain doctor` and the CLI as
`brain seo [audit|preview|og|links|redirects] [--json]`, and into MCP as `brain_seo_*`. Everything is computed at build
time from the prerendered HTML, the route graph and the content collections — no plugins, no database.

| Capability | WordPress (Yoast/RankMath) | BRAINLLM |
|---|---|---|
| Per-page audit with score | plugin, per edit | `brain seo audit`: score 0–100 per page with actionable `fix`es (title/description length, H1, focus keyword in title/H1/first paragraph/URL, word count, image alt, internal links in/out, canonical, indexability, LCP image weight); B18xx diagnostics; fails `doctor` and `build` when a page scores below `seo.minScore` (default 80, Chris 2026-10-13; `brain.json`) |
| Focus keyword | manual field | `<meta seo keyword="…">`; density and placement checked; cannibalization (two pages on the same keyword) reported |
| Structured data | plugin per type | JSON-LD from `jsonld`/`schema`, plus one automatic `@graph` per page (with `site`): `WebSite`+`SearchAction` (`<site>/#website`) and `Organization` (`<site>/#organization`) on every page, `WebPage` (`<url>#webpage`: name, description, `isPartOf`, `about`/`publisher`, `inLanguage`, `dateModified` from `lastmod` or the entry date, `primaryImageOfPage` from `og:image`, `breadcrumb`), typed `["WebPage","FAQPage"]` with `mainEntity` when `<details>` groups are marked `faq`, `BreadcrumbList` (`<url>#breadcrumb`) from the route tree and `BlogPosting` (`<url>#article`, `isPartOf`/`mainEntityOfPage` → the WebPage) for content collections. Every `@id` reference names a node of the same graph; an author-declared type replaces its automatic node (a declared page type such as `AboutPage` replaces `WebPage`). Without `site` only the self-contained `FAQPage`/`BlogPosting` blocks are emitted. Validated against required properties per type, including Google's nested rules (Search Central, fetched 2026-10-31): Product/SoftwareApplication offers need `price` (AggregateOffer: `lowPrice` + `priceCurrency`), SoftwareApplication needs `name` and a rating or review, JobPosting `jobLocation.address.addressCountry` and the `employmentType`/`unitText` enums, Event Place locations an `address`. Article, BlogPosting and NewsArticle have no required property (Google lists only recommended ones), so a missing `headline` is info, like `image`, `datePublished` and `author` |
| Social images | manual upload | `views/_og.brain` template rendered per page to PNG at build (headless Chrome via the CDP client) with title/description; cached by content hash |
| Links | plugin, partial | broken internal links and anchors fail the build; external links checked on `brain seo links --external`; orphan pages and internal-link suggestions by shared keywords |
| Redirects | plugin + DB | `redirects.json` (`from`, `to`, `301|302|307|308`, `:param`, trailing `*`) emitted into nginx/_headers/Caddy/server.mjs; renamed routes detected from the journal propose a redirect automatically. One hop (2026-10-31, `redirects::resolve`): server.mjs `canonicalOf` and the Rust static server (`prod.rs`, reading `_redirects` and its `# brain trailingSlash=… host=…` line) apply the rule, then `seo.trailingSlash` (`never` default: `/a/` → `/a`; `always`; `ignore`) on its target, then `seo.canonicalHost` (other non-loopback `Host` → `site`), as one response; the build names pages in the form the policy serves (`Policy::path_form`, 2026-11-02): sitemap `<loc>`, hreflang, JSON-LD `url`/`@id`, and a page's own canonical (a literal `canonical="/a/"` becomes `/a` under `never`/`ignore`, `/a/` under `always`; the root and file-like paths keep their form); the query is kept unless the rule's target has one. Guard redirects and 403 answers carry `X-Robots-Tag: noindex, nofollow`. `gone()` in `load` compiles to `{ notFound: true, gone: true, status: 410 }` (410 page, else the 404 page; dev server too). Streamed routes wait `STREAM_WAIT` (50 ms) for `load()` before the 200 head, so a quick notFound/gone/redirect keeps its status; a later redirect becomes `location.replace` behind `<meta name="robots" content="noindex">` |
| i18n | plugin | `hreflang` + `x-default` from `i18n/` languages and localized routes |
| Content | posts in a DB | content collections: `content/blog/*.md` with frontmatter (title, description, date, tags, cover) → typed pages through a `views/blog/[slug].brain` template; tag/category pages, pagination with `rel` links, reading time |
| Feeds & sitemaps | plugin | RSS 2.0 + Atom per collection, sitemap index + image sitemap, robots.txt, llms.txt, llms-full.txt (§32) |
| Freshness | plugin | IndexNow ping on deploy (opt-in, key in env; a sitemap index is read through its local `sitemap-N.xml` children, deduplicated, 10 000 URLs per request; `<loc>` entities decoded; an empty index or a missing, nested, off-site or truncated child fails with a fix, 2026-10-31); `lastmod` from content dates |
| Preview | plugin box | `brain seo preview <route>`: Google result snippet (title truncation by pixel width), social cards, JSON-LD — text for the LLM, HTML in devtools |
| AI answers (GEO) | — | info hints that never change the score: B1830 answer first (opening paragraph ≤ 300 chars), B1831 question headings not grouped as `faq` (the FAQPage markup is for answer engines and Bing: Google stopped showing FAQ rich results on 2026-05-07), B1832 visible `<time>` + `dateModified` on articles, B1833 author as Person with `url`/`sameAs` (`authors` in brain.json fills it), B1834 long content without external sources |
| Video & news | — | `VideoObject` + video sitemap from `<video poster>` (duration from `data-duration` or the MP4 header), `sitemap-news.xml` from `seo.news` collections; B1835 video without poster/date |
| Staging safety | manual | non-production builds (`--env staging`) force `noindex` + robots `Disallow: /` |
| Performance as SEO | external tools | Core Web Vitals lab metrics from the runaway probe (§30) + LCP image size per page in the audit |
- Codes assigned for §30–§31 (#92, #93): B0905 unbounded append, B0906 timer/listener/subscription without cleanup,
  B0213 unsafe static URL, B0215 (info) `rel="noopener noreferrer"` added, B0122 import by URL, B0123 third-party
  script without `integrity` (`<script src>` is accepted in `app.brain`'s head), B0710 external redirect, B0711 (warning)
  SSRF-prone server fetch, B0713 session cookie written through `document.cookie`, B1310 (warning) large literal list
  rendered without virtualization, B1311 (warning) `RegExp` from an unescaped value.

**Social and AI visibility** (Chris to SEO 2026-10-13: "Facebook, LinkedIn, Instagram metadata and modern techniques
to appear in LLMs, traditional search and social networks"; implemented in brain-seo):
- **Social cards:** Open Graph is what Facebook, LinkedIn, WhatsApp, Slack, Discord, iMessage and Instagram DMs read.
  The build fills what is missing and never overwrites the author's tags: `og:site_name`, `og:locale` (`es-MX` →
  `es_MX`), `og:image:alt`, `og:image:width/height` read from the image file, `twitter:card` (`summary_large_image`
  with an image), `twitter:site`, `fb:app_id`, `fediverse:creator`, `<link rel="me">` to each profile on the home
  page, and `og:type=article` + `article:published_time/modified_time/author/section/tag` on collection entries.
  `brain.json` `"social": { "x", "facebook", "instagram", "linkedin", "youtube", "tiktok", "github", "threads",
  "mastodon", "facebookAppId" }` (validated against injection); the profiles also feed `Organization.sameAs` in the
  JSON-LD so search engines and LLMs tie the site to its accounts.
- **AI crawlers:** `seo.ai` = `"allow"` (default: be cited by LLMs) · `"search-only"` (blocks training crawlers such as
  GPTBot, ClaudeBot, Google-Extended, Applebot-Extended, CCBot, Bytespider, Meta-ExternalAgent; keeps the search ones:
  OAI-SearchBot, Claude-SearchBot, PerplexityBot) · `"block"`; robots.txt also carries the matching `Content-Signal:`
  line (search, ai-input, ai-train). Tokens verified against the vendors' docs on 2026-10-13; re-check every 30 days.
- **Markdown copies for LLMs** (llmstxt.org convention, extension-replaced form): every indexable page also exists as
  Markdown (`/pricing.md`, `/` → `/index.md`), named from the route so the page file's extension never shows, linked
  with `<link rel="alternate" type="text/markdown">`; `llms.txt` points to them.
  `seo.markdown` (default true). Content negotiation (2026-10-29): `brain start` (server.mjs `serveTwin`, also before an
  SSR route) and the Rust static server (`brain-serve` `prod.rs`) answer the page URL with its twin when `Accept` ranks
  `text/markdown` above `text/html` (equal weight: the one listed first; an explicit type beats `*/*`, so browsers keep
  HTML), and both representations carry `Vary: Accept`. No twin (`seo.markdown=false`, noindex pages) → HTML as before.
  2026-11-01: a twin sends `Link: <url>; rel="canonical"` when its page declares an absolute canonical without a route pattern, never an invented one, and no `noindex`. Google documents this header for non-HTML documents such as PDF, for web search; Markdown is not named, so its effect on Markdown is unverified.
- **Large previews:** indexable pages without their own robots meta get
  `max-image-preview:large, max-snippet:-1, max-video-preview:-1`.
- **IndexNow** (opt-in, existing) also reaches Bing, which feeds Copilot and ChatGPT search.
- **Generated OG images:** default template, `views/_og.brain` overrides it, `seo.og` = `"missing"` | `"always"` |
  `"off"`, cached by hash in `.brain/cache/og/`, rendered with TOOLCHAIN's screenshot closure (#126 → SEO #127).
  `"missing"` (the default) renders nothing while brain.json has no `site`: the card's og:image could only be relative,
  which social networks ignore (B1827), and the browser launch cost 2–4 s on every cold build (#282).

### Site focus, live probes and the URL register (2026-11-01)

`seo.focus` (B1861-B1866, `focus.rs`) is an editorial consistency report, deliberately not a score input and with no
wording about positions: Google documents crawlable links, descriptive anchors and keyword stuffing as spam, and documents
no such focus scheme. Its numbers (a limit of 3 terms, "nearly every title") are heuristics, so over-limit and overuse only
warn. Matching folds case and marks for Latin-1, Extended-A and combining marks; other scripts match as written.

`checks.rs` holds rules with a documented basis only (B1871-B1878). URL depth, word minimums, intro position, byte position
and a unique H1 as a rule stay out: nothing documents them. B1872 is an error although new diagnostics start as warnings:
Google states that later pages of a listing must not declare page 1 as canonical, and the request was explicit. B1876 uses
Googlebot's documented 2 MB fetch limit. B1878 joins internal search views and thin tag listings to stay in range.

`seo_live.rs` no longer follows redirects, so a 301 in the sitemap stays visible; only the sitemap files are followed by
hand. `Probes` separates what was requested from the checks, which are fed responses in tests. One page stands for the
variants and only the first page's assets are requested, so the probe costs a handful of requests, each after `--delay-ms`.
The client has no decoder, so `Accept-Encoding` is sent only to responses judged by their headers. Focus and B1863-B1865
need every page and are dropped when the audit is a sample.

`seo-urls.txt` (B1879) exists because B1826 compares with the previous `dist/`, which a clean clone or CI does not have. It
is opt-in by the file's existence. The build only reads it (generators never write to the source tree without being asked);
`brain seo urls --write` only adds, sorted, so the diff shows what was published.

## 34. Production process manager: `brain start` (fixed 2026-10-13, Chris: our own manager, not PM2)

Lifecycle CLI targets (2026-11-04): stop/reload/status accept one optional directory, canonicalize explicit paths
and resolve the nearest app before connecting. Missing, non-directory and non-app targets fail without cwd fallback.
The existing per-app socket and deep-path hash fallback are unchanged; no global process search is introduced.

Successful `brain reload` control replies carry optional freshness advice. A read-only timestamp scan compares source
files to the newest regular output file, bounded to 20,000 entries and 32 levels per tree, skipping symlink traversal,
generated/cache folders and test-only inputs. Newer source or incomplete coverage is advisory; reload never compiles
or mutates dist. Timestamps are not a source-hash attestation; environment/provider configuration is not probed.

The `brain` binary runs the app in production. Optional PM2/nginx configuration can be prepared with `brain deploy config` (maintainer request, 2026-10-30; PM2 files pass `BRAIN_ENV_ONLY=file` so `main::apply_env_policy` drops the daemon's environment before any thread exists, and the preview carries a `recipe` that recreates processes, 2026-11-01; `brain deploy check --url`, `crates/brain-cli/src/deploy_check.rs`, reads only `/` and `/health`: B1857, B1891 early hints behind nginx below 1.29, B1892 hop header without `trustPublicProxy`, B1893 health not 200, 2026-11-01); it never installs or activates services. The native manager remains the default:
one way for the model, `brain build` then `brain start`.

Native nginx previews route the exact `/_brain/health`, `/_brain/health/live` and `/_brain/health/ready` aliases to the frontend gateway,
ahead of the general `/_brain/` backend prefix. The gateway owns alias mapping and app-page precedence; ordinary
native API/auth traffic keeps its backend upstream. Existing deployment files remain preserved for explicit review.
Rust production and dev gateways use the same exact alias map to `/health`, `/health/live` and `/health/ready` before
native forwarding. Production keeps the base namespace/root probe behavior and existing `/health` app-file precedence;
GET/HEAD preserve upstream status and no-store responses, with a healthy JSON answer when no backend is configured.

Static gateway rebuilds (2026-11-04): `StaticGateway` keeps an Arc policy snapshot under a short mutex. Metadata
identity/mtime/length for the output directory, index/not-found pages and host-policy files triggers refresh of cached
headers, redirects and build validators. Requests clone the snapshot and release the lock before I/O. Recreating
dist/ therefore recovers without restart and a previous HTML ETag cannot incorrectly return 304 for the new build.
This does not turn a build into an atomic deployment or hot-reload Node SSR processes; those still use brain reload.

- **Static apps** (no `load`, no `server:`): `brain start` serves `dist/` from Rust with brain-serve's static handler
  (precompressed files, clean URLs, immutable `/_b/`, §31 headers). No Node.
- **Apps with a server**: the supervisor binds the port once (`--port`/`PORT`, default 3000; `--host`/`HOST`, default
  127.0.0.1) and starts `--instances N` (default 1) children `node dist/server.mjs` with the environment resolved from `.brain_env` (§36), each
  inheriting the listening socket as fd 3 (`BRAIN_LISTEN_FD=3`). The child listens on that fd (`listen({ fd })`) and
  prints `brain:ready` on stdout once it accepts and has imported every module a request can need (each route's server
  module, view, layouts, guards, middleware and dictionary, the RPC modules, the DOM shim; at most 15 s, then it
  continues while serving), so the first request after a start or reload does not pay for the app's module graph. A
  module that throws at import does not stop the start: its route answers as it would have. All children accept from the same socket, which is also how several
  instances share one port. `SO_REUSEPORT` is not used: Node returns ENOTSUP on macOS (measured on 22.21).
- **Zero-downtime reload**: `brain reload` (or SIGHUP to the supervisor) starts new children on the same socket, waits
  for `brain:ready` (30 s), then sends SIGTERM to the old ones. On SIGTERM a child drains: it stops accepting, finishes
  in-flight requests for up to 10 s and exits 0. If a new child never gets ready, the old ones keep serving and the
  reload exits 1 with the new child's log. Handoff verified on macOS 2026-10-13 (Python parent + two Node children on
  one inherited socket): the old child exited 0 and the next request was served by the new one.
- **Restart policy**: a child that exits is restarted with backoff (100 ms, doubling, max 30 s; reset after 60 s
  alive). Crash loop (10 restarts in 60 s) → stop, print the last 50 log lines and exit 1, so the agent or the init
  system sees it. Liveness: `GET /_health` every 10 s; 3 failures in a row → restart. Memory: child RSS sampled every
  5 s; above `--max-memory` (default 300M) → graceful restart (B0908 stops runaways in the browser; this covers the
  server).
- **Logs**: the children's stdout/stderr (JSON lines with `rid`, §31) plus the supervisor's own events (start, ready,
  exit code, restart reason) go to `logs/app.log`, rotated at 10 MB or daily, 7 files kept; also to the terminal when
  attached.
- **Control**: `brain stop` (graceful) · `brain reload` · `brain status [--json]` (pid, uptime, instances, restarts, RSS,
  last exit code and reason), through the control socket `.brain/run/brain.sock` and `.brain/run/brain.pid`. MCP exposes
  `brain_status` read-only; start, stop and reload stay CLI-only so an agent cannot stop production by mistake.
- **Boot**: `brain start --install` writes a systemd unit (Linux, `Restart=always`, runs `brain start`) or a launchd
  plist (macOS) and prints the one command that enables it; it never runs sudo. Windows is out of scope.
- **Exit codes**: 0 stopped cleanly · 1 crash loop or never ready · 2 usage · 3 internal error.
- **Dependencies**: std, plus `libc` for fd inheritance and signals if std is not enough (low level, allowed by the
  dependency rule). No third-party supervisor library.
- **Owners**: TOOLCHAIN #119 (supervisor and CLI/MCP; static mode) · RUNTIME #118 (`server.mjs` listens on
  `BRAIN_LISTEN_FD` when set, else PORT/HOST; prints `brain:ready`; drains on SIGTERM; `ecosystem.config.cjs` removed) ·
  QA #120 (kill -9 → back in < 1 s; memory over the limit → restart; reload under load with 0 failed requests; crash
  loop → exit 1 with log; `--install` output reviewed; static app without Node).
- A host that already has its own process manager runs `brain start` as one more command; the framework emits no
  config for it.

### Provider profile hints (2026-11-04)

Migration 0027 adds bounded `display_name` and `picture_url` to users, backfilling existing identity names in a
stable provider/subject order. Verified provider adapters normalize optional name/picture hints before identity login;
Apple's unsigned first-authorization name remains display-only. `Auth::finish_identity` updates hints in the same
transaction as session issuance without changing linking or grants. Omitted claims retain stored hints. User reads
for password/opaque/JWT/refresh/API-key authentication select the same columns; JWT claims remain unchanged.
`server.mjs` native `safeUser` admits only bounded plain name and credential-free HTTPS picture hints alongside its
existing authority fields. OpenAPI marks both optional. No image fetch or provider access-token storage is added.

### Session secrets, rotation and revocation (2026-10-31)

`sessionSecrets` (server.mjs) adds `setSecret`/`secret`/`clearSecret` to any session handle, signed or native: AES-256-GCM in `__Host-session-secret`, key from HKDF over `BRAIN_SECRET`, the cookie name as AAD, expiry inside the ciphertext, 3.5 KB limit. It never fills `ctx.user`. `BRAIN_SECRET_PREVIOUS` still verifies and decrypts; new cookies use the current key. `BRAIN_SESSION_VERSION` travels in the signed cookie, and a mismatch reads as anonymous. Logout remembers the MAC until expiry in one process only, because no shared store exists here; several processes revoke through the version.

### Refresh grace, client lockout, operator actions and internal routes (2026-10-31)

- Refresh grace (`jwt.rs`, migration 0016 `grace_uses`): a retired token within `BRAIN_AUTH_REFRESH_GRACE_SECONDS` (0..60, default 10), up to 8 uses, with a successor never used, answers an access token and an empty `refresh_token`; `jwt_response` then sets no refresh cookie. The bridge `renew()` in `prepare` renews during SSR and rewrites `req.headers.cookie`; 401/403 is anonymous, anything else 503.
- Lockout (`auth.rs` `login_from`, `login_attempt`): pair key 10, email key 100; success clears only the pair. Reset (`request_reset_from`, `flow_budget`): pair 3, email 10. `None` client keeps the previous per-email budget. The client is `ClientIp`, so without `trustProxy` plus forwarded headers all proxied visitors share one pair.
- Operator actions (`auth_flows.rs` `set_user_disabled`, `revoke_user_sessions`, `set_identity_disabled`) are static functions, shared by routes and `brain backend users`; `set_user_disabled` takes the role-change advisory lock for the last-admin guard. `_brain_identities.disabled_at` is read in the `existing` query of `google_login`, so a disabled identity returns before link or create.
- `Access::Internal` (`routes.rs`): `InternalAuth` keeps HMAC tags under a per-process key; `Debug` shows only the count. `signed()` in `routes_http.rs` exempts it from the Origin check. The site proxy has a header allowlist that omits the secret header on purpose.
- B0751 (`backend.rs` `config_problems`): JWT, Google and session length, while `startup_config.rs` owns mail, storage, database and pools (B0760–B0764); single source for startup, `brain check` and `brain doctor`, runs before the database connection and reports names, never values. Check and doctor read the development section only, and only when the app has `server/resources`, `server/jobs`, `server/routes` or `BRAIN_BACKEND_URL`. Not covered, because each validates files or needs the listener: `socket_settings`, `api_key_config`, `hooks_executor`, `platform_config`, route contracts, the database URL and the connection budget. A route whose internal or webhook secret is unset passes `brain check` and stops startup with the variable name and no code.
- `ctx.backend.google` (server.mjs `googleStep`): the bridge reads the nonce cookie and sends it alone; the dev sidecar gets it through `nativeBridge.backend`.

### 34.1 Answers with a cause (2026-11-01)

- **Every 5xx leaves a line.** `server.mjs` registers each request in `liveRequests` and logs `server error answered` (rid, path, status, cause) from `res.once('close')`, whichever path answered: `errorPage`, `failPage`, a relayed backend 5xx, the health probe, sign-out, a stream failure. `trace()` adds `cause` only for the exact messages in `OWN_CAUSES`, plus `transport` (the system code such as `ECONNREFUSED`) and `backendCode`: an application message can carry values, so it stays out. A status page opened directly logs at info level, at most 16 per minute.
- **A swallowed read is visible.** A native read that fails during a page render marks the request (`$brainFailedReads`). A page that still renders logs `page rendered after a failed backend read` with its `fix`, and answers `cache-control: no-store`, so the page cache never keeps it. `bucketClient` goes through `fetch` and is not covered.
- **A read refused for now is a 503 (2026-11-01).** A read answered 429 or 503, unreachable, timed out or refused by a busy bridge is `temporary`: a page whose `load` caught it answers 503 with the backend's `Retry-After` through `_error.brain`, not a 200 without its data; the log keeps the warning with its `backendCode`. There is no opt-out: a caught 500 still renders. A streamed page past its head stays 200 and adds `noindex`.
- **Cold `<meta cache>` is single-flight (2026-11-01).** Requests for one key wait for the first render. Only a cacheable result is shared (a private one holds its visitor's cookie); the rest render on their own. HEAD uses the same path, so it never runs `load` for a stored page. A cold render that answers something other than a 200 (an error page, a redirect, a not-found) is replayed to the request that started it, so `load` and its backend reads run once, as on a route without `<meta cache>` (2026-11-02; before, it rendered a second time).
- **Reads revalidate (2026-11-01).** A `ctx.backend` read made with no cookie and no key remembers its body by path when the backend sent an `ETag` (256 entries, 16 MB, 1 MB each) and sends `If-None-Match`; a 304 parses the kept body afresh. The backend still runs the query, so this saves transfer and parsing, not database time, and spends the same limit token. Signed-in and service-key reads are never kept.
- **One URL per page.** `urlVariant` answers one 301 for `//route`, `/a//b` and `/route/index.html` (GET and HEAD, backend paths exempt). `prod.rs` does the same for `/a//b` and `/route/index.html`; its parser answers 400 to a leading `//`, which is also one answer and no duplicate page. `/route.html` is not redirected: a verification file may legitimately be HTML at the root.
- **Log lines.** `brain_backend::log_line!` formats first and writes once. A pipe write is atomic only up to `PIPE_BUF` (512 bytes on macOS, 4096 on Linux); a longer line (a stack event, up to 32 KB) is one call but can still interleave on a pipe shared by several processes. `crates/brain-backend/tests/log_lines.rs` proves short lines and forbids the standard print macros in the crate.

## 35. Caching and the repeat visit: on by default (fixed 2026-10-13, Chris: "the second visit must be faster")
Cache audit (2026-10-28): anonymous SSR caching bypasses every cookie-bearing request and Authorization. Captured
session headers are preserved; Set-Cookie, Vary and private/no-store/no-cache responses never enter the shared map.
SSR retention is bounded to 500 entries, 1 MiB per HTML entry and 32 MiB total serialized HTML. A private response
is returned once with no-store instead of running its loader again just to set the session. Cold concurrent SSR
requests can still render independently; private responses must never be shared through an anonymous-flight cache.

Client HTTP retention uses resolved API URLs, bypasses dynamic identity headers and visible cookies, respects
private/no-store, and prevents late GETs from restoring data after a write. The in-memory map retains at most 256
entries / 8 MiB serialized JSON / 512 KiB per entry. Persistent opt-in data keeps at most 128 entries, with serialized
storage operations, expiration deletion and invalidation after writes; saturation coalesces to one cache purge.
HttpOnly cookie identity is not visible to JavaScript: authenticated APIs must return private/no-store and personal
data must never opt into persistent caching, as SPEC already requires. These are retention budgets, not total JS
heap caps. Existing service-worker/assets and detailed cache contracts follow.


Every app gets the full caching stack from `brain build` with no code from the author. The second visit should
download no unchanged JS or CSS, render from cache, and work offline for routes already visited. Each layer can be
turned off in `brain.json`:
`"cache": { "http": true, "sw": true, "offline": true, "precacheKB": 2048, "data": true }`.
What each switch turns off, exactly: `http: false` → no cache headers and no hashing of `public/` · `sw: false` →
the self-unregistering worker (below) and no registration snippet · `offline: false` → the worker still precaches and
serves `/_b/*`, but page navigations become network-only: no cached pages, no offline page, `views/_offline.brain`
is not built · `data: false` → `resource()` keeps no memory cache and every `cache:` option in `api()` is ignored
(B1102 warns that it has no effect) · `precacheKB: 0` → nothing is precached on install.
`brain run` (dev) never registers a service worker and unregisters any it finds on its origin, so stale code is
impossible while developing.

**1. HTTP caching** (TOOLCHAIN: `brain start` static, nginx snippet, `_headers` for static hosts; RUNTIME: server.mjs)
- Static files in both production servers (2026-11-04): full GET and HEAD carry Content-Length and a SHA-256 strong ETag;
  a byte range carries the same identity validator and an exact Content-Range and Content-Length. If-Range accepts
  only an exact strong identity ETag; weak, stale, encoded and date validators get the full response. Conditional
  If-None-Match is evaluated before Range. Precompressed siblings have their own content validator and length.
  Rust omits Content-Length on 304, for both static HTML and assets, rather than declaring an empty representation.
  Hashing uses a 64 KiB buffer and a 256-entry metadata cache, with a 30-second cold-hash deadline. Node reads
  asynchronously, coalesces identical hashes and admits at most eight cold hashes with no queue; saturation answers
  503. Rust hashes in its existing connection worker. Only the negotiated representation is hashed. Stat, hashing
  and streaming keep one descriptor, so an atomic replacement cannot combine old headers with new bytes.
  Transmission uses backpressure and closes on client disconnect. No throughput improvement is claimed.
- Streaming playlists/manifests (`.m3u8`, `.m3u`, `.mpd`, case insensitive) keep their original public paths in both
  HTML/CSS and script references, so relative segments, encryption keys and nested playlists remain resolvable.
  Segments retain their original public path and may also have an immutable copy. The Node and Rust MIME tables
  cover HLS playlists, fragmented MP4 segments, DASH manifests, MPEG transport streams and WebVTT captions.
  Both production servers revalidate these unhashed manifests (`no-cache`) instead of retaining a stale playlist.
- `/_b/*` (content-hashed JS, CSS, fonts, images): `public, max-age=31536000, immutable`.
- Relocatable assets in `public/` that HTML or CSS reference are copied into `/_b/` with a hash and their references
  rewritten, so they become immutable too. Static-host rules revalidate unhashed public files; Node revalidates
  streaming manifests and uses `public, max-age=3600` for other public assets.
- HTML: `no-cache` plus a strong `ETag` (the build's content hash) → a repeat visit gets `304` with no body. No
  `no-store` on public pages (it disables bfcache, §19). Anonymous SSR HTML: `no-cache` and a weak ETag of the
  rendered body. SSR HTML rendered with a session cookie (valid or stale): `private, no-store` and no ETag, so Back
  after logout cannot show the previous user's page from the HTTP cache or bfcache (2026-10-29).
- `/_r/` RPC and anything personal: `no-store`. Logout (`ctx.session.clear`) sends
  `Clear-Site-Data: "storage"` so the next user on the device sees nothing of the previous one (not `"cache"`:
  Chrome clears the whole HTTP cache before finishing that response, which held logout ~18 s; F16, 2026-10-29).
  Nothing private needs that purge: RPC and session-bearing SSR HTML are never stored.
- Back after logout is verified only in Chrome (2026-10-29: the page is re-fetched and the guard redirects);
  Safari and Firefox are pending. Possible follow-up, not implemented: a `pageshow` (`persisted`) reload on session
  pages, for a browser that still restores one from bfcache.
- Precompressed `.gz` and `.br` for every text asset when smaller than the file, plus `.zst` (zstd level 19) only when
  it beats Brotli; `Vary: Accept-Encoding`. Servers send the smallest sibling the client accepts (2026-10-31).
- `dist/server.mjs` compresses per-response bodies (pages carry a per-response nonce): zstd on ties when `node:zlib`
  has it, then Brotli q5, then gzip; ≥ 1 KiB, never `HEAD`/204/304/`no-transform`; streamed SSR flushes per chunk.
  `compress: false` / `BRAIN_COMPRESS=off` turns it off (docs/PERFORMANCE.md, 2026-10-31).

Document selection and aliases (2026-11-05): generated host headers include URL-encoded physical document aliases,
including internal error pages. Rust static fallback selects document headers from the actual file, with legacy
canonical rules accepted only when they resolve to that same file; missing assets served as HTML errors do not
inherit immutable asset caching. Literal encoded stars remain exact patterns, not wildcard policy grants. Node
recognizes `.brain` documents and `.jsbrain`/`.cssbrain` assets alongside ordinary extensions, including Markdown
canonical lookup. Decoded error aliases keep their HTTP status. Production host error routing still requires the
host configuration described in the deployment guide.

Own script execution: the Node production resolver gives `module` format only to compiler-owned `.jsbrain` URLs
under the build's `_b/` directory. Direct file URLs, mounted root imports and relative chunk imports share the same
resolver in synchronous `registerHooks` and asynchronous `register`; outside-build files and ordinary Node imports
retain their normal loader. Real HTTP regressions cover SSR, streaming, dictionaries and transitive chunks under
root and base paths with each hook implementation.

Generated SEO facts (2026-11-05): finish passes the configured locale home to breadcrumb generation, avoiding a
default-home/localized-home duplicate while retaining global WebSite and Organization identities. Blank optional
strings are absent in runtime SEO overrides and generated schema/article facts; explicit author JSON-LD is untouched.
Social image URLs share app-base and canonical-page semantics across prerender and SSR, including already-prefixed
paths, relative paths, protocol-relative URLs and existing percent escapes. Rust packaging and SEO share canonical
slash formatting; Rust and Node image resolution use the same regression vectors. Generated SSR WebPage graph identities include the mounted base even when there is no SEO callback; primary image
objects follow the request's normalized social image. Explicit JSON-LD remains authoritative, and authored Twitter
name/property overrides replace generated counterparts without duplicate tags. Literal SEO callbacks are not
evidence of load dependency: the compiler already streams only static SEO, and a held-load real build regression
requires the head to arrive before load is released. No unconditional SEO wait was added.

Nonce metadata and cache updates: Node HTML HEAD and 304 omit a newly generated CSP nonce policy while retaining
stable ETag, opener and cache metadata. RFC 9111 sections 4.3.5 and 3.2 allow a matching HEAD to update a stored GET's
headers without replacing its body; sending a new nonce would deny that body's scripts after later revalidation.
The regression models those cache update rules against real GET/HEAD/304 responses; it does not claim a measured
failure in a particular browser or CDN. GET responses retain fresh per-response script nonces.

**2. Service worker** (RUNTIME writes `runtime/sw/brain-sw.js`; TOOLCHAIN emits `dist/sw.js` with the build manifest)
- Precache on install: the runtime core, the app shell, every route's entry chunk and CSS, up to `precacheKB`
  (default 2048 KB); the rest is cached on first use. At most 4 downloads run at once (a burst of hundreds tripped
  per-IP proxy rate limits after a deploy), and an update copies every `_b/` file the previous build of the same app
  already cached instead of downloading it again: the URL is content-hashed, so only what changed reaches the network.
- Strategies: `/_b/*` cache-first (immutable) · page navigations network-first with a 3 s timeout, then the cached
  page, then the offline page (`views/_offline.brain` if present, else a minimal built-in one) · unhashed `public/`
  assets stale-while-revalidate · `/_r/`, non-GET requests and other origins are never touched.
- Byte-range requests and streaming manifests (`.m3u8`, `.m3u`, `.mpd`, case insensitive) bypass every service-worker
  cache strategy (2026-11-04), including explicit data routes. The network handles validators and ranges; a cached
  complete response cannot satisfy a partial request, and a live playlist must not trail one revalidation behind.
- Navigation Preload is enabled, so the network request starts while the worker boots.
- Updates: the worker file carries the build hash and is served with `no-cache`. A new worker activates at once
  (`skipWaiting` + `clients.claim`; safe because assets are hashed and HTML is network-first) and keeps the previous
  build's cache for one generation, so a page that is still open can lazy-load its old chunks. Older caches are
  deleted.
- Registration: a short inline script added by the build to every page, run after `load` in idle time. It is not
  part of the core runtime (the 4 KB budget stays). Only on HTTPS or localhost.
- Opt-out that reaches existing visitors: with `"sw": false` the build emits a `sw.js` that deletes its caches and
  unregisters itself, so turning it off really turns it off.

**3. Data caching** (RUNTIME, `resource()` and `load`)
- In memory by default: GET resources are cached per URL for the session, shown at once on back/forward and
  revalidated in the background (stale-while-revalidate); a newer response replaces the old one (§31 races).
- Persistent by opt-in per endpoint, never by default for personal data:
  `api('/posts', { get: { out: '[Post]', cache: '10m' } })` keeps the response in the Cache API for 10 minutes and
  serves it on the next visit while it revalidates. Cleared by `Clear-Site-Data` on logout.

**4. Local storage** (RUNTIME, module `brain-persist.js`, loaded only when used)
- `store(init, { persist: true })` saves the store to IndexedDB (localStorage below 4 KB), restores it before the
  first render, and versions it by the shape of `init`: a changed shape discards the old data instead of crashing.
  Quota and private-mode errors fall back to memory and are reported once, never thrown.

**5. Web Workers** (COMPILER + RUNTIME)
- A module named `*.worker.jsbrain` (or `*.worker.js`) runs in a dedicated Web Worker; importing it gives the same
  exports as async functions over a tiny message bridge (`brain-worker.js`, structured clone, no third-party
  library). Use it for parsing, sorting or computing large data off the main thread. DOM access inside it is an
  error at compile time. The service worker is the framework's own use of workers.

**6. Loading extras that also help the first visit** (mostly in place: §13, §19, §32)
- Exact `modulepreload` per view, critical CSS inline, `103 Early Hints`, the router's predictive prefetch of the
  next route, bfcache-safe pages (B0211), `<link rel="preconnect">` for every third-party origin in the CSP,
  `font-display: swap`, `fetchpriority="high"` on the LCP image,
  lazy images with dimensions (B1821).

**7. Build signature: a new build always replaces the old one** (Chris 2026-10-13: "every build must come out with a
different signature so nothing old stays cached by mistake")
- Every build gets a signature: a hash of every file in `dist/` (paths and bytes). Any change anywhere gives a new
  signature; an identical rebuild gives the same one, and then there is nothing old to clear. It is not a timestamp:
  that would break invariant 8 (deterministic builds) and make every visitor download unchanged files again after
  each deploy.
- Where it goes: `<meta name="brain-build" content="<sig>">` in every page, the `X-Brain-Build` header on every HTML
  response, the strong `ETag` of static HTML, `self.BRAIN_BUILD.hash` in `sw.js`, the service worker's cache names
  (`brain-<sig>`), `/_health` (`{"ok":true,"build":"<sig>"}`) and `brain status`. An agent checks a deploy by
  comparing the signature `brain build` printed with the one `/_health` returns.
- Nothing can stay stale:
  - HTML, `sw.js` and streaming manifests are revalidated on every request; static-host rules also revalidate other
    unhashed public files (Node retains those for up to one hour). Content-hashed files get a new URL when changed.
  - The registration uses `updateViaCache: 'none'`, so the browser never serves `sw.js` or its imports from the HTTP
    cache; the page also calls `registration.update()` on every full load.
  - A new service worker deletes every cache except its own and the previous build's (kept one generation only for
    chunks an open tab may still lazy-load; it never answers a navigation).
  - Open tabs learn about a new build: the worker posts `brain:build <sig>` to its clients, and the router compares
    the `X-Brain-Build` of each navigation with the page's own; when they differ, the next navigation is a full page
    load, so the tab moves to the new build instead of mixing old and new code.
  - A chunk that fails to load (deleted by a deploy) triggers one full reload, guarded so it never loops.
- `brain build` prints the signature as its last line (`build <sig>`), and `--json` returns it.

**Verification** (QA): a bench that loads each example app cold and then again (CDP, mobile throttling) and reports
transferred bytes, requests and LCP for both visits. It also deploys a second build while a tab is open and checks
that the next navigation shows the new build, no request returns an old file and an identical rebuild keeps the signature. Targets: the repeat visit transfers 0 bytes of unchanged JS/CSS
(only HTML revalidation or nothing), its LCP is lower than the cold one, visited routes open offline, and a deploy
while a tab is open never breaks that tab (old chunks still load). Each opt-out switch is tested to really disable
its layer.

**Owners**: TOOLCHAIN #121 (headers, ETag, hashing of `public/`, `dist/sw.js` with the precache manifest,
registration snippet, `_headers` and nginx snippet, dev unregister, `brain.json` `cache`) · RUNTIME #122
(`brain-sw.js`, server.mjs headers and `Clear-Site-Data`, `resource()` memory cache and `cache:` option,
`brain-persist.js`, `brain-worker.js`) · COMPILER #123 (`*.worker.*` modules: worker chunk, async exports, DOM-in-worker
error; `cache:` accepted in `api()` schemas) · QA #124 (repeat-visit bench and the tests above).

### 35.8 Declarative data routes and user-controlled updates (2026-10-31)
`brain.json` `cache.routes` lists same-origin data paths for the service worker: `match` (`/api/products/*`, trailing
`*` = prefix, never `/_b/`, `/_r/` or `/sw.js`), `strategy` (`cache-first`, `network-first`, `stale-while-revalidate`,
`network-only`), `maxEntries` (1-1000, default 64), `maxAge` (`30s`…`1y`, default none) and `timeout` (network-first,
1-60 s, default 3 s). The first match wins; unlisted data stays untouched as before. The build validates entries
(`cache::sw_route`) and writes them, base path included, into `self.BRAIN_BUILD.routes` only when declared, so a
worker without routes is byte-identical. Each route owns a `brain:route:<match>` cache that survives builds and is
deleted on activate when the route is removed. Entries carry `x-brain-at`; older than `maxAge` they are never served.
Eviction is oldest-first by insertion. Only `200` responses that are not `private`/`no-store`, carry no `Set-Cookie`,
do not vary by `Cookie`/`Authorization`/`*` and whose request has no `Authorization` header are stored. Same-origin
cookies are invisible to the worker: personal endpoints must be `network-only` (or return `private`).
`cache.update: "prompt"` keeps a new worker in `waiting` while an older one controls tabs (the first install still
activates at once). `updateReady()` / `applyUpdate()` (`brain-update.js`, auto-imported only by views that use them,
0 bytes elsewhere) expose the waiting worker as a signal and post `brain:skip-waiting`; the page reloads once on
`controllerchange`. Tests: `runtime/test/sw-routes.test.js`, `runtime/test/update.test.js`,
`brain-pack` `worker_routes_and_update_prompt`, `brain-cli/tests/sw_routes.rs`.

### 35.1 The page cache key (2026-11-01)

`cacheSearch(url)` drops tracking parameters (`TRACKING`, the same list as `brain-seo`) and sorts the rest, so `?utm_source=x` shares the entry of the bare URL. A `load` that echoes such a parameter shares that copy too. Answers from the cache send `stale-if-error=<n>`, and a failed revalidation keeps the copy until `2n` rather than dropping it. The render captured for the cache now has the request surface a native read needs (`once`, `removeListener`, `getHeader`): before, a `<meta cache>` page reading `ctx.backend` rendered its fallback or was never stored.

## 36. Environment file `.brain_env` (fixed 2026-10-13, Chris: "loaded by default, understood by the compiler")

One file at the project root holds every variable for every environment. It is loaded by default; there is no other
way to configure variables (a `.env` is ignored and `brain doctor` offers to convert it).

```
# shared by every environment
PUBLIC_SITE_NAME=Nimbus
API_TIMEOUT_MS=8000

[development]
PUBLIC_API_URL=http://localhost:5180/api
DATABASE_URL=postgres://localhost/nimbus_dev

[staging]
PUBLIC_API_URL=https://staging.nimbus.dev/api

[production]
PUBLIC_API_URL=https://nimbus.dev/api
DATABASE_URL=            # empty: must come from the host's real environment
```

- **Syntax:** `KEY=value` lines (dotenv: optional quotes, `#` comments, `${OTHER}` expansion inside the same
  environment), keys `UPPER_SNAKE_CASE`. Lines before any `[section]` are shared; a section overrides them for that
  environment. Section names are free (`[preview]` works with `--env preview`).
- **Which environment:** `brain run` → `development`; `brain build` and `brain start` → `production`; `--env <name>` on
  any of them selects another one. Precedence: the process environment > the section > the shared lines, so a host or
  CI can always override without editing the file. A key whose final value is empty is an error at `brain build` /
  `brain start` (`B0720 variable X has no value in <env>`), so production never boots half-configured.
- **Client vs server:** only `PUBLIC_*` keys reach the browser; `brain build` substitutes them as constants (dead
  branches removed, §13). Every other key exists only on the server (`load`, `server:` functions, `ctx.env.X`). The
  supervisor (`brain start`, §34) passes the resolved environment to `server.mjs`; Node never reads the file itself.
  A `PUBLIC_*` key no environment declares compiles to `undefined` outside server code (every read form: dotted,
  computed, optional, `globalThis.process.env`, `import.meta.env`), so a browser never meets a missing `process`. Server
  code (`load`, `server:`, `<meta server>`) keeps reading the real environment; `import.meta.env.X` there becomes
  `process.env["X"]` (2026-10-31).
- **The compiler understands it** (COMPILER): `brain check` reads the keys (never the values into diagnostics) and
  - B0702 (existing) private key in client code;
  - B0721 key used in code but missing from `.brain_env` (warning, with an edit that adds `KEY=` to the shared block);
  - B0722 key declared but used nowhere (info);
  - B0723 a `PUBLIC_*` value that looks like a secret (key-shaped, §11.1 B0703 patterns) → error: it would ship to every
    visitor;
  - B0724 malformed line or unknown section syntax, with `file:line`.
  `brain map --json` lists the keys per environment (names only) so an agent knows what exists without reading values.
- **Scaffold:** every `brain new` project already contains `.brain_env` with the shared block and the `[development]`,
  `[staging]` and `[production]` sections, prefilled with the keys the template uses (`PUBLIC_SITE_NAME`,
  `PUBLIC_SITE_URL`) and a comment per section; plus `.brain_env.example` (same keys and sections, no values;
  committed) and the gitignore entry. The author never creates the file by hand.
- **Git and safety:** `.brain_env` is gitignored because it may hold secrets. `brain doctor` fails if `.brain_env` is tracked by git, and syncs the example's
  keys when they drift. No MCP tool, log, error or devtools panel ever prints a value (§26 already forbids reading
  `.env`; it now covers `.brain_env*`).
- **Owners:** COMPILER #137 (parser in brain-check shared with the CLI, B0720–B0724, `PUBLIC_*` constants for codegen) ·
  TOOLCHAIN #138 (selection by command and `--env`, precedence, substitution at build, env to the supervisor's child,
  `brain new` files and gitignore, doctor checks, map, MCP redaction, dev reload when the file changes) · RUNTIME #137
  (`server.mjs` takes its environment from the supervisor only; `ctx.env` frozen; no `.env` handling left) · QA #138
  (per-environment builds give different `PUBLIC_*`, process env overrides, empty production key fails, no value in any
  output, the LLM gate with a brief that needs an API URL per environment).

## 37. Languages: one convention for multilingual sites (fixed 2026-10-13, Chris)

A site in several languages is a configuration, not a rewrite. The model writes every view once, with keys; adding a
language is one command.

- **Config:** `brain.json` `"i18n": { "default": "en", "locales": ["en", "es", "pt"] }`. One locale means a
  single-language site with no prefixes (the default for `brain new`).
- **Files:** `i18n/<locale>.json`, flat dotted keys (`"nav.home": "Home"`, `"posts.count": { "one": "{n} post",
  "other": "{n} posts" }`). `brain new` makes a single-language site with no `i18n/` folder and plain text in its
  views; the first extra language (`brain add locale`) moves every fixed text to keys (below), so going from one
  language to two never means editing views by hand.
- **In code:** `t('key', vars?)` in markup and BrainScript; plurals by `Intl.PluralRules`; `fmt.date(d)`,
  `fmt.number(n)`, `fmt.money(n, 'USD')` and `fmt.relative(d)` over `Intl` for the current locale; `locale()` returns
  it. Text written straight into markup is allowed in single-language sites; with 2+ locales it is B1403 (warning,
  with an edit that moves the text to a new key in every locale file).
- Literal builtin `t('key')` calls without placeholder arguments evaluate at component creation without a retained
  translation effect. Locale switches remain full document loads; reactive placeholders and local helpers still
  track. Do not hoist runtime dictionary reads to module initialization across SSR/prerender locale jobs.
- **One language** (2026-10-31): `i18n` with one locale has no prefixes; `brain run` loads its dictionary, sets `lang` and passes it to `load()` and form posts, as `brain build` already did, so `form.*` keys apply in both.
- **Internal links keep the language** (2026-10-31): a page outside the default language has its `<a href>` and `<form action>` that name a page of the app rewritten to that language (`/about` → `/es/about`, or its `<meta route>`): `LocaleLinks` in brain-pack on the written HTML, `route.hrefs` in server.mjs on rendered bodies and on `redirect()`, and `brain-links.js` (loaded only by interactive pages of those languages, ~610 B gz) on what views mount, and on `go()`/`navigate()` through `__brainHref`. `hreflang` opts a link out, which is how `<LangSwitch>` works. Files, other origins and paths with no page stay. The default language and single-language apps ship nothing. `brain run` covers mounted links and navigation but not translated routes, nor the HTML before scripts run.
- **What B1403 moves** (2026-10-31): text nodes; a sentence with interpolations as one message with placeholders
  (`Signed in as {email}` becomes `t('account.signed_in_as_with', { email })`, never two fragments); visible
  attributes (`placeholder`, `title`, `alt`, `aria-label` and the other `aria-*` texts, `label`, `description`,
  `help`, `hint`, `caption`, `heading`, `message`, `empty`, and `value` on submit/button/reset inputs);
  `<meta seo title/description>` (`<scope>.meta_title`, `<scope>.meta_description`); string literals in markup
  expressions (both branches of `a ? 'x' : 'y'`, the right side of `||`/`??`, `+` concatenations, untagged
  templates). Numeric or unknown `+` subexpressions stay whole placeholders (`i + 1 + ' of ' + n` keeps
  `i + 1` as one value); parentheses around string literals do not prevent extraction. On the client,
  `<script>` literals a person reads: the value of a variable markup only displays when it
  reads as prose (a space or a capital), messages of `alert`, `confirm`, `prompt`, `toast.send` and what is assigned to
  `.textContent`, `.innerText`, `.title`, `.placeholder`, `.alt`. A variable the code also reads stays: `t()` would change what
  it compares or sends. Not moved: `app.brain` (static shell, B0132: put translatable chrome in `views/_layout.brain`),
  server code, data returned by `load()` and props defaults.
- **Key names** (2026-10-31): `<scope>.<first four ASCII words>`. The scope is the view path without `views/` and
  `index` (`views/account/index.brain` is `account`, `views/posts/[id]/edit.brain` is `posts-id-edit`, `home` for the
  root) or the component file name. Words are folded to ASCII (accented Latin letters lose the accent); a text with no word
  gets `text_<hash>`. The same text in the same file reuses its key; a different text with the same name gets `_2`;
  a key already in the default file with the identical text is reused. Each key appears once in every locale file.
- **Our own standard, no library** (Chris 2026-10-13): no i18next, FormatJS or ICU runtime. The format above is the
  whole standard (flat JSON, `{var}` placeholders, `one/other/...` plural objects named after `Intl.PluralRules`
  categories), small enough to implement in the compiler and runtime by hand, and the compiler understands it: it
  reads the locale files, checks every `t()` key and placeholder at build time, and resolves static keys itself.
- Localized link identity (2026-11-05): build/browser/Node look up translated source routes with trailing slashes
  removed, then preserve a requested slash, query and fragment. A full current-locale prefix segment is already
  localized even when a dynamic route regex matches it. Root-relative canonical overrides use the same translation
  table in build and normal/streamed SSR; SSR retains the app base once. Absolute canonical overrides stay explicit.
- Build-time load context (2026-11-05): the prerender runner installs the job's route, locale, alternates and
  dictionary before importing its server module or executing load. Existing locale()/t() APIs see that job's
  language even after awaits and alternating-language jobs; subsequent component rendering shares the context.
  This does not add a new paths(locale) contract or promise locale-dependent module-level constants are reevaluated.
- **Only the selected language is loaded; everything else on demand** (Chris 2026-10-13):
  - Prerendered and SSR pages already contain their text in the page's language (static `t('key')` calls are
    resolved at build, so no dictionary is shipped for them).
  - What the client still needs (dynamic keys, plurals, islands) goes in one hashed chunk per view and locale with
    only the keys that view uses (`/_b/i18n-<view>-<locale>.<hash>.js`); the page preloads only its own locale's.
  - Switching language is a navigation to the other locale's URL, which loads that locale's chunks then; nothing of
    the other languages is downloaded before. The service worker precaches only the current locale (§35).
- **Routing:** the default locale has no prefix (`/about`); the others do (`/es/about`). Views are not duplicated:
  the router and the prerender produce one page per locale. Optional translated slugs per view:
  `<meta route es="/es/acerca">`. hreflang pairs pages by view plus page identity (S03): page 2 of a listing pairs
  with page 2 in each language, translated slug or not, and never with page 1. `/` with no prefix never auto-redirects by `Accept-Language` (crawlers and shared
  links must see the same page); instead the language switcher suggests the browser's language once.
- **`t()` in server code** (2026-10-31): `load()`, guards and form handlers overlap across awaits, so each runs with its page's language in an `AsyncLocalStorage` behind the `__brainLocale` accessor, after the route's dictionary loads. An RPC has no page: it keeps the default language. Dev runs requests one at a time and passes the language and dictionaries to the sidecar.
- **Framework texts:** messages the runtime and the kit write themselves read the app's dictionary by key and fall back
  to English when the key is missing, without loading `brain-i18n.js`: form validation (`form.*`, §14) and
  `<ErrorState>` (`error_state.offline`, `network`, `timeout`, `schema`, `unauthorized`, `forbidden`, `not_found`,
  `invalid`, `rate_limited` with `{n}`, `server`, `sign_in`, `retry`, `reference` with `{id}`). brain-pack ships the
  `error_state.*` keys in the dictionary chunk of the pages whose graph reaches `<ErrorState>`, like `form.*`.
- **Switcher:** `<LangSwitch>` in the `brain/ui` kit: links to the same page in each locale (keeps the path, query and
  hash), names each language in itself (`Deutsch`, not `German`), sets `hreflang` on the links; the scaffold puts it in the nav,
  hidden while there is one locale.
- **Document:** `<html lang>` and `dir` (`rtl` for ar, he, fa, ur) per page; CSS uses logical properties so RTL
  works (B0306 warning for `margin-left`-style physical properties when an RTL locale is configured).
- **Content:** collections per locale in `content/<collection>/<locale>/*.md`; an entry missing in a locale is shown
  at its own URL with the default locale's text (never a 404), marked `noindex, follow` and left out of the sitemap,
  hreflang, llms.txt and feeds (B1829), and `post.fallback` is true so the template can say so.
- **SEO** (SEO role, §33): `hreflang` for every locale plus `x-default`, sitemap `xhtml:link` alternates, `og:locale`
  and `og:locale:alternate`, `llms.txt` per locale, canonical per locale (never cross-locale).
- **Checks** (COMPILER): B1401 key missing in any locale (error, with an edit adding the key, value copied from the
  default and marked); B1402 value identical to the default locale's (warning: probably untranslated; info for words
  that are the same in both); B1403 hardcoded text with 2+ locales; B1404 placeholder mismatch (`{n}` in one locale,
  missing in another; error); B1405 unknown locale code in `brain.json`.
- **Commands:** `brain add locale es` creates `i18n/es.json` with every key, values copied from the default and marked
  untranslated (B1402 lists them), adds the locale to `brain.json`; `brain i18n --missing --json` lists the work left.
  The first extra language also applies every B1403 edit (so the result has no fixed text left), and when the new
  locale is `en`/`en-*` and the default is not English, the framework keys (`form.*`, `error_state.*`) are written
  in English instead of copied. `brain fix` applies identical edits only once (one key per text).
  Machine translation is never automatic: an agent translates the file, `brain check` validates it.
- **Owners:** COMPILER #149 (`t()` keys, B1401–B1405, B0306, translated-slug meta) · RUNTIME #150 (`t`, plurals,
  `fmt.*`, `locale()`, `<LangSwitch>`, `lang`/`dir` swap on navigation, one-time suggestion) · TOOLCHAIN #151 (config,
  per-locale routing and prerender, `brain add locale`, `brain i18n`, scaffold `i18n/en.json` with `t()` in every
  view) · SEO #152 (hreflang, x-default, sitemap alternates, og:locale, llms.txt and canonical per locale, content
  fallback) · QA #153 (a 3-locale example site, RTL check, the LLM gate: a brief asking for an English + Spanish site
  must pass with `brain add locale` and no hardcoded text).

## 38. HTML first, then hydration, everywhere possible (fixed 2026-10-13, Chris)

Every page arrives as complete HTML (prerendered, or SSR when it has `load`) and the client only hydrates what is
interactive (resumable islands, §13). Client-only rendering is the exception, never the default.

- **Works before the JS loads:** links are real `<a href>`; the router only enhances them. Forms are real
  `<form method="post">`: when JS has not loaded yet, the browser posts to the page and the server runs the same
  `server:` function (form-encoded body, same validation and CSRF defenses as RPC, §11.1), then re-renders the page
  with `f.err`/`f.error` filled or redirects. Once hydrated, `form()` takes over without a reload.
- **Text and data are in the HTML:** `t()` static keys, `load` data and SEO tags are rendered on the server; the
  client never fetches data it already has (props are embedded once, §22). i18n switching (§37) is a link.
- **What must stay client-only** (canvas, maps, browser-only APIs) is marked `<meta client>` in that component: it
  renders a sized placeholder on the server (no layout shift) and mounts on the client. B0730 (info) lists each one
  in `brain doctor`, so client-only code stays visible and rare.
- **Hydration cost stays small:** only components with state or handlers ship JS; static parts ship none
  (measured per view in the build summary, §23).
- **No lost first interaction:** clicks and submits inside a region that the router just brought back are held until
  that region's mount script has run, then replayed on the mounted view; the hold releases itself after 5 s (RUNTIME
  #291).
- **Owners:** COMPILER #154 (`<meta client>`, B0730, forms compiled as real `<form method="post">` with the server
  function's endpoint) · TOOLCHAIN #155 (form-encoded POST routing to `server:` functions in dev and in `server.mjs`'s
  manifest; prerender everything that has no `load`) · RUNTIME #156 (server side of the form POST in `server.mjs`:
  parse, validate, re-render with errors or 303; `form()` enhancement without double submit) · QA #157 (every
  scaffold and example page with JavaScript disabled: renders, navigates, the login and the forms submit and show
  errors; then the same with JS and no reload).

## 39. API shape for an LLM: declarative or a function (fixed 2026-10-13, Chris)

Every new capability picks ONE shape with this test; the answer goes in its INTERNALS section.

**Declarative** (an attribute, a `<meta>`, a markup block or a data literal) when all of these hold:
1. It describes *what* the component is or needs, not a sequence of steps (props, emits, SEO, route, fallback,
   client-only, cache policy, tools, the async states of a value).
2. Its vocabulary is closed and small, so the compiler validates every key and value and a typo is an error with
   an exact edit (an LLM fills a known template far more reliably than it composes calls).
3. It sits where the reader already looks: in the component's `<meta>` or around the markup it affects, never in a
   separate config file.
Examples already in BRAINLLM: `<meta props>`, `<meta emits>`, `<meta seo>`, `{#if}`/`{#each}`/`{#await}` (the
`{#await}` block forces loading, error and empty to be written), `api('/users', { get: { out: '[User]' } })` (a
data literal), `form(schema)`, `<meta client>`.

**A function** (a primitive called in BrainScript) when any of these holds:
1. It carries data flow or logic that composes with other values (`sig`, `drv`, `fx`, `resource(fn)`, `store`).
2. It runs at a moment chosen by the code (a submit, a click, a retry, a navigation).
3. Its options would be open-ended or depend on runtime values.

**Rules for both shapes**
- Declare the contract, write the logic: the shape of data and requests is declared (types, `api()`), their use is
  a function (`resource()`, `await save()`), their rendering is declarative (`{#await}`).
- No configuration objects with many optional knobs (the model invents keys); a closed vocabulary validated by the
  compiler, or separate small primitives.
- No behaviour hidden in naming conventions (magic method names, decorators): lifecycle and effects are explicit
  primitives the model can see and the compiler can check.
- One way per job: if a declarative form exists, the imperative equivalent is not offered to authors (it may exist
  internally).
- When unsure, QA runs the LLM gate on both shapes with the same briefs and the higher first-pass rate wins (§21).

## 40. HTTP client: robust by default (fixed 2026-10-13, Chris: "cancel, network errors, permissions, status codes, local cache")

Already in `runtime/brain-data.js` (#122): `api()` declares endpoints with typed `in`/`out` (validated both ways),
same-origin credentials, a 10 s timeout, `__proto__` stripping, an in-memory stale-while-revalidate cache per URL,
the opt-in `cache: '10m'` in the Cache API, invalidation of a path's GETs after a write, cancellation of a
`resource()` run when it re-runs, and `{ errors }` for form fields. This section adds what a real app needs, with
nothing for the author to wire (shapes chosen by §39).

**1. One typed error.** Every rejection is an `ApiError` with `kind`:
`'offline'` (no connection, detected before sending) · `'network'` (the request failed) · `'timeout'` · `'aborted'` ·
`'http'` (with `status`, and `errors` on 422, `retryAfter` on 429/503) · `'schema'` (the response did not match
`out`, with the diff). Every error also carries `id` (the server's request id) for support. In markup:
`{:catch e}` then `e.kind`, `e.status`, or `e.is(403)`.

**2. Error states solved once** (RUNTIME, `brain/ui` kit): `<ErrorState error={e} retry={...}>` renders the right
message and action per kind: offline and network → "You are offline / connection lost" + retry (auto-retries when
`online()` turns true) · timeout → retry · 401 → sign-in link · 403 → names the missing permission (never a blank
screen) · 404 → not found · 422 → points to the form fields · 429 → waits `retryAfter` · 5xx → generic message with
the request id. Views and the `{:catch}` of every template use it unless they need something special.

**3. App-wide policy, declared once** in `app.brain`, closed vocabulary checked by the compiler:
`<meta http base="/api" timeout="10s" retry="2" login="/login">`
- `base` prefixes relative endpoint paths; `timeout` (default 10s); `retry` (default 2).
  Not the same thing as `brain.json` `"base"` (#148): that one is where the whole app is hosted (`"/app/"`); the
  build prefixes it to every page, chunk, link, form and RPC, while `<meta http base>` only prefixes `api()` endpoints.
- `login`: a 401 anywhere redirects to `login?next=<current path>` (same origin only, §31).
- 403 from a view's `load` renders `views/403.brain` with status 403 (like `deny()`, §38); from a component it
  reaches `{:catch}` / `<ErrorState>`.
- Headers that depend on state (a bearer token for a third-party API) are logic, so they are a function in
  `api/_http.jsbrain`: `export default http({ headers: () => ({ authorization: 'Bearer ' + session.token }) })`;
  same-origin apps need nothing (the signed session cookie, §11.1).

**4. Behaviour every request gets**
- Cancellation: requests started by a component are cancelled when it unmounts; a `resource()` cancels its previous
  run; an imperative call returns a promise with `.cancel()`. Aborted requests never reach `{:catch}` or
  `reportError`.
- Retries only where safe: GET/HEAD/PUT/DELETE retry on `network`, `timeout`, 502, 503 and 504 with backoff
  (300 ms ×2, jitter) up to `retry`; 429/503 honour `Retry-After`; POST and PATCH are never retried automatically.
- Deduplication: identical GETs in flight share one request.
- Offline: `online()` is a signal (navigator.onLine + events); GETs answer from the cache when offline (marked
  `stale`), writes fail at once with `kind: 'offline'`.
- Local cache: in memory per session by default (SWR); `cache: '10m'` per GET endpoint persists in the Cache API;
  `cache: 'no'` turns caching off for an endpoint; `refetch()` on a resource forces a reload. Personal data is never
  persisted unless the endpoint says so, and logout clears it (§35).
- Upload with progress: an `in: 'file'` or `in: '[file]'` endpoint sends multipart and exposes progress as a signal:
  `let up = files.post({ file }); up.progress` (0–1); the request still supports `.cancel()`.
- Every request and response is visible in dev devtools and `brain_logs` (method, URL, status, ms, size; never
  bodies with secrets).

**5. Checks** (COMPILER): B1107 `fetch()` used directly in app code (warning: use `api()`, which brings all of the
above; B1103 was already "unknown type in api()") · B1104 an unknown attribute or value in `<meta http>` · B1105 `{#await}` without `{:catch}` (already B1101)
when the endpoint can fail with 401/403 and the view has no `<ErrorState>`.

**Owners:** RUNTIME #168 (ApiError, retries, dedupe, offline, cancel on unmount, upload progress,
`<ErrorState>`, `online()`, `http()` headers) · COMPILER #170 (`<meta http>`, B1104, B1105, B1107, `in: 'file'`)
· TOOLCHAIN #171 (`base`/`login` wiring in dev and SSR, devtools network panel feed) · QA
#172 (each error kind reproduced in the browser, offline mode, retry and Retry-After, 401 redirect, 403 view,
cancel on unmount, upload progress, the LLM gate with a brief that needs auth and an upload).

## 41. Core parity with Angular and React: lifecycle, state scopes, precise updates, data at scale (fixed 2026-10-13, Chris)

Audit of 2026-10-13 (every capability checked against SPEC, runtime and compiler): the reactive core, the 8 channels,
keyed lists, error fallbacks and forms are shipped; the gaps below close the distance to Angular and React. Shapes
follow §39; each primitive lands in SPEC as a planned block and unhides only after QA's gate (#172) shows the
first-pass rate does not drop.

**1. Lifecycle** (components run once; there is no re-render, so there are no update hooks)
- `mounted(() => { …; return cleanup })`: once, after the component's DOM is in the document (measure, focus,
  third-party widgets). On the server it never runs.
- `watch(() => query, (now, prev) => …)`: runs only when that value changes, with the previous value; the callback
  is untracked, so reading other signals inside never re-triggers it. `fx` stays for "keep this in sync".
- `await tick()`: after the pending DOM updates are applied (focus the new row after adding it).
- Cleanup is always the function returned by `fx`/`mounted`; there is no separate `onCleanup` (one way).
- Timers with automatic cleanup: `every('30s', fn)` inside a component stops on unmount and while the tab is hidden.

**2. State scopes: pick the narrowest that works** (the SPEC "Communication" ladder gains the group step)
| Scope | Write | Who updates |
|---|---|---|
| Local | `let n = sig(0)` | only the nodes of that component that read `n` |
| Group / subtree | `provide(Cart, store({ items: [] }))` in the subtree root; `use(Cart)` below | only readers inside that subtree; two subtrees get two independent instances |
| Global | `export default store({...})` in `channels/` | only readers anywhere that read the changed field |
| Server data | `resource(api.get)` | only the nodes that render that resource |
`store()` becomes valid inside a `provide()` call (a scoped instance) besides `channels/`; `$reset()` restores its
initial value; `persist` and `sync` work for both.

**3. Precise updates, guaranteed and measured** (no virtual DOM: the unit of update is a DOM node, never a component)
- Every store field is its own signal; writing `cart.total` re-runs only what read `cart.total`.
- Writes with an equal value (`Object.is`) are ignored, so nothing downstream runs.
- Lists are keyed: changing one item updates only that row; inserting or moving rows moves only those nodes (LIS).
- Nested data is replaced, not mutated (B0113 already enforces it); with keyed lists and per-field signals, a nested
  change touches only the row and fields whose values changed.
- `drv` memoizes: dependents re-run only when the derived value actually changes.
- QA proves it with a MutationObserver harness: changing one field in a page with 1 000 components mutates exactly
  the text/attribute nodes bound to that field (count = expected), in local, group and global scope, and in a
  1 000-row list. A regression fails the build bench.

**4. HTTP without bottlenecks** (adds to §40)
- Concurrency queue per origin (6 by default) with priorities: user action > visible resource > prefetch/background;
  background requests yield to user actions, and navigation aborts the previous route's low-priority requests.
- No waterfalls: `load` with independent awaits is a warning (B1106) with an edit to `Promise.all`; every
  `resource()` of a view starts in parallel at mount, and the router prefetches the next route's `load` data on
  hover/viewport (the router's own background fetch; Speculation Rules were removed because the router intercepts
  every click, so a browser prerender was never used).
- After a write, every mounted `resource()` on the same path re-fetches by itself (a version signal per path), so
  screens never show stale lists; `refetch()` forces it.
- Optimistic writes: `await todos.post(body, { optimistic: () => (list = [...list, body]) })` rolls back
  automatically if the request fails.
- Pagination: `resource(posts.get, { page: true })` → `posts.more()`, `posts.done`; infinite scroll is
  `{#each}` + `<Visible on:enter={posts.more}>` from the kit.
- Polling: `resource(f, { every: '30s' })`, paused while the tab is hidden or offline.
- Server-sent events: `let feed = stream('/events')` is a signal with automatic reconnection; WebSocket stays
  `store(init, { sync: 'ws', url })`.

**5. Router on the client**
- `navigate('/x', { replace })`; `let r = route()` with `r.path`, `r.params`, `r.query` as signals (read-only).
- The compiler adds `aria-current="page"` to `<a href>` that match the current route (and a class hook), so active
  links need no code.
- Leaving a page with a dirty `form()` asks before discarding (built in; no API), except a form holding a password field: a half-typed sign in is not work to lose, and the prompt was only noise.

**6. Environment and UI helpers** (each a read-only signal that cleans up after itself)
- `media('(max-width: 600px)')`, `visible(ref)`, `debounce(value, '300ms')`, `online()` (§40).
- `<Portal>` (render elsewhere in the document, e.g. a toast layer), `<component is={C} …props>` for dynamic
  components, `transition:fade|slide|scale` on elements inside `{#if}`/`{#each}` (Web Animations API, respects
  reduced motion), a retry button in every `<meta fallback>`, async field validation in `form()`
  (`email: 'email!', check: { email: async (v) => (await users.exists({ email: v })) ? 'Taken' : '' }`), slot props
  (`<slot item={x} />` read by the parent as `{#slot item}`).
- Dependency injection: `services/*.jsbrain` + `provide(Api, real)` / `use(Api)` (planned #65) is the DI story; tests
  swap services with `mount(C, props, { provide: [[Api, fake]] })`.

**Premise for every item above** (Chris 2026-10-13: "it must feel made for LLMs, zero human contact"): the reader
of everything the framework says is a model working alone. A capability is not done until it ships with: (a) a
compiler check for its likely misuse, with an exact edit (`brain fix` applies it); (b) a `brain add`/recipe when it
creates files; (c) visibility through MCP (`brain_inspect` shows each resource's state, request queue, route signals
and store scopes; `brain_logs` the requests); (d) one SPEC line in the shape the model already predicts; (e) a brief
in the LLM gate that needs it. No step may require a person to look at a screen.

**Owners:** RUNTIME #169 (sections 1–6 runtime) · RUNTIME #168 (§40 + section 4 queue and priorities) · COMPILER #170
(new primitives in scope, `store()` in `provide()`, `aria-current`, `transition:`, `<component is>`, slot props,
B1106) · TOOLCHAIN #171 (SSR no-ops, devtools network panel, route data prefetch) · QA #172 (MutationObserver precision
harness, HTTP scenarios, LLM gate per batch).

## 42. Every component is isolated by default (fixed 2026-10-13, Chris: "if a call fails, only that scope fails")

A failure stays inside the component where it happened; the rest of the page keeps working. Nothing for the author to
write: every component behaves as if wrapped in try/catch.

- **Default boundary on every component** (COMPILER wraps every `comp/`, `views/` and `app.brain` component; `ui/`
  ones are stateless and fall to their parent's boundary). The minimal boundary lives in the core runtime (core
  budget ≤ 4 KB gz stays); `brain-err.js` adds rich capture only when `--report` or devtools ask for it.
- **What it catches:** an error while the component sets up or renders, in its `fx`, `drv`, `watch`, `mounted`, its
  child list rows, or a `resource()` without `{:catch}`. The component's nodes are replaced by its fallback, its
  effects and handlers stop, and the error goes to `brain_errors` (component name, `file.brain:line` through the dev
  source map, #158).
- **Default fallback:** a compact inline notice ("This section could not load") with a **Retry** button that remounts
  only that component; `<meta fallback="…">` customizes the text. A component that fails again 3 times in 10 s keeps
  its fallback without the button (no retry loops).
- **Event handlers do not destroy the component:** an error in a handler (sync or async, including a rejected
  `await api…`) is attributed to its component and reported; the component stays as it was, and the user sees a
  short toast (the `ApiError` message for HTTP errors, §40). `form()` keeps handling its own submit errors.
- **Nesting:** the nearest boundary wins: a failing child shows its own fallback inside an intact parent; a view's
  own code failing shows the view's fallback inside the layout; only a failure in `app.brain` itself shows the
  app-level fallback.
- **Server:** during SSR/prerender a failing component renders its fallback into the HTML (the page still answers
  200) and the error is logged with the request id; `load` failures keep using `_error.brain` (§22).
- **Development:** in `brain run`, a component with a compile error renders, in its place, a box with the diagnostic
  and its exact fix while the rest of the app keeps hot-reloading. `brain build` still fails on any error (§21:
  nothing ships with an error).
- **Owners:** RUNTIME #173 · COMPILER #174 · TOOLCHAIN #175 · QA #176 (fault injection in every place listed above,
  3 levels of nesting, retry, SSR, dev compile error).

## 43. Intelligence for the agent while it builds; large apps by components (fixed 2026-10-13, Chris)

The bar (Chris): at the market's level and beyond, with tools no other framework offers, all driven by the LLM:
valuable information at the moment of building, performance, and development at record speed. And for large apps:
the model grows the app by components without ever reading huge files.

**1. Impact report on every save** (TOOLCHAIN #200, BUILD #201, RUNTIME #198). After each save in `brain run`, one
compact JSON record (terminal line in plain mode, MCP `brain_impact`, last 50 kept): new or fixed diagnostics with
their edits · Δ bytes gz per affected view (against the previous build) · Δ mount time of the touched component ·
a11y and SEO deltas for the views that use it · new requests and waterfalls in its `load`/`resource` · updates per
interaction (how many nodes an interaction mutates). The agent reads it instead of rerunning doctor.

**2. Why did this change** (RUNTIME #198, TOOLCHAIN #200): `brain_why "<selector>"` answers, for a node that
updated, which signal write caused it (component, field, `file.brain:line`) and the chain of `drv`s between them.
Dev only; nothing ships.

**3. Who reads what** (COMPILER #199): `brain map --json` adds the reverse dependency graph: for every store field,
prop, context key and resource, the components that read it. "If I change `cart.total`, these 3 components update"
becomes a query, not a guess.

**4. Large apps without reading large files** (COMPILER #199, TOOLCHAIN #200)
- `brain_card <component>` (MCP) returns only the contract: props with types and defaults, emits, slots, the
  stores/context it reads and writes, its children, its size, and its doctor status. A model composes a page from
  cards, never from bodies.
- Size limit per component: over 150 lines of markup+script (or 20 KB), B1010 (warning) suggests the split with an
  edit; `brain split <comp> "<selector>" <new-name>` extracts that subtree into a new component, infers its props from
  the variables it reads and its emits from the handlers it calls, and leaves the original using it. The result
  passes `brain check` or the split is refused.
- `brain map --json` paginates and filters (`--view`, `--comp`, `--reads cart.total`) so even a 500-component app
  answers in a few hundred tokens.
- The views stay thin: a view over the limit with repeated markup gets B1010 pointing to the repeated block.

**5. Record speed, measured** (QA #202): the agent loop (brief → app with `brain doctor` exit 0) is timed before and
after §43 on the same briefs; a 60+ component app is built by a model using only cards and the map, with 0 reads of
files over 200 lines. Both numbers go to the REPORT; a regression fails the bench.


## 44. Experimental Rust backend and durable work (2026-10-29)

Crate `brain-backend` owns only infrastructure and resolved data contracts; CLI `backend.rs` parses literal BrainScript
with the existing parser and generates small schema/policy/hook/job files. [BACKEND.md](BACKEND.md) defines the running
contract, limits, migration semantics, auth, private image storage, transactional hook outbox and leased Postgres workers.
[BACKEND-DESIGN.md](BACKEND-DESIGN.md) records current Fastify study, latest stable direct crate selections, LLM-oriented
deviations and the unmeasured HTTP/SSR topology decision. No replacement for existing production APIs is claimed.

Navigation update (2026-10-29): `brain-pack::navigation` emits page-local `brain-next` routes/assets before signature and
CSP/SRI. The router validates plans against current links and uses a dedicated content-hashed `brain-prefetch.js` worker
when available, with a main-thread fallback. Two concurrent requests, 512 KB/page, 4 MB tab memory, six-second deadline,
48 preload links; a click reuses a speculative request still in flight (its deadline dropped) instead of starting a second one (0.6.0). Writes advance an epoch and abort
requests, preventing late responses from reappearing after logout. Build changes stop speculation; stop removes owned
workers/listeners/idle jobs. Static assets retain one-year immutable HTTP caching and cache-first service-worker reuse;
data/private HTML remains separate. Browser/benchmark acceptance is tracked in GAPS.md.

Multi-process and caching (2026-10-31): `cluster.rs` moves route `rate` and API-key per-minute windows to
`_brain_rate_windows` (one atomic upsert, digest subjects) and relays socket events with `pg_notify` +
`PgListener` when `Platform.cluster.shared`; the global per-address governor stays per process. `http/cache_http.rs`
is the innermost layer for `GET /api/...`: it hashes bodies up to 2 MiB into weak ETags (handlers may set a strong one;
`read` uses id + xmin + visible keys), answers 304, and marks anonymous reads of `server/cache.jsbrain` resources with
a `Declared` extension that `response_headers` turns into `Cache-Control` instead of `no-store`. `If-Match` is checked
in `precondition` against the same xmin the write CAS uses. See [backend/cluster.md](backend/cluster.md) and
[backend/caching.md](backend/caching.md).

Client navigation states (unreleased, `runtime/brain-router.js` `load`): one navigation token (`seq`) decides every late
callback. A superseded navigation never swaps, never runs its View Transition callback, never scrolls or moves focus
(History fallback), and never mounts its loading view. The current page stays until the next one is parsed; DOM, head,
active links and the next-route plan change in one swap callback. The region gets `aria-busy="true"` and `route().navigating`
is true at once; at 150 ms `#brain-nav` shows (the app's own element, else a fixed native `<progress>`); at 400 ms the
nearest `_loading.brain` replaces the region. A fetch, parse or swap that throws ends in
a full page load, never a half-swapped page. View Transitions are skipped by CSS under reduced motion (SPEC). The nonce for recreated scripts is read once at `router()` start.
After a swap the current navigation runs `landed`: `place` focuses the fragment target, else the region's `h1`, else the
region (`tabindex=-1` added, `preventScroll`), unless focus is already on a descendant of the region; the Navigation API path
passes `focusReset: 'manual'` so the browser neither resets focus to `body` nor overrides this, and keeps its own scroll
handling. A mount script replaces the server's nodes, so `hold`'s `$d` setter calls `$mounted`, which brings focus back only
when it fell to `body`. `#brain-announce` (`aria-live=polite`, created at `router()` start, outside the region) takes
`document.title`. A root `_loading.brain` alone emits one `^/` pattern; an empty `fullLoad` is omitted.
Past 8 s a navigation becomes a full load. `hold` lets real links through. `predict` orders its queue by rank: intent -1,
main links by `getBoundingClientRect().top`, plan 1e6, `footer`/`aside` 1e7 + top. Measured on the admin edit route of the
CRUD fixture (`backend.rs` test): 30686 B of the 30720 B default budget; the shell's tag indentation is not sent (`unindent_shell`).


## 45. Agent coordination and focused evidence (2026-10-30)

`brain-cli::changes` coordinates per-file expiring leases and SHA-256 revisions. Checked private snapshots precede
short commits under the existing writer lock. Each file replacement is atomic; batches use durable before/after
journals and conditional recovery, not filesystem-wide atomic visibility. External editor races remain explicit.
See [CHANGES.md](CHANGES.md) for request schemas, privacy, bounds and recovery.

Release 0.5.0: state, preview-admission and writer leases explicitly unlock before descriptor close. A forked
child or cloned descriptor must not extend a completed operation's lease. Real contention returns immediately;
only interrupted lock syscalls receive up to three attempts. Filesystem errors retain their category. Retained-
descriptor regressions verify both immediate reuse and continued exclusion while a successor holds the lock.

`intelligence` builds static evidence across routes, imports, literal API calls, policies, resource hooks and jobs.
Bounded graph pages, task context, impact, why and preventive checks distinguish compiler observations from runtime
proof. [INTELLIGENCE.md](INTELLIGENCE.md) specifies unknown edges and context byte estimates.

`recipes` prepares resource-ui and auth-ui operations without writing. Applying them uses the change protocol;
experimental Rust sessions remain separate from JS SSR. [RECIPES.md](RECIPES.md) records deployment requirements.
`interop` is an extended primitive mapped to `brain-interop.js`; foreign libraries own only the adapter mount.
[INTEROP.md](INTEROP.md) defines deferred mount, explicit updates and component disposal.

Internal backend failures capture a private bounded native stack and Rust callsite with the HTTP request ID;
message payloads, SQL values and credentials are excluded. Public responses contain actionable codes and request IDs.


## 46. Native evidence, helpers and emitted byte accounting (2026-10-30)
List/text helpers (2026-11-04): scope maps `range` to `brain-list.js` and `text` to `brain-text.js`, admitted by the
pure-server import path too. Both stay outside the core. Numeric iteration is half-open, safe-integer and bounded to
10000 outputs; classic numeric loop syntax is still rejected. OSA distance reuses three bounded typed rows, scans
only the cutoff band and exits when the row minimum exceeds the cutoff; full-matrix oracle tests cover recycling,
cutoffs and transpositions. Folding uses platform NFD plus the stated combining-mark range. Fuzzy matching validates
all candidates, caps aggregate candidate text at 1048576 UTF-16 units, reads a fixed number of array indices rather
than an overridable iterator, preserves original values/order and derives its cutoff from folded needle length. No persistent
cache, DOM dependency or locale state is introduced.


Native type inference (`brain-check::infer`) consumes literal named schemas through `Ctx.type_shapes`. B2001–B2004
ship as warnings: proven prop, field, API-body and form/binding inconsistencies. Unknown mutation, alias and dynamic
flows are not assigned invented types. [NATIVE-TYPES.md](NATIVE-TYPES.md) records scope and remaining flow analysis.

Browser compatibility uses the pinned generated web-features dataset, corrected from the handoff worktree, with
verified provenance and bundled license. `Ctx.browser_target` defaults to widely available; B2101–B2103 report known
API/CSS/HTML target violations or invalid targets as warnings. Static receivers and guards matter; unclassified keys
are not leaf-name heuristics. See [BROWSER-COMPATIBILITY.md](BROWSER-COMPATIBILITY.md). The older B12xx design above
was a proposal, not evidence that those diagnostics or automatic lowering were implemented.

Pure date and hash primitives map to separate runtime modules. Platform Date/Intl/Web Crypto provide semantics;
compiler-used named imports go through existing tree shaking. [NATIVE-LIBRARIES.md](NATIVE-LIBRARIES.md) specifies
UTC/calendar boundaries, secure-context crypto, bounds, helper authoring and contribution review.

Production `Output.ledger` derives raw/gzip/Brotli from final emitted assets. It separates inline scripts, initial
references, deferred dependencies and conservative graph totals; none is measured network traffic. Static successful
prerenders can emit zero executable JavaScript in mixed sites; exact full-document destinations remain eligible for
predictive fetching from interactive pages. See [OUTPUT-BUDGETS.md](OUTPUT-BUDGETS.md).

### Design-token reuse diagnostic

B2201 is a migration-safe warning enabled only for projects with a bounded regular `design.brain.json` foundation. The checker compares component declarations against project token values by role (color, spacing, radius or font). One exact matching token suggests `var(--token)`; ambiguous aliases, existing variables and unrelated property roles remain untouched. There is no automatic replacement because a custom property's dynamic cascade can change behavior. The quality collector reports the same native evidence from bounded root token input; full project `brain check` remains the authority.

### Native server resource facade (2026-10-30)

Internal Node socket bridge (0.4.1 development, 2026-10-30): a strict private manifest `nativeSockets: true`
or an explicit `createServer({nativeSockets:true})` override requires a fixed native loopback backend and installs
only `GET <base>/_brain/socket`. An explicit false override disables it; absent flags leave no Upgrade listener.
Malformed manifest flags are rejected even when overridden. Rust
still requires its own explicit socket configuration and owns authentication, origin allowlists, frame parsing,
heartbeat and row policy. The bridge preserves one canonical Origin and one `brain.v1` protocol, rejects queries
and duplicate credential headers, filters native identity cookies and forwards Bearer headers without fallback.
No request header, credential, frame or exception message enters its bounded diagnostic log.

The bridge admits at most 32 connections, with 16 KiB request headers/credentials, 4 KiB initial client bytes,
8 KiB upstream headers/initial bytes, 256 KiB queued writes per direction and 64 MiB aggregate transport bytes per
connection. Native `pipe` backpressure, a two-second stalled-write deadline, five-second handshake deadline,
70-second idle timeout, 15-minute lifetime
and one-second half-close flush bound owned work. These are transport budgets, not a claim about total process
heap. Other Upgrade listeners retain their endpoints; unknown requests are rejected when this is the only
listener. `closeAllConnections` closes current upgraded sockets but preserves new admission; `close`, explicit
stop and the process drain stop admission permanently. Production
places the helper under private `_server/`; the development sidecar copies its companion before importing the
server. Public opt-in settings feed the compiler manifest and fixed Rust development/static gateways. CLI and
MCP project development share the same startup validation; an automatically selected port outside the explicit
Origin allowlist is stopped before readiness. Full browser/native integration acceptance remains separate.

`nativeAuthBridge.resource` exposes a request-scoped repository via `ctx.backend.resource(name)` in both production
SSR/RPC and the development sidecar. It shares the fixed loopback authority, connection budget, cancellation and
HTTP schema/policy implementation with the native proxy. Only native identity cookies are forwarded; incoming
Bearer credentials are deliberately excluded to match `ctx.user`. Mutations require a same-origin POST and do not
retry. The response budget is a stricter 2 MiB, independent from media's 20 MiB proxy allowance. Repositories retained
after their response ends reject further calls. Server-only service/repository modules receive `ctx` explicitly.
`runtime/test/server-native-resource.test.js` checks actual loopback transport, CRUD, bounds and redacted failures.
Native OAuth redirect relay (2026-11-05): only GET/HEAD on /_brain/auth/linkedin/start with backend303 and the
exact https://www.linkedin.com/oauth/v2/authorization origin/path may pass configuredExternal to the final sender.
Credentials, fragments, raw controls/whitespace/backslashes, other ports/endpoints/methods/statuses stay rejected.
The native backend still owns configured client/redirect URI, one-use state, nonce and callback verification.
Google/Apple challenge/token endpoints and the generic OIDC verifier add no external redirect exception.

`nativeAuthBridge.backend(req,res,rid)` builds the frozen `ctx.backend` object for SSR, RPC and the sidecar alike. List
options map 1:1 to the HTTP query string (`field[op]=value`, `sort`, `cursor`/`after`, `include`, `aggregate`,
`count`); only unrepresentable shapes fail locally, so the backend stays the single validator. A 422 keeps the
backend's string field messages as `error.errors` (bounded to 64 entries); other failures stay generic.
`runtime/test/server-native-queries.test.js` covers the mapping, PATCH and slug redirects.
Slug access validation (2026-11-04) admits role-restricted writes as well as readonly/public fields. These write grants
do not hide the slug from readers; private, write-only and visibility-restricted slugs remain invalid. B0794's
explicit-writer fix therefore applies to unique slug fields without contradicting contract validation.
Job handlers get an equivalent client from `crates/brain-cli/src/templates/job-runner.mjs` (`jobBackend(env)`): same
mapping and budgets, but it authenticates with `X-API-Key: BRAIN_JOBS_API_KEY` (no cookies, no `Origin`), refuses a
non-loopback `BRAIN_BACKEND_URL`, deletes the key from `ctx.env`, and maps network failure or timeout (10 s) to 503.
`http_api_keys::resource_of` accepts `/api/<name>/redirects/<slug>` (read) besides collection and item paths.
`runtime/test/job-backend.test.js` spawns the real runner against a loopback stub.

Search language and weights (`search_config.rs`, migration 0015) travel as trigger arguments of
`_brain_search_update`: `=<regconfig>` (always from the `LANGUAGES` allowlist) and `field:W`; a bare field keeps the
old default weights, so resources without settings keep byte-identical `tgargs` and the drift check does not fire.
`migrate` compares the previous `tgargs` and, when they changed, rewrites rows with `UPDATE ... SET data=data` (the
trigger reindexes; `updated_at` is set by application code, not a trigger). Query paths use `regconfig(r)` in
`websearch_to_tsquery`. Before typo fallback, an `EXISTS` probe applies the same authorization and declared filters
as the page and stops at its first match; the separate exact total is computed only when requested. pg_trgm remains required.

The same bridge exposes `ctx.backend.login(credentials)` for same-origin POST server functions. It calls only the
fixed native password endpoint, validates native cookie attributes, returns a sanitized user and relays cookies
through both RPC and progressive form responses. Login/logout are exclusive within one request; the original
request identity is not rewritten. New scaffolds share this path for JavaScript and no-JS submissions.

### Production private-file boundary and runtime identity (2026-10-30)

Node static lookup checks decoded normalized public paths, independently from route dispatch: encoded separators
and underscores cannot expose `_server/`, `server.mjs`, host configuration or dotfiles. Native static lookup also
rejects private dotfiles, compressed server-source variants and backslash paths; `.well-known` remains public.
Private server diagnostics discard arbitrary exception messages but retain bounded source frames. Development
sidecars enable Node source maps so existing inline maps report the original `.brain` source line.
The pack fingerprint includes embedded `.mjs`, `.brain`, `.cssbrain` and JSON runtime sources as well as JS/Rust;
server runtime and UI changes cannot reuse a toolchain identity that omits those files.

### Server render errors and temporary failures in the private log (2026-11-02)

The core's console fallback (`BRAIN_ERROR: Operation failed. Try again.`) is a browser message. `server.mjs`
installs the core's `_err` hook once per core module: an fx/drv error during a render joins that render's failures
(`component failed during SSR`, with `rid`, `path`, `where` and `comp: null` when no `brain-guard` boundary owns it);
an error after the render window logs `component failed after SSR`. Boundaries keep reporting their own failures
(`render`, `B0908`, owners with `$g`), so each failure is logged once. A thrown value that is not an `Error` adds
`thrown: <kind>` and never its value. A 429/503 or an error marked `temporary` (closed connection, 504,
`BRIDGE_BUSY`) logs `code: NODE_TEMPORARY_ERROR` and `temporary: true` instead of `NODE_INTERNAL_ERROR`.
The development `load` sidecar describes non-`Error` rejections (plain objects, cycles, `undefined`) instead of
printing `[object Object]` or crashing on `JSON.stringify`.

### `<select multiple bind:value>` binds an array (2026-11-02)

Codegen writes `Array.from(e.target.querySelectorAll("option")).filter((o) => o.selected).map((o) => o.value)` on
`change` when the `<select>` carries a plain `multiple` attribute; other selects keep `e.target.value`. In the core,
`attr(select, 'value', …)` routes through `pick`: an array selects each option whose value attribute (else text) it
lists, also when `{#each}` options mount later (the `keepSelected` observer), and never lands on `select.value`.
SSR marks those options `selected` through `fillValues`.

### A visible lazy child is released on unmount (2026-11-02)

`<X lazy />` (visible mode) leaked its parent and child: after unmount and `io.disconnect()`, both survived forced GCs
for the whole `--heap` grace (B0601), while `idle`, `interaction` and eager children were released. The observer's
callback closed over `lazy()`'s scope (owner, props, built nodes), and Chrome still held the observer. The observer now
lives in its own function and reaches the trigger through a box emptied on stop, so whatever keeps the observer keeps
nothing of the component. Sweep: `visible(ref)` in brain-env.js (its callback holds only the signal) measured no leak; the
router's prefetch observer and the inspector's observers are page-wide, not per instance. Test:
`crates/brain-cli/tests/lazy_heap.rs` (four lazy modes plus `visible(ref)` under `--heap`).

### Instance ids restart on each client navigation (2026-11-02)

A component with ids numbers its instances (#268): the first keeps `id`, later ones get `id~2`, and `for`, `aria-*`
and `href="#…"` follow the same suffix. The count restarts when `globalThis.__brainIds` changes, which the server did
per render but the client router never did. A layout header mounted again with each swapped view therefore counted the
page it replaced and came back as `main-nav~2`, so a component rule `#main-nav { … }` stopped applying. The router now
bumps `__brainIds` right after disposing the old region (and before its early mount of a slow page), so the view that
mounts next numbers its ids from the start. Known limits: an element outside the swapped region keeps its id, so the same
component live there and inside the new view would share it; a single dev HMR remount or a guard Retry remount still
counts the instance it replaces. Test: `runtime/test/router.test.js` («a navigation restarts instance ids»).

### Native reads wait out an autodeploy drain (2026-11-02)

During an autodeploy the backend first answers `503 DRAINING` (Retry-After 5) on readiness while it finishes its
requests, then closes its listener until the new process listens. The old read retry covered only a refused or reset
connection for about 3.75 s, so a page rendered during the switch failed with a 503. A `GET` through `ctx.backend`
(resources, `service`, `operation` reads) and through `ctx.backend.files(bucket)` now asks again while the failure is
"not now": a refused or reset connection, or a 502/503/504 with no code, `DRAINING` or `OVERLOADED`. It waits 250 ms
doubling to 2 s, or the answer's Retry-After when longer, and gives up as soon as the next wait would pass its budget:
the load timeout minus one second (`nativeAuthBridge(..., readBudget)`, capped at 30 s; 8 s when the bridge is built
alone). A code of the backend's own (`BATCH_TOO_SLOW`) is an answer on purpose and is never repeated, and writes are
sent once. The failure that ends the wait keeps `temporary: true` and its Retry-After, so the page answers 503 with it
and the log says `NODE_TEMPORARY_ERROR`. Tests: `runtime/test/server-native-drain.test.js`.

### Mandatory native before-hooks and single-row conflict checks (2026-10-30)

`Resource.hooks.before_create/before_update/before_delete` reference private `server/hooks/<name>.jsbrain`
modules. `brain_backend::before_hooks::BeforeHooks` declares compiled names and receives the canonical
`{resource,operation,payload,existing,user,rid}` context. HTTP startup rejects references missing from the executor.
Every native CRUD write checks policy first, runs the callback when declared, and revalidates its returned data.
Delete supplies existing data; its output approves/rejects deletion rather than updating that data.

`hooks_executor::load` compiles the existing private server-module graph, probes callable default exports,
and owns two warm `load::Sidecar` instances. `try_acquire_owned` rejects excess requests without queueing.
`Sidecar::spawn_bounded` has a two-second combined stdin/response deadline, 64 KiB frames, private stderr,
explicit production mode and a 128 MiB V8 old-space budget. Payloads are objects within 48 KiB/depth 32/4,096
nodes and the complete context must fit its frame. Those restrictions apply to declared hooks; resources without
before-hooks preserve their existing input limit. The watchdog's separate Worker clears inherited Node execArgv,
checks the recorded parent PID and stops its own process when that owner disappears, including a busy main loop.
Timeout/frame failure kills its owned Unix process group and reaps the sidecar; Drop also closes and joins the bounded I/O threads. Native before-hook provisioning fails on non-Unix platforms in this checkpoint; ordinary frontend/dev sidecars remain available. The V8 old-space setting is not a universal process-memory limit.

Updates/deletes capture `xmin::text` with the server's existing row before callback execution. Checked SQL helpers
compare that short-lived internal revision and owner in the final mutation. A mismatch returns 409. Existing
transactional after-hooks enqueue only after a successful mutation in the same transaction. There is no transaction
covering arbitrary cross-record callback reads or external effects: use constraints or a dedicated transactional
operation for those invariants. No public durable version is inferred from xmin.

Application failure values are discarded. Rejections expose only known schema field names with static messages;
private diagnostics name the hook and explicitly identify module-only precision when no source line is proven.
See [BEFORE-HOOKS.md](BEFORE-HOOKS.md) for the author contract and verification commands.

Additive migration 7 fixes the zero-argument search trigger by coalescing `TG_ARGV` to an empty text array.
The mandatory-hook HTTP fixture deliberately uses `search: []` and asserts empty search text/vector after a real
write. No previous migration or drift guard was edited. Unexpected HTTP errors now publish RID/caller immediately;
one unqueued diagnostic job formats at most 32 KiB of stack output without formatting the underlying error value.
Backend library 22/22 and all 15 distinct integration cases passed across full/focused PostgreSQL 18 runs; the
compiled hook-executor scenario and the independent blocking-main-thread watchdog scenario passed separately.


### Doctor stage evidence (development 0.4.1)

Production building returns an internal assessment containing fresh output and a separate budget result.
Normal build callers must convert that assessment through its enforcing result; a failed budget remains an error.
Doctor may audit the fresh pages after that specific failure and includes measured route payloads/ledgers in the
failed build section. Check/code-generation/assembly failures have no fresh-output proof, so prior dist pages are
never presented as this run's SEO evidence. Browser tests already run before the production build. With no test
files, heap is explicitly unmeasured rather than an empty list presented as evidence of zero leaks.


### Native live-resource client and one origin declaration (development 0.4.1)

`brain-live.js` is opt-in through `liveResource(resourceHandle, options)`. Codegen preserves the first resource
argument as a handle using the same narrow pattern as `visible(ref)`. Shared native-binding metadata marks live
getters reactive in templates and constant-fold analysis, without a new signal kind. Existing resource/stream
members and the legacy state-publishing WebSocket protocol retain their behavior.

The broker shares one transport per loaded app runtime/tab, requires consistent public auth scope, waits for
ordered JOIN acknowledgements before refetch, and coalesces a single in-flight resource Promise plus dirty bit.
Owner disposal nulls a retained cell and releases resource/callback/owner references even when an external Promise
never settles. Private `_liveStats` reports only capacity counts; protocol metadata never contains row values.

`backend::socket_settings` reads bounded regular brain.json through the owned JSON parser, rejects duplicate backend
properties and validates canonical HTTP origins/socket subsets. Explicit --origin replaces the HTTP list. It produces
native backend configuration and typed pack options; no SQL/database dependency enters brain-pack. `build_with_options`
keeps legacy Input literals/build callers compatible and adds the validated boolean/public origin to the private
manifest BEFORE cache hashing. Static applications keep server.mjs absent. Exact public ws/wss CSP sources are additive;
legacy broad sources are preserved for a separate reviewed hardening change. See NATIVE-LIVE-RESOURCES.md for bounds,
principal-switch rules and the distinction between scoped unit proofs and full network integration evidence.


### Development value-module failures (2026-10-30)

The development server keeps component `.brain` compilation failures as renderable diagnostic fallbacks so
unrelated UI and HMR remain usable. An invalid `.jsbrain` is a value module, not a fallback component: it declares
a linkable default and rejects evaluation with `BrainCompileError`, preserving the original structured diagnostics,
code, file, line, column and fix. Durable compiler feedback is recorded before the HTTP response in either case.
A static named import can still fail at ESM linking before that error executes; persistent compiler feedback is
then authoritative. No named exports, substitute API methods or production traces are fabricated. A corrected
module is recompiled and its ordinary development reload event remains available.

## 47. Measured edit risk (2026-10-31)

`brain-cli::risk` scores an edit before it is made from `intelligence::impact` (reverse closure), checker findings and,
only on request, `impact::measure` against `budget.viewKB`. Factors, points and levels are fixed constants so the same
sources give the same report; [INTELLIGENCE.md](INTELLIGENCE.md#measured-edit-risk) lists them. `preflight` embeds the
static score (no build). The score orders review effort and names `requiredChecks`; it never claims runtime safety.
Atomic changesets, claims and the multi-agent view that consume it are planned for 0.6.1.

## 48. System identity and key-backed files (2026-10-31)

A system key is a row flag (`_brain_api_keys.system`, migration 0019) settable only through `api_keys::create_system`,
which the CLI reaches. `http_api_keys` runs the request inside the `api_keys::as_system` task-local; the bypass reads
`system_active()` at the four central checks (`Policy::allows`, `ownership::scope`, `visibility`, `domain` roles plus
`files::role_allows`), so no handler can forget it and work spawned on another task (socket fan-out) does not inherit
it. Runtime hooks run in another process, so they see no bypass. Scopes remain the boundary: wildcard resources are
refused at creation. `crates/brain-serve/src/key-backend.js` is the single key client, concatenated into both the job
runner and the route sidecar, which delete every key variable from `process.env` before importing user code.

## 49. Development trace (2026-10-31)

`brain run` and MCP `brain_run_start` call `brain_serve::trace::enable(root)`; nothing else does, so `brain test`,
`brain doctor`, the debugger and every production path never record. Off with `--no-trace`, `BRAIN_TRACE=0` or
`brain.json` `trace.enabled: false`.

**Store.** `.brain/trace.db` (mode 600), in the app's gitignored `.brain/`. `crates/brain-serve/src/trace-store.mjs` runs in
a long-lived Node child (`node:sqlite`, Node 22.13 or newer) because the compiler crates carry no SQLite and
`runtime/brain-sqlite.js` is browser OPFS only. All redaction happens in that process, before the first write. Two
tables: `events` (id, ts, session, tab, request_id, module, kind, level, fingerprint, route, component, file, line,
duration_ms, status, data JSON) and `issues` (fingerprint, title, module, kind, level, count, first_seen, last_seen,
b_code, fix, route, component, file, line, last_request_id). Age first, then size: `trace.maxAgeHours` (72),
`trace.maxMB` (64), oldest events go first; bodies stop at `trace.maxBodyKB` (16).

**Sources.** Server: `handle` opens a `RequestScope` that records the request when the handler returns; the status comes
from `http::respond_with_length` or the proxied file writer through a thread-local that production never sets. Sidecar:
`TRC <json>` lines on the protocol channel for exceptions and `console.warn`/`error`, with the stack Node already maps
through source maps. Browser: `runtime/brain-trace.js`, imported by the dev client only and listed with the other dev
modules excluded from a build (empty stub when capture is off). Backend: `brain-backend/src/dev_trace.rs` appends to
`.brain/trace-spool/backend.ndjson` the `sqlx::query` events of a request (statement text, rows, time; bound values never
exist there) and the finished request by route template; installed by `brain backend serve` outside production only.

**One id.** The browser sets `x-brain-trace` (a UUID, only same-origin) and the dev server adopts it as `req.id`, which
the sidecar sees as `ctx.rid` and the native proxy forwards as `X-Request-Id`. A document cannot set a header on its own
load, so the server returns the id in `Server-Timing: rid;desc=…`. A failed request counts once per id in `issues`,
however many layers saw it.

**CSP.** A dev page gets `Content-Security-Policy-Report-Only` with the production directives (`cache::csp`) plus
`'unsafe-inline'` scripts, local HTTP APIs, wasm and Google sign-in, which a build adds per page. A build's `ext` sources
are not known to the dev server, so those appear as violations.

**Limits.** Server-function arguments are positional, so their top-level strings keep only their length. A streamed
upload's body is not recorded. Only requests through the dev server carry the id; a direct call to the backend gets its
own. A violation with no source file is placed at the active route.

**Codes.** B1840 failed request · B1841 server exception · B1842 CSP violation · B1843 cross-origin failure · B1844
socket failure (`brain-seo` `CATALOG`, §5 lists the B18xx family). An event may bring its own `code`, `fix` and `title`.

**A new module** picks a lowercase name and its kinds; no table changes. Browser:
`globalThis.__brainTrace('payments', 'declined', { level: 'error', title: 'Card declined', code, fix, request_id, data })`.
Another process: append the same object as one line, with `module` and `kind`, to
`.brain/trace-spool/<module>.ndjson`. `level` of `warn` or `error` opens an issue.

**Proof.** `crates/brain-cli/tests/trace_production.rs` scans a real `dist/`, `crates/brain-serve/src/trace.rs` tests drive
redaction, caps and grouping through the store, `runtime/test/trace.test.js` covers the browser half.

## 50. File deletion tombstones (2026-10-31)

Migration 0020 adds `_brain_file_deletions` and an `AFTER DELETE` trigger on `_brain_files`, the pattern of
`_brain_media_deletions`: any removal of a row records its `storage_key` in the same transaction, whichever path removed
it. `files::finish` removes the bytes and clears the tombstone, `files::sweep` claims due tombstones with `FOR UPDATE SKIP
LOCKED` and pushes `next_attempt_at` forward as its lease and backoff, and `files::sweep_loop` runs from `http::serve_full`.
`Storage::remove` treats missing bytes as finished and refuses a key outside storage, which then waits in the table instead
of being followed. Rejected uploads use `files::discard`. Bytes written just before a crash, with neither row nor
tombstone, are still not recovered: nothing records them until the row exists.

## 51. Server LLM module (2026-10-31)

`runtime/server/llm.mjs` exports `llmClient(env, signal)`. `server.mjs` puts it on `context()` as `ctx.llm`; the build copies
it to `dist/_server/llm.mjs`, and client chunks never import it. `brain run` writes it next to `.brain/server/_rt/server.mjs`.
Sidecars and the job runner have no `server.mjs` for every call, so `load::llm_script()` strips `export` and prepends the
module: appended text would stay uninitialized behind the runner's top-level await (`const` in the temporal dead zone).
Requests are built only in `llmRequest` and answers parsed only in `llmParsed` and `llmStream`; the file header records the
date and sources of the contract review. Anthropic uses `anthropic-version: 2026-10-21`, which rejects sampling parameters,
so the neutral API has none. OpenAI uses the Responses API with `store: false`; a gateway that offers only Chat Completions
is not supported. Thinking blocks and reasoning items are dropped. Limits: 32 MiB answer, 4 MiB event, no retries.
Browser streaming is absent on purpose: RPC is one JSON answer and no server route emits events.
B0920 is in `brain-check` (`llm_client`, `env_access`, `env::check_file`), B0923 in `crates/brain-cli/src/env.rs`, and B0921 and
B0922 only at runtime. Tests: `runtime/test/llm.test.js` (wire contracts against a fake provider) and
`crates/brain-cli/tests/llm.rs` (built app through RPC, no key material in client files).

## 52. Dev observability panel and Web Vitals (2026-10-31)

`runtime/brain-observe.js` is loaded by the dev client (`crates/brain-serve/src/client.js`) and excluded from builds by the
same lists as the other dev modules (`main.rs`, `impact.rs`). It stays closed until Alt+Shift+O or `?brain-observe`, so
screenshots and tests see an unchanged page; `sessionStorage` keeps it open across reloads. It reads `brain:net`,
`brain:hmr`, `brain-err`, Long Animation Frames and the `live` registry of `brain-dev.js`; Pick uses the `$i` mark on root
elements. It sends nothing over the socket: `brain-inspect` remains the recorder and this is the on-page view.
`runtime/brain-vitals.js` is a normal opt-in primitive module (`scope.rs`) and costs 1116 B gzip when imported. INP follows
the metric definition (slowest interaction, one in 50 discarded), and Event Timing is observed from 40 ms, so a faster
interaction reports nothing. Browser test: `scripts/test/dev-observe.test.mjs`.

## 53. Speculation Rules on document navigations (2026-10-31)

Commit e0e1fb4 removed Speculation Rules because the router intercepts same-origin clicks: a rule on those targets never
activated and doubled the request. That reason still holds, so rules appear only where a click is a real document
navigation: a zero-JS page lists public static routes (it has no router), and a router page lists only zero-JS targets
(`fullLoad`). `speculation_rules` emits a hover `prefetch`, never a prerender, so no target script runs ahead of the
visit. Guarded, SSR, dynamic, `/_` and current routes are never listed; at most 64 targets. `has_executable_scripts`
accepts the type because it runs no code, and the build hashes its body into `script-src` as for any inline script.
Opt-outs: brain.json `"speculation": false`, and `data-prefetch="false"` on a link or ancestor. A server sees
`ctx.prefetch` for `Sec-Purpose: prefetch` too. Browser proof: `scripts/test/speculation-browser.test.mjs`.

## 54. Backend import (2026-10-31)

`brain_backend::import` runs a file inside one transaction with a savepoint per row, so every bad row is reported and a preview is the same code path followed by a rollback. Reasons are built from fixed text and field names, never from a database message, because those quote the row. `auth::legacy_scrypt` bounds the parameters of an imported hash; `scrypt` is used without its PHC feature to avoid a second `password-hash` version next to `argon2`.

## 55. Accounts generator and catalog (2026-10-31)

`auth_setup.rs` writes the accounts path for `brain add auth` and `brain new --backend` and never overwrites a file.
`backend::start_for_dev` owns the child of `brain run --backend` (startup-deadline cleanup also kills and waits, 2026-11-04): it refuses a port that already answers (B0763), waits
for `/health/ready` instead of a TCP connect, sets the child's `PUBLIC_SITE_URL` when the dev port differs from the
declared site, and the printed page URL follows the origin the backend accepts. `build.rs` embeds the closure of
documents cited by SPEC, guides and manifests, plus `docs/backend/*.md`, and `brain guide docs/<path>` serves them; a
deny list keeps internal documents out. `capabilities.rs` renders `docs/CAPABILITIES.md` from `GUIDES`, `ADD_KINDS`,
`recipes::list`, the backend inventory and capability manifests; a unit test compares the bytes, so a new manifest needs
`brain capabilities > docs/CAPABILITIES.md`.

## 56. SaaS building blocks (2026-10-31)

Six independent slices. Tenancy compiles into the row scope (`ownership::tenant_scope`), so every read channel inherits it; organization references are verified by `_brain_org_row()`. The audit log is a trigger (`_brain_audit_row`) because a request-layer hook could be skipped by a new write path; `audit::mark` passes the actor through transaction-local settings, and the entry stores names only. A resource without `audit` has no trigger, so it pays nothing. Facets reuse `Ctx::push_where`, with `Ctx.facet` dropping only that field's own filters and never the scope. Geo uses an expression GiST index over `point(lng,lat)`, an expanded box (`near_boxes`) and a haversine recheck. Captcha runs in the handlers rather than a layer, after the route's IP rate limit and before the handler, and any answer other than `success: true` admits nothing. Credits take `FOR UPDATE` on the balance row before reading the idempotency key, which orders same-key races too. No new diagnostic codes: invalid contracts fail at load with a message and fix.

## Captcha contract and session controls (2026-11-01)

`crates/brain-backend/src/sessions.rs` holds the idle setting, list and revoke, the auth age and the one-use signup grant
(migration 0024). Revocation always matches `user_id`, so another account's id is indistinguishable from an unknown one. The
idle check lives in each credential's query (`Auth::user`, `Jwt::user`, `Jwt::refresh`), and it is one process-wide setting,
hence its own test executable (`tests/session_idle.rs`). The grant is consumed before the password runs, so a wrong password
spends it. The operator key is compared by digest and is deliberately missing from CORS and from both proxies.
`cache::captcha_csp` derives the policy from `verify_url` for known providers only.

## 57. Data maintenance, retired references and per-owner limits (2026-11-01)

Default repair (2026-11-04): `brain-backend::defaults` and `brain data defaults` preview absent/null key counts for
non-null declared defaults. `--apply` updates at most 1000 rows per field (explicit limit 1..10000), never overwrites
non-null values, and includes nulls only with `--include-null`. A shared migration advisory lock plus an in-transaction
recorded-contract recheck protects the source schema; one transaction makes selected-field updates atomic. Keyset
selection uses `FOR UPDATE SKIP LOCKED`; counts after each field report remaining work, without an implicit retry loop.
Lock/SQL/total deadlines are 2s/30s/60s. Values are parameter-bound and omitted from reports. Exact-type integration
covers historical missing keys, explicit-null opt-in, false defaults, batch limits, idempotence and unique-failure
rollback. The CLI validates option bounds before selecting a database. Contracts also reject defaults incompatible with a field's
pattern, including text, list elements and object keys; this protects existing migration backfills as well as repair.

`crates/brain-backend/src/maintenance.rs` is `brain data prepare|maintain`: `CREATE STATISTICS` over the expressions of declared filters (`query::statistics_expressions`, named `_brain_st_`, retired when they change), `SET COMPRESSION lz4` plus a keyset rewrite of `pglz` rows under `session_replication_role=replica`, `VACUUM (ANALYZE)`, a warm-up and an `EXPLAIN (ANALYZE,BUFFERS)` gate on the newest page. The gate selects `octet_length(data::text)`, because `SELECT data` alone never opens TOAST and would measure nothing.

An approved contract change runs `relations::retire`: keys not in the contract go (a changed action or target used to leave the old key next to the new one), and so do `ref_*` columns, with their index and the sync trigger, which `install` rebuilds. Unchanged recorded contracts run the same retirement during explicit migration, repairing mirrors left by older releases; pending unapproved contract changes remain untouched. Values stay in `data`. `install_indexes` drops `_brain_ix_*` indexes of removed filters and sorts on every migrate: they are derived and every write pays for them.

Lists never open the stored document to filter, sort, count, facet or aggregate (2026-11-02, a 5,700-row jobs table read 46,000 to 75,000 TOAST buffers per home page call): `query.rs` reads `query_keys`, a stored generated copy of `data` (`public._brain_query_keys_v1`) with every string cut at `TEXT_KEY` and lists and objects whole, kept in the row with `STORAGE MAIN` so a wide row moves `data` and the search columns out of line first. `install_keys` adds it under the migration lock, which rewrites the table once with no statement timeout; `db::verify` refuses to serve a database without it (`BDB_SCHEMA_DRIFT`). Equality on a value as long as the key still rechecks `data`, and text facets and `ne` read the whole text when it is longer than the key. Indexes hash the expression, so the migrate that adds the copy replaces every `_brain_ix_*` index. The wider heap can change plans: in `data_queries`, a rare typo is now answered from the trigram index instead of a sequential scan.

`$owner` in a unique set maps to the `owner` column. `perOwner: { except }` cannot be an index, since it depends on the owner's role, so `_brain_per_owner()` (migration 0025) checks under an advisory lock per table and owner, and raises `unique_violation` naming `per_owner_name()`, so `constraint_fields` reports it like an index. It fires on every write path; `verify` compares its arguments. `upsert_keys()` keeps `$owner` out of client upsert keys.

List counts read a document once per row with `Ctx::push_counted` when `decide_read_once` finds two or more data reads, a seq scan and TOAST larger than the heap; pages stay plain so they can use indexes. `?fields=` is trimmed in SQL by `ListRequest::stored_keys`. `reject_server_fields` also runs on batch and upsert, and takes the role for `access: { write }`.


Owned browser cleanup (2026-11-05): test/recovery tabs and screenshot comparison tabs share CDP target cleanup.
The captured target alone is closed, and its disappearance is confirmed through the inventory before returning.
A finite total cleanup budget covers requests/polling and restores the caller deadline on success or error. A
Chrome acknowledgement is not proof that the target has already disappeared; other tabs remain usable.
