# guide: data — load, server functions, resources, channels, caching

## When
A view needs data from a database or another service, the client reads or writes an API, or several components share
state. Pick the first row that fits:

| Need | Use |
|---|---|
| data a page needs to render (SEO-visible) | `export async function load(ctx)` in the view (server only) |
| a write or a server action from the page | `server: async function save(ctx, data) {…}` in the `.brain`; client calls `await save(data)` |
| a typed HTTP endpoint the client lists, polls, pages | `api/*.jsbrain` + `resource()` |
| server-only helpers (db, secrets) | `server/*.jsbrain` starting with `<meta server>` (used only in `load`/server functions) |
| state shared by distant components | `store` in `channels/*.jsbrain` |
| a broadcast (toast, analytics) | `bus` in `channels/*.jsbrain` |
| values for a subtree | `provide(K, v)` at its root, `use(K)` below |

## Steps
1. **Shape first.** `brain add type User --fields "id:num! email:email!"` → `types.jsbrain`.
2. **Endpoint.** `brain add api users --get "[User]" --post "User:User"` →
   `export default api('/users', { get: { out: '[User]' }, post: { in: 'User', out: 'User' } })`. Responses are validated.
3. **Read in a component.** `let users = resource(usersApi.get)` and render it with `{#await}` covering loading, error
   and empty:
   `{#await users}…{:then list}{#each list as u (u.id)}…{:empty}…{/each}{:catch e}<ErrorState error={e} />{/await}`.
   Pages: `resource(posts.get, { page: true })` → `posts.more()`. Polling: `{ every: '30s' }`.
4. **Write.** `await usersApi.post(user)`; every `resource` of that path refreshes by itself. `list.refetch()` forces it.
   Optimistic: `todos.post(x, { optimistic: () => … })` rolls back on failure.
5. **Server-rendered data.** In the view: `export async function load(ctx) { return { user: await db.get(ctx.params.id) } }`;
   the result arrives as `props`. Start independent awaits together (`Promise.all`) to avoid B1106.
6. **Shared state.** `brain add channel cart --store "{items:[]}"` → `provide(Cart, store({ items: [] }))` in the
   subtree root, `use(Cart)` below. `store`/`bus` exist only in `channels/` (B0806).
7. **App-wide HTTP policy** (optional, in `app.brain`): `<meta http base="/api" timeout="10s" retry="2" login="/login">`.
   A 401 then redirects to `login?next=<path>`; GETs retry on network errors; POST/PATCH never retry automatically.
8. **Develop without a backend:** `brain run --mock` serves fake data generated from the schemas.

## Caching (automatic; never write a service worker or cache headers)
- `brain build` adds immutable hashed assets, a service worker (visited pages work offline) and an in-memory
  `resource()` cache. Opt out in `brain.json` `"cache": { "sw": false }` (also `http`, `offline`, `data`).
- Keep a GET across visits: `api('/posts', { get: { out: '[Post]', cache: '10m' } })` — GET only, never for personal
  data (a malformed `cache` is B1102).
- Server pages: `<meta cache="60">` in a view rebuilds it at most every 60 s (stale-while-revalidate).
- Keep client state across visits: `store(init, { persist: true })`. Never `sync: 'storage'` for the signed-in user.
- Heavy computation off the main thread: name the module `*.worker.jsbrain`; its exports become async functions.

## Verify
```bash
brain check --json          # []
brain run --mock            # every view renders with fake data
brain test                  # resource states covered by tests
```
Over MCP: `brain_load` runs a view's `load`, `brain_api_call` calls an endpoint, `brain_mock` controls fake data.

## Common mistakes
| Mistake | Code | Fix |
|---|---|---|
| `fetch()` in app code | B1107 | declare an `api()` endpoint (validation, retries, cancel, cache) |
| `{#await}` without error or empty branch | B1101 · B1105 | add `{:catch e}<ErrorState error={e} />` and `{:empty}` |
| sequential independent awaits in `load` | B1106 | `const [a, b] = await Promise.all([…])` |
| server module imported by client code | B0701 | use it only inside `load` or a `server:` function |
| `entries('blog')` on the client | B0705 | read collections in `load()`/`paths()` |
| private env var in client code | B0702 | only `process.env.PUBLIC_*` on the client |
| `cache` on a POST or without a unit | B1102 | GET only, `cache: '30s' \| '10m' \| '1h'` |
| store used by one component | B1002 | a local `sig` instead |
| `items.push(x)` | B0113 | `items = [...items, x]` |
| `store`/`bus` outside `channels/` | B0806 | `brain add channel <name> --store|--bus` |

## Minimal correct example
```js
// api/posts.jsbrain
export default api('/posts', { get: { out: '[Post]' }, post: { in: 'Post', out: 'Post' } })
```
```html
<!-- comp/post-list.brain -->
<script>
  import posts from '../api/posts.jsbrain'
  let list = resource(posts.get)
</script>
<section aria-labelledby="h">
  <h2 id="h">Posts</h2>
  {#await list}<p>Loading…</p>
  {:then items}
    <ul>{#each items as p (p.id)}<li>{p.title}</li>{:empty}<li>No posts yet.</li>{/each}</ul>
  {:catch e}<ErrorState error={e} />
  {/await}
</section>
```
