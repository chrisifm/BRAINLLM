# BRAINLLM SPEC
Normative. Anything not here does not exist. Internals: docs/INTERNALS.md (not needed to write apps).

## Files
- `app.brain` entry: `<html lang>`, common `<head>`, layout, `<slot />` for the view. No `.html` anywhere.
- `views/*.brain` routes (`views/users.brain` → `/users`). `comp/*.brain` stateful. `ui/*.brain` stateless.
- `channels/*.jsbrain` shared state/events. `lib/*.jsbrain` logic, no markup. `tokens.cssbrain` design tokens (root).
- `*.test.brain` tests next to the component. Name = file in PascalCase (`user-card.brain` → `UserCard`).

## Component (blocks in this order, all optional except markup)
```html
<meta props="user:obj label:str='Hi' size:num? value:str~" emits="select:obj close" expose="open">
<script> BrainScript </script>
MARKUP
<style> CSS </style>
```
Props: `name:type=default` · `?` optional · neither = required · `~` bindable (child may write it).
Types: str num bool list obj fn any.

## BrainScript (closed JS subset)
Allowed: let const · arrow/function · if/else · for-of · return · await · try/catch/finally · throw · objects arrays
spread destructuring · template strings · JS built-ins and Web APIs · `import x from './a.brain'|'./a.jsbrain'|'./a.js'`
· `export default`/`export const|function` (only .jsbrain, and `load` in views).
Forbidden: class this var with eval new-Function generators get/set labels for-in for(;;) while switch.
Signals are plain variables: `n++`, `items = [...items, x]`. Never mutate in place (`items.push` is an error).

| Primitive | Use |
|---|---|
| `sig(v)` | reactive state |
| `drv(() => e)` | derived, read-only |
| `fx(() => { …; return cleanup })` | side effect; return cleanup for listeners/timers |
| `export async function load(ctx)` | views only, runs on server; result arrives as `props` |
| `emit('ev', data?)` | event to parent; must be declared in `emits` |
| `ref()` + `bind:ref={r}` | DOM node access (never `document`/`window`) |
| `provide(K, v)` · `use(K)` · `key('name')` | context; `K` exported from channels/ |
| `store(init, {sync?:'tab'\|'storage'})` · `bus('name')` | only in channels/ |
| `log(...)` | dev log, removed in build |

## Markup
`{expr}` · `attr={e}` · `class:x={bool}` · `style:prop={v}` · `on:ev={fnRef | statement}` (`on:submit` prevents default)
· `bind:value|checked|ref|api={x}` · `{#if c}…{:else if d}…{:else}…{/if}` · `{#each list as it (it.id)}…{:empty}…{/each}`
(key required) · `{#await p}…{:then v}…{:catch e}…{/await}` · `<slot />` `<slot name="x" />` · `<Comp a={b} on:ev={f} />`.
Text interpolation is escaped. Static `id`s are made unique per instance automatically.

## Communication (components are isolated; pick the first that fits)
1 show parent data → props · 2 editable control → `~` prop + `bind:value` · 3 tell parent → `emits` + `emit` + `on:` ·
4 container → slots · 5 imperative action (open, focus) → `expose` + `bind:api={m}` + `m.open()` ·
6 data for many levels → `provide`/`use` · 7 state shared by distant branches → `store` in channels/ ·
8 stateless broadcast (toast, analytics) → `bus` in channels/. Nothing else: no globals, no `window.x`.

## CSS
Native CSS + nesting + `@mixin n {}` / `@include n;`. Scoped to the component automatically. No global selectors.
Colors/spacing only via `var(--token)` from tokens.cssbrain. Text color pairs (AA): fg/bg · muted/bg · accent/bg ·
ok/bg · bad/bg · on-accent on accent|ok|bad. No `color-mix()` or opacity on text color. Animations inside
`@media (prefers-reduced-motion: no-preference)`.

## Accessibility (build fails otherwise)
img has alt · clicks only on button/a · every input/select/textarea has label or aria-label · no heading level skips ·
one h1 and one main per view · buttons have text · no tabindex > 0.

## SEO
Every view: `<meta seo title="…" description="…" canonical? og:image? schema? robots?>`.

## Server and secrets
`load`, `.jsbrain` with `<meta server>` and `server:` handlers run only on the server. Client code may only read env
vars prefixed `PUBLIC_`.

## Tests (`*.test.brain`)
`test(name, async () => {…})` · `await mount(Comp, props)` · `await click(sel)` · `await type(sel, text)` ·
`text(sel)` `attr(sel, n)` `hasClass(sel, c)` `count(sel)` · `expect(x).toBe(y)|toEqual(y)|toBeTruthy()`.

## CLI (also MCP: `brain mcp`)
`brain new <app>` · `brain add view|comp|ui <name> --props "…"` · `brain check --json` · `brain test [--heap]` ·
`brain dev` (app :5180, devtools :5181) · `brain build` · `brain spec`.
`brain build` is production and always: minifies, mangles, obfuscates (JS/CSS/HTML), removes dead code and `log()`,
precompresses br/gz, adds SRI + strict CSP, emits sitemap/robots and server config. No flags needed.

## Loop
1 `brain add` · 2 edit · 3 `brain check --json` → apply each `fix` literally, repeat until `[]` · 4 `brain test`.
Error: `{"code","file","line","col","msg","fix"}`. Families: B00 structure · B01 script · B02 markup · B03 css ·
B04 a11y · B05 seo · B06 memory (dev) · B07 server/secrets · B08 isolation.

## Reference component
```html
<meta props="label:str='Tasks'" emits="done:obj">
<script>
  let items = sig([])
  let draft = sig('')
  let left = drv(() => items.filter(it => !it.done).length)
  const add = () => {
    if (draft.trim() === '') return
    items = [...items, { id: Date.now(), text: draft.trim(), done: false }]
    draft = ''
  }
  const toggle = (it) => {
    items = items.map(x => x.id === it.id ? { ...x, done: !x.done } : x)
    emit('done', it)
  }
</script>
<section aria-labelledby="t">
  <h2 id="t">{label}</h2>
  <form on:submit={add}>
    <label for="new">New task</label>
    <input id="new" bind:value={draft} />
    <button type="submit">Add</button>
  </form>
  <ul>
    {#each items as it (it.id)}
      <li class:done={it.done}>
        <input type="checkbox" aria-label={'Complete ' + it.text} checked={it.done} on:change={toggle(it)} />
        {it.text}
      </li>
    {:empty}<li>No tasks.</li>
    {/each}
  </ul>
  <p>{left} left</p>
</section>
<style>
  li.done { text-decoration: line-through; color: var(--muted); }
</style>
```
