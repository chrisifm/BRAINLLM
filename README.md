# BRAINLLM

**The web framework whose first user is a language model.**

A model reads an 18 KB spec, writes one `.brain` file per component, and gets every mistake back from the compiler
with the exact edit to apply. What ships is direct-DOM JavaScript on a 3.1 KB runtime: no virtual DOM, and no Vite,
webpack, esbuild or Babel anywhere in the toolchain.

Build from source today with `cargo build --release -p brain-cli`, then run `./target/release/brain new my-app`.
The npm commands below describe the prepared distribution; registry publication is a separate release step.

## Why it exists

Models already know HTML, JavaScript and CSS. What trips them up is everything around those: ten ways to do the same
thing, APIs that changed after their training cutoff, and errors that say what failed but not what to write instead.

BRAINLLM takes those choices away:

- **One spec, and nothing outside it.** [`SPEC.md`](SPEC.md) (also `brain spec`) is normative: if it is not in the
  spec, it does not exist. The model reads it instead of guessing.
- **One file per component.** Markup, script and style live together in a `.brain` file. The script is BrainScript,
  a closed subset of JavaScript with no classes, no `this` and no in-place mutation of state.
- **One place for every file.** Each kind of file has one folder, one naming rule and one generator
  (`brain add view users`), so there is nothing to invent about project layout.
- **Errors that carry the fix.** Every diagnostic names the change to make, and the JSON form includes the edit
  itself, ready to apply.

## A component

```html
<meta props="start:num=0 label:str='Clicks'">

<script>
  let n = sig(start)
  let dbl = drv(() => n * 2)
  fx(() => log(n))
</script>

<button on:click={n++} class:hot={n > 10}>{label}: {n} × 2 = {dbl}</button>

<style>
  button { padding: var(--space-2); &.hot { color: var(--bad); } }
</style>
```

State is a plain variable (`n++`), derived values update themselves, and the style is scoped to the component.

## When the model gets it wrong

```html
<script>
  let items = sig([])
  const add = () => items.push('new')
</script>
```

```text
$ brain check
comp/todo-list.brain:3:21 error B0113 `items.push()` mutates the sig in place and the view is not notified
  fix: reassign a copy: `items = [...items, item]`
```

`brain check --json` returns the same diagnostic with the replacement text and its exact range, so an agent applies it
without reading anything else:

```json
{"code":"B0113","line":3,"col":21,"fix":"reassign a copy: `items = [...items, item]`",
 "edits":[{"line":3,"col":21,"endLine":3,"endCol":38,"text":"items = [...items, 'new']"}]}
```

`brain fix` applies every mechanical edit at once, and `brain explain B0113` says why the rule exists, with a correct
example.

## Quickstart

```sh
npm create @christianirack/brainllm my-app
cd my-app
npm run dev          # brain run: dev server with hot reload
```

Every new app comes with an `AGENTS.md` that gives the agent its loop:

1. Read `brain spec` and `brain map --json` (the routes, components and props that exist).
2. Create files with `brain add view|comp|ui <name>`, then complete them.
3. Edit, run `brain check --json`, apply the fixes, repeat until it prints `[]`.
4. Done means `brain doctor` exits 0: check, tests, accessibility and leaks.

Then `brain build` for production and `brain start` to serve it with a supervisor. Every command accepts `--json`,
and `brain mcp` exposes the same commands as MCP tools over stdio. With Claude Code:

```sh
claude mcp add brain -- npx brain mcp
```

From source (Rust stable):

```sh
cargo build --release -p brain-cli
./target/release/brain new my-app
```

## Measured, not promised

The numbers below are historical measurements from 2026-10-13, not results for every later source revision.
Each comes with the command that reproduces it; the rows of
[`bench/REPORT.md`](bench/REPORT.md) add the raw data file and the versions compared.

