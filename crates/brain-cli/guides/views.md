# guide: views — routes, layouts, guards, navigation

## When
Adding a page, a dynamic route, a section with shared chrome, or protecting pages behind sign-in or a permission.

## Routing is the file tree
| File | Route |
|---|---|
| `views/home.brain` (what `brain new` creates) | `/` |
| `views/users.brain` or `views/users/index.brain` | `/users` |
| `views/users/[id].brain` | `/users/:id` → `ctx.params.id` in `load` |
| `views/docs/[...rest].brain` | `/docs/a/b/c` → `ctx.params.rest = 'a/b/c'` (ranked after static and `[id]`) |
| `views/404.brain` · `views/403.brain` | not found · access denied (`reason` prop) |
| `views/<dir>/_layout.brain` | wraps every view of its folder with `<slot />` |
| `views/<dir>/_guard.jsbrain` | runs before every view of its folder, on the server and before client navigation |
| `views/<dir>/_loading.brain` · `_error.brain` | shown while `load` runs · when it fails |

## Steps
1. `brain add view users` → `views/users.brain` with `<meta seo>`, `<main><h1>` and an optional `load`.
   Dynamic: `brain add view users/[id]`. Folder chrome: `brain add layout users`. Protection: `brain add guard admin`.
2. Fill `<meta seo title="…" description="…">` (B0501/B1601 without it; `brain guide seo`).
3. Exactly one `<main>` and one `<h1>` per view (B0405). Headings never skip a level (B0404).
4. Link the view from the navigation in `app.brain` with a real `<a href="/users">` (works before JS loads).
5. Dynamic data: `export async function load(ctx)` in the view (server only; `brain guide data`).
   `throw notFound()` → 404 with `views/404.brain`; `throw redirect('/x')` → 303.
6. `[id]` views that should be prerendered list their pages: `export async function paths() { return [{ id: '1' }] }`.
7. Guard: `export default (ctx) => ctx.user ? true : redirect('/login?next=' + ctx.url.pathname)`.
   `deny('reason')` renders `views/403.brain` with that reason. `ctx` = `{ params, query, user, session, cookies, url, env }`.
8. In code: `navigate('/x')` (no reload); `let r = route()` → `r.path`, `r.params.id`, `r.query.q` (reactive).

## Rules that bite
- `app.brain` is the static document shell: `<html lang>`, common `<head>`, site `<meta seo titleTemplate=…>`, nav and
  `<slot />`. Expressions, blocks or components there are B0132; put them in `views/_layout.brain`.
- `load` runs only in views; in `_layout.brain` it is not run (B0117 for an invalid `load`).
- In the browser there is no `ctx.user` (the session cookie is HttpOnly): a guard that would stop on it makes a full
  page load and the server guard decides. Never mirror auth in a client store to skip the guard.
- `redirect()` only to same-origin paths or hosts listed in `brain.json` `redirects` (B0710 open redirect).
- Hosted under a sub-path: set `brain.json` `"base": "/app/"` and keep writing app-absolute paths (`/about`).

## Verify
```bash
brain map --json            # the route appears with its file, params and load
brain check --json          # []
brain shot /users --text    # outline: one main, one h1, links reachable
```

## Common mistakes
| Mistake | Code | Fix |
|---|---|---|
| no `<meta seo>` or no description | B0501 · B1601 | add title and description |
| two `<h1>` or no `<main>` | B0405 | one of each per view |
| `location.href = '/x'` | B0129 | `navigate('/x')` or an `<a href>` |
| dynamic markup in `app.brain` | B0132 | move it to `views/_layout.brain` |
| `redirect(ctx.query.next)` unchecked | B0710 | accept only paths starting with a single `/` |
| `paths()` and `sitemap()` in one view | B0117 | prerender with `paths()` or list live pages with `sitemap()` |

## Minimal correct example
```html
<!-- views/users/[id].brain -->
<meta seo title={props.user.name} description={props.user.bio}>
<script>
  import users from '../../lib/users.jsbrain'
  export async function load(ctx) {
    const user = users.find(ctx.params.id)
    if (!user) throw notFound()
    return { user }
  }
</script>
<main class="page">
  <p><a href="/users">All users</a></p>
  <h1>{props.user.name}</h1>
  <p>{props.user.bio}</p>
</main>
```
```js
// views/admin/_guard.jsbrain
export default (ctx) => (ctx.user ? true : redirect('/login?next=' + ctx.url.pathname))
```
