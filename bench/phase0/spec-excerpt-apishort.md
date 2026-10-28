## 1. `.brain` file

Fixed block order, all optional except the one marked:

```html
<meta props="name:type=default ...">   <!-- types: str num bool list obj fn any; ? = optional -->
<script> BrainScript </script>
MARKUP (HTML + interpolation)
<style> CSS </style>
```

Props: `name:type=default` · optional `name:type?` · no `=` and no `?` = required. `tokens.brain.css` lives at the app root.

Component name = file name in PascalCase (`user-card.brain` → `UserCard`).
Level by folder: `ui/` micro (no `<script>` with state) · `comp/` component · `views/` view/route.

## 2. BrainScript (closed subset of JS)

Allowed: `let` `const` · arrow `() =>` and `function` · `if/else` · `for (x of y)` · `return` · `await` · `try/catch/finally` · `throw` ·
objects, arrays, spread, destructuring · template strings · JS operators · `import x from './a.brain'|'./a.js'`.
Forbidden (error with fix): `class`, `this`, `var`, `with`, `eval`, `new Function`, generators, getters/setters,
labels, `for in`.

Primitives (the only globals):

| Primitive | Signature | Semantics |
|---|---|---|
| `s` | `s(v)` | reactive state; read and written like a normal variable (`n++`, `n = 5`) — the compiler rewrites it |
| `d` | `d(() => expr)` | memoized derived value, read-only |
| `f` | `f(() => { ...; return cleanup? })` | effect; automatic tracking; cleanup on unmount |
| `load` | `export async function load(ctx)` | only in `views/`; runs on the server, its return value becomes the props |
| `emit` | `emit('event', data)` | event to the parent (`on:event`) |
| `ref` | `ref()` | node reference (`bind:ref={x}`) |
| `log` | `log(...a)` | console.log that the prod build removes |

"The only globals" refers to the framework's: standard JS built-ins and Web APIs (Math, JSON, Date,
Promise, fetch, Array/String/Object methods) are allowed. Touching the DOM through `document`/`window` is
warning B0201 (fix: `ref()` + `bind:ref`). In `views/`, the return value of `load()` arrives as `props`
(`props.users`). `emit('event')` without data is valid.

## 3. Markup

- Interpolation `{expr}` · attributes `attr={expr}` · `class:name={bool}` · `style:prop={v}`
- Events `on:click={…}`: a bare identifier is a function reference and receives the Event (or the `emit`
  data); any other expression is a statement. `on:submit` does an implicit `preventDefault()`.
- Two-way binding: `bind:value={s}` · `bind:checked={s}` · `bind:ref={x}`
- Flow: `{#if c}…{:else if d}…{:else}…{/if}` · `{#each list as it (it.id)}…{:empty}…{/each}` (key required)
- Async: `{#await p}…{:then v}…{:catch e}…{/await}`
- Components: `<UserCard user={u} on:select={pick} />` · default slot `<slot />`