| What | Result | Source |
|---|---|---|
| Views written from the spec alone that pass `brain check` on the first try | 20 views, two runs per model, with today's 18 KB spec: **Sonnet 5.5 40/40 (100 %) · Haiku 5.5 37/40 (92.5 %)**; every view passes once the model applies the fixes | row 8 · `bench/llm/gate-twice.sh` (`bench/llm/run.mjs --mode spec`) |
| Multi-page apps built from a one-paragraph brief with no human turns | **20/20** finished with `brain doctor` exit 0 (Haiku 5.5 and Sonnet 5.5, 5 apps per setup), 0 human interventions, 0 stuck loops | row 11 · `bench/autonomy/run.mjs` |
| Runtime size | **3,147 B gzip** · 2,828 B brotli | row 5 · `brain build` on `examples/`, then `gzip -9` / `brotli -q 11` on the core chunk |
| Bytes the browser receives for the same page (`/dashboard`, HTML + CSS + JS, gzip) | BRAINLLM **11.59 KiB** · Svelte 5 + Vite 16.72 KiB · React 19 + Vite 68.17 KiB | row 9 · `bench/build/run.sh` on the frozen app in `bench/build/app/` |
| Bytes per page of this project's website (JS + CSS + HTML, gzip) | **7.6 KB** for the home page: 3,107 B of JS, 762 B of CSS, 3,707 B of HTML | the per-page lines of `brain build` in `site/` |
| Production build of the example app, cache kept | **47.4 ms** median · Svelte 5 + Vite 578.1 ms · React 19 + Vite 437.2 ms, on a shared machine at load average 16–42 | row 7 · `bench/build/run.sh` |
| Hot reload, file write to change applied in Chrome | **2.2 ms** median for a style edit, 1.8 ms for markup or script, state kept in all 60 edits (plus ~27 ms of macOS file-event latency) | row 10 · `node bench/hmr/run.mjs --reps 20` |
| DOM writes when one field changes | exactly that field's nodes in **8/8** cases, dev and production, including a 1,000-row keyed list | row 17 · `node bench/precision/run.mjs` |
| Component encapsulation | **12/12** rules in dev and production: local state, context, store and update precision, CSS isolation, ids, slots, error isolation | row 15 · `node bench/scopes/run.mjs` |
| Cross-browser smoke: WebKit, Firefox, Chromium × dev and build × 4 apps, including a 1,000-row keyed list | **24/24 cells pass** | row 12 · `node bench/xbrowser/run.mjs` |
| Second visit (Slow 4G, 4× CPU) | **0 B** of unchanged JS and CSS downloaded again in 5 apps, LCP 3.5–4.1× lower | row 14 · `node bench/cache/run.mjs` |
| `brain start` under failure | back **87 ms** (median) after `kill -9`; **0 failed of 45,476** requests across three reloads under 50 clients (load average 5–21) | row 16 · `node bench/start/run.mjs` |
| This project's website ([`site/`](site/)), served by `brain start` | `brain seo audit` **100** on all 14 indexable pages; Lighthouse 13.5.0 **100 / 100 / 100 / 100** on all 14, mobile and desktop | `brain build` · `brain seo audit` · `brain start` + `npx lighthouse@13.5.0` per sitemap URL |

## Not there yet

BRAINLLM is at 0.1.0. Measured gaps, in the same report:

- **Rendering speed.** On the js-framework-benchmark (keyed, CPU geomean, lower is better) BRAINLLM is at **2.46**
  in an earlier run. The later run in report row 6 measures **1.28**, against vanillajs 1.00, Solid 1.10,
  Svelte 1.15, Vue 1.23, React 1.49 and Angular 1.97. The fault-isolation regression was fixed; the target
  of matching Solid remains unmet (`bench/jsfb/build.sh` + the official runner).
- Parts of the spec are still marked as planned: lazy components and `<Img>`, the `channel`/`service`/`api`
  generators, screenshots with baselines and budgets, and AST edits with `brain edit`. They are not available yet.
- The license has not been chosen yet.

## Repository

| Path | What it is |
|---|---|
| `crates/brain-lex`, `brain-parse`, `brain-check` | tokenizer, parser with spans, checker with fix-carrying diagnostics |
| `crates/brain-codegen`, `brain-css` | direct-DOM JavaScript and scoped CSS |
| `crates/brain-pack`, `brain-min` | bundler, prerender, cache and signatures, minifier |
| `crates/brain-serve`, `brain-mcp`, `brain-cli` | dev server with hot reload, MCP server, the `brain` binary |
| `crates/brain-seo` | robots.txt, sitemaps, `llms.txt`, social cards, structured data, the SEO audit |
| `runtime/` | the browser runtime, hand-written ES modules with no build step |
| `examples/`, `bench/` | example app, benchmarks and the LLM tests behind every number above |
| `site/` | the project website, built with BRAINLLM |
| `npm/` | the npm packages: launcher, per-platform binaries, runtime, `create-brainllm` |

## Read next

- [Migrate an existing React, Angular, Vue or Svelte app](docs/MIGRATION.md): a read-only planner and tested, incremental cutovers.
- [Improve the compiler while using it](docs/EVOLUTION.md): isolated proposals, validated binaries and automatic review-branch contributions.

Contributors can run `bash scripts/verify.sh` to build the current CLI and run all four required checks.
The same command runs in the verification workflow. LLM generation benchmarks are separate: evaluate a frozen
run with `node bench/llm/gate.mjs <run.json>`; historical gate results do not validate a newer spec or binary.

- [`SPEC.md`](SPEC.md): everything an app author, human or model, can write.
- [`docs/INTERNALS.md`](docs/INTERNALS.md): how the compiler, runtime and build work inside.
- [`CONTRIBUTING.md`](CONTRIBUTING.md) and [`AGENTS.md`](AGENTS.md): invariants and the definition of done for
  changes to the framework.
- [`llms.txt`](llms.txt): the entry point for a model reading this repository.

Created and maintained by **Christian Irack Flores Maldonado**, principal author.
Contact: [christian@irack.mx](mailto:christian@irack.mx).
Independent development: [ingenieria.dev](https://ingenieria.dev) · [irack.mx](https://irack.mx).
Human and LLM contributions are welcome through pull requests; the principal author reviews changes before merge.
