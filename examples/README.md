# examples/

Source of the COMPILER snapshot tests and the QA benchmarks. Everything conforms to `SPEC.md` v0.

| File | Level | Covers |
|---|---|---|
| `comp/counter.brain` | comp | `sig`, `drv`, `fx`, `log`, `on:click` with a statement, `class:` |
| `comp/todo.brain` | comp | `{#each … (key)}` + `{:empty}`, `bind:value`, `on:submit`, derived values, filters |
| `comp/user-form.brain` | comp | optional prop `obj?`, `emit` with and without data, `bind:checked` |
| `ui/badge.brain` | micro | no `<script>`, dynamic attribute, required prop without a default |
| `views/dashboard.brain` | view | `import` .brain/.js, `load()`, `{#if}`, `ComponentRef` + `on:event`, async CRUD |
| `app.brain` | app | §10 shell: `<html lang>`, `<head>`, layout and `<slot />` (no `index.html`) |
| `lib/format.jsbrain` | — | BrainScript module without markup (`export default`), used by the dashboard |
| `tokens.cssbrain` | — | tokens used by every `<style>` (a nonexistent `var(--x)` is an error) |
| `lib/api.js` | — | plain JS escape hatch (in-memory API); published as `.jsbrain` (§10) |

## Conventions fixed in SPEC

The 9 initial assumptions were folded into the SPEC (2026-10-13): `on:x={ident}` = function reference, `on:submit` with
`preventDefault`, `bind:checked`, `try/catch`, `props` in views, optional `type?`, `emit` without data and JS
built-ins allowed (touching `document`/`window` gives B0201). Accessibility (B04xx) and `<meta seo>` in views (B0501) per §8.
