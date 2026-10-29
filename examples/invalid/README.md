# examples/invalid/ — checker fixtures (SPEC §5, §8)

One minimal case per rule. Each `<case>.brain` produces **exactly** the Diag in `<case>.expected.json`
(an array, in the §5 format, with `file` relative to `examples/`).

Comparison contract for the COMPILER tests:
- `code`, `file`, `line`, `col` (1-based, pointing at the offending token): exact equality.
- `msg` and `fix`: must exist and be non-empty. The text here is the reference wording, not an equality check.
- No other Diag may appear: each case violates a single rule.

The tokens used by the `<style>` blocks live in `invalid/tokens.cssbrain`; `--faint` on `--bg` gives a contrast of 1.53:1.

| Code | Case | Rule (§8) |
|---|---|---|
| B0120 | `comp/b0120-export-const.brain` | `export const` inside a `.brain` |
| B0124 | `lib/b0124-two-exports.jsbrain` | more than one export in a `.jsbrain` |
| B0125 | `comp/b0125-block-order.brain` | blocks out of order (meta, script, markup, style) |
| B0126 | `comp/b0126-no-markup.brain` | component without markup |
| B0127 | `lib/b0127-dom-in-worker.worker.jsbrain` | DOM access inside a worker module |
| B0201 | `comp/b0201-document.brain` | touching `document`/`window` (warning) |
| B0301 | `comp/b0301-global-selector.brain` | global selector in `<style>` |
| B0304 | `comp/b0304-color-literal.brain` | color literal instead of a token (error) |
| B0305 | `comp/b0305-spacing-literal.brain` | spacing literal that matches no token (warning) |
| B0307 | `comp/b0307-unreachable-selector.brain` | selector that `@scope` never reaches (`main` in a component without `<main>`, warning) |
| B0308 | `comp/b0308-line-as-background.brain` | `--line` used outside borders (warning) |
| B0401 | `comp/b0401-img-alt.brain` | `<img>` without `alt` |
| B0402 | `comp/b0402-click-noninteractive.brain` | `on:click` on a non-interactive element |
| B0403 | `comp/b0403-input-no-label.brain` | input without a label or aria-label |
| — (B0403 silent) | `b0403-ok-dynamic-for-id.brain` | regression guard: `<label for={expr}>` + control with the same `id={expr}` is labelled; expected `[]` |
| B0404 | `comp/b0404-heading-skip.brain` | skipped heading level |
| B0405 | `views/b0405-two-h1.brain`, `views/b0405-two-main.brain` | a single `<h1>` and a single `<main>` per view |
| B0406 | `comp/b0406-button-no-text.brain` | `<button>` without accessible text |
| B0407 | `comp/b0407-tabindex.brain` | `tabindex` > 0 |
| B0408 | `comp/b0408-contrast.brain`, `comp/b0408-color-mix.brain`, `comp/b0408-opacity-text.brain` | token contrast < 4.5:1; `opacity` < 1 on a rule that sets `color` |
| B0409 | `comp/b0409-reduced-motion.brain` | animation without `prefers-reduced-motion` |
| B0501 | `views/b0501-no-seo.brain` | view without `<meta seo>` |
| B0502 | `views/b0502-no-keyword.brain` | indexable view without `keyword` (warning) |
| B0901 | `comp/b0901-fx-self-cycle.brain` | an `fx` that writes a sig it reads (cycle) |
| B0903 | `comp/b0903-recursion-no-base.brain` | recursion without a base case |
| B0906 | `comp/b0906-timer-no-cleanup.brain` | `setInterval` inside an `fx` without cleanup (§30) |
| B1001 | `comp/b1001-drv-passthrough.brain` | `drv` that only returns another value |
| B1004 | `comp/b1004-fx-writes-only.brain` | `fx` that only writes (should be `drv`) |
| B1005 | `comp/b1005-unused-prop.brain` | declared prop never used |
| B1009 | `comp/b1009-dead-function.brain` | function never called |
| B1101 | `comp/b1101-await-no-catch.brain` (+ `api/users.jsbrain`, `types.jsbrain`) | `{#await}` of a resource without `{:catch}` |
| B1102 | `api/b1102-cache.jsbrain` | malformed `cache` in an `api()` contract |
| B1601 | `views/b1601-no-description.brain` | `<meta seo>` without `description` |
| B1602 | `views/b1602-long-title.brain` | `title` longer than 60 characters (warning) |
| B1603 | `views/b1603-jsonld-no-type.brain` | `jsonld` without `@type` |
| B1604 | `views/b1604-canonical-a.brain`, `views/b1604-canonical-b.brain` | the same `canonical` in two views (one Diag in each) |
| B1701 | `comp/b1701-tool-no-function.brain` | WebMCP tool without a function of the same name |
| B1702 | `comp/b1702-tool-bad-name.brain` | tool name not in snake_case |
| B1703 | `comp/b1703-tool-untyped-param.brain` | tool parameter without a type |
| B0701 | `comp/b0701-server-import-client.brain` (+ `lib/db.jsbrain` with `<meta server>`) | server module used in client code (§11.1) |
| B0702 | `comp/b0702-private-env.brain` | `import.meta.env.X` without `PUBLIC_` in the client |
| B0703 | `comp/b0703-secret-literal.brain` | literal shaped like a key (`sk-…`) in the client |
| B0704 | `comp/b0704-server-not-async.brain` | `server:` function without `async` |
