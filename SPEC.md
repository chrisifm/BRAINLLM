# BRAINLLM SPEC
Normative. Anything not here does not exist. Internals: docs/INTERNALS.md (not needed to write apps).

## Files
- `app.brain` entry: `<html lang>`, common `<head>`, layout, `<slot />` for the view. No `.html` anywhere.
- `views/*.brain` routes: `users.brain` → `/users`. `comp/*.brain` stateful. `ui/*.brain` stateless.
- `views/users/[id].brain` → `/users/:id` (`ctx.params.id` in `load`) · `views/users/_layout.brain` wraps its folder · `views/404.brain`.
- `views/docs/[...rest].brain` matches `/docs/a/b/c` with `ctx.params.rest = 'a/b/c'` (ranked after static and `[id]` routes).
- Route files per folder: `_guard.jsbrain` (`export default (ctx) => ctx.user ? true : redirect('/login')`, runs on server
  and before client navigation; `deny('reason')` renders `views/403.brain`) · `_error.brain` · `_middleware.jsbrain`.
  `ctx` = `{ params, query, user, session, cookies, url, env }`. In the browser there is no `ctx.user` (the session
  cookie is HttpOnly): a guard that reads it and would stop the navigation makes a full page load, and the server guard decides.
- In code: `navigate('/x')` (router, no reload; never `location.assign`) · `let r = route()` → `r.path`, `r.params.id`,
  `r.query.q` (reactive).
- `services/*.jsbrain` are injectable: `provide(Api, realApi)` in `app.brain`, `use(Api)` anywhere; tests swap them with
  `mount(Comp, props, { provide: [[Api, fakeApi]] })`.
- `_loading.brain` shows while `load` runs. `<meta cache="60">` in a view = rebuild at most every 60 s.
- Languages: `brain.json` `"i18n": { "default": "en", "locales": ["en", "es"] }`; texts in `i18n/<locale>.json`
  (`"nav.home": "Home"`, plurals `{ "one": "{n} post", "other": "{n} posts" }`); write `t('nav.home')`, never literal
  text once there are 2+ locales. Add a language with `brain add locale es`, then translate that file.
  `fmt.date/number/money(v)` format for the current locale; `<LangSwitch>` links to the other languages.
  Content per language: `content/blog/es/*.md`; a missing translation shows the default text and `post.fallback`
  is true, so the template can say so.
- HTML first: use real `<a href>` and `<form>` (they work before JS loads and are enhanced after). Only a component
  that cannot render on the server (canvas, map) gets `<meta client>`.
- `lib/*.jsbrain` logic, no markup. `tokens.cssbrain` design tokens (root).
- `brain new` already creates the whole layout: `views/ comp/ ui/ channels/ services/ api/ lib/ server/ content/
  public/`, `brain.json`, `.brain_env`. Put each file in its folder (each holds a README saying what goes there);
  never create other top-level folders.
- Import only files that exist (`brain map --json` lists them). Primitives need no import.
- `*.test.brain` tests next to the component. Name = file in PascalCase (`user-card.brain` → `UserCard`).

## Architecture (generate, don't hand-create files)
Every file kind has one folder, one naming rule and one generator. `brain add <kind> <name>` creates it with the
pattern already in place (contract `<meta>`, typed stubs, a11y-correct markup, a colocated test).

| Kind | `brain add …` | Path | Purpose |
|---|---|---|---|
| view | `view users` | `views/users.brain` | a route: `<meta seo>`, `<main><h1>`, optional `load` |
| comp | `comp user-card --props "user:obj"` | `comp/user-card.brain` | stateful component |
| ui | `ui badge` | `ui/badge.brain` | stateless presentational piece |
| test | `test user-card` | `comp/user-card.test.brain` | colocated test for a component or view |
| view (nested) | `view users/[id]` | `views/users/[id].brain` | dynamic route (`ctx.params.id`) |
| layout | `layout users` | `views/users/_layout.brain` | wraps every view in its folder (`<slot />`) |
| lib | `lib format` | `lib/format.jsbrain` | pure functions, no markup |
| channel | `channel cart --store "{items:[]}"` · `--bus` · `--key` | `channels/cart.jsbrain` | shared state, event bus or context key |
| service | `service payments` | `services/payments.jsbrain` | injectable dependency (`provide`/`use`) |
| api | `api users --get "[User]" --post "User:User"` | `api/users.jsbrain` | typed endpoint + client |
| type | `type User --fields "id:num! email:email!"` | `types.jsbrain` | shared data shape |
| guard | `guard admin` | `views/admin/_guard.jsbrain` | protects a folder of routes |
| server | `server users-db` | `server/users-db.jsbrain` | server-only module (`<meta server>`) |
Names: files kebab-case, components PascalCase from the file name, one export per `.jsbrain`. Keep this layout;
`brain map` and every tool assume it.

## Component (blocks in this order, all optional except markup)
```html
<meta props="user:obj label:str='Hi' size:num?">
<script> BrainScript </script>
MARKUP
<style> CSS </style>
```
Props: `name:type=default` · `?` optional · neither = required. Types: str num bool list obj fn any.
More `<meta>` attributes: `value:str~` (`~` bindable: the child may write it) · `emits="select:obj close"` · `expose="open"`.

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
| `export async function load(ctx)` | views only, runs on server; result arrives as `props`. `throw notFound()` → 404 with `views/404.brain`; `throw redirect('/x')` → 303 |
| `emit('ev', data?)` | event to parent (`on:ev` on the parent) |
| `ref()` + `bind:ref={r}` | DOM node access (never `document`/`window`) |
| `log(...)` | dev log, removed in build |
| `provide(K, v)` · `use(K)` · `key('name')` | context; `K` exported from channels/ |
| `store(init, {sync?:'tab'\|'storage'})` · `bus('name')` | only in `channels/*.jsbrain`; never `sync: 'storage'` for the signed-in user (another tab's copy brings a logged-out user back): read the user from `load` |

## Markup
`{expr}` · `attr={e}` · `class:x={bool}` · `style:prop={v}` · `on:ev={fnRef | statement}` (`on:submit` prevents default)
· `bind:value|checked|ref={x}` · `{#if c}…{:else if d}…{:else}…{/if}` · `{#each list as it (it.id)}…{:empty}…{/each}`
(key required; list of primitives → key by value: `{#each perks as perk (perk)}`) · `{#await p}…{:then v}…{:catch e}…{/await}`
· `<slot />` `<slot name="x" />` · `<Comp a={b} on:ev={f} />`.
Lazy components: `<Chart lazy />` loads when visible · `lazy="idle"` · `lazy="interaction"`. Images: `<Img src alt width height>`
(lazy, decoding async, LCP priority set by the compiler). Interaction mode renders an accessible load button;
CSS remains eager to preserve styles when the deferred component appears.
Text interpolation is escaped. `id`s in `comp/` and `ui/`, static or computed (`id={'faq-' + it.id}`), keep their name
in the first instance (so `/page#id` links work); later instances get `~2`, `~3`… and their `for`, `aria-*` and
`href="#…"` references follow. Ids in views and layouts stay as written.
`<pre raw>` and `<code raw>` keep their text literal (code samples with `{` `}`).
`bind:value|checked` also accepts a field of a sig object: `bind:checked={prefs.email}`.
Raw HTML only as `{@html sanitize(x)}`.

## Lifecycle and helpers
`mounted(() => { …; return cleanup })` runs once after the DOM is in the page · `watch(() => q, (now, prev) => …)`
runs only when `q` changes · `await tick()` after DOM updates · `every('30s', fn)` timer stopped on unmount ·
`media('(max-width: 600px)')` `visible(ref)` `debounce(v, '300ms')` `online()` are read-only signals, read like any other ·
`<component is={C}>` · `<Portal>` · `transition:fade` on elements inside `{#if}`/`{#each}`.
Writes refresh every `resource` of that path by themselves; `list.refetch()` forces it.
Errors: `{:catch e}<ErrorState error={e} />{/await}` handles offline, 401, 403, 404, 422 and 5xx.
Group state: `provide(Cart, store({ items: [] }))` in the subtree root, `use(Cart)` below (one instance per subtree).
Lists from an API: `resource(posts.get, { page: true })` → `posts.more()`; `{ every: '30s' }` polls;
`todos.post(x, { optimistic: () => … })` rolls back on failure.

## Communication (components are isolated; pick the first that fits)
1 show parent data → props · 2 tell parent → `emit` + `on:` · 3 container → slots.
4 editable control → `~` prop + `bind:value` · 5 imperative action (open, focus) → `expose` + `bind:api={m}` + `m.open()`.
6 data for many levels → `provide`/`use` · 7 state shared by distant branches → `store` in channels/ ·
8 stateless broadcast (toast, analytics) → `bus` in channels/.
Nothing else: no globals, no `window.x`.

## CSS
Native CSS + nesting + `@mixin n {}` / `@include n;`. Scoped to the component automatically. No global selectors.
Color properties (color, background, border-color, outline-color, fill, stroke) and spacing properties (padding,
margin, gap, inset) use only these tokens (tokens.cssbrain); sizes (width, font-size, radius…) are free: `--space-1..6` `--radius` `--radius-sm` `--radius-lg` `--font`
`--fg` `--muted` `--bg` `--surface` `--line` `--accent` `--on-accent` `--ok` `--bad` `--glass` `--glass-edge` `--shadow`
`--scrim`. Text color pairs (AA): fg/bg · muted/bg · accent/bg ·
ok/bg · bad/bg · on-accent on accent|ok|bad. `--line` is for borders only (never a background). Modal and
overlay backdrops: `background: var(--scrim)`, never `rgb(0 0 0 / x)`. No `color-mix()` or opacity on text color.
Animations inside `@media (prefers-reduced-motion: no-preference)`.

## Accessibility (build fails otherwise)
img has alt · clicks only on button/a · use `<Input label="…">` `<Select label="…">` from the kit (a raw
input/select/textarea needs a `<label>`) · no heading level skips ·
one h1 and one main per view · buttons have text · links have a name (text, `aria-label` or img alt) · iframe
has `title` · no tabindex > 0. Warnings: generic link text ("click here", "read more") · alt that is a file name or
"image"/"logo" · `<video>` with controls or sound needs `<track kind="captions">`.

## SEO
Every view: `<meta seo title="…" description="…" canonical? og:image? schema? robots?>`.
Values may be expressions from `load` props: `<meta seo title={props.user.name} description={props.user.bio}
jsonld={{ '@type': 'Person', name: props.user.name }}>`. Site defaults live in `app.brain`:
`<meta seo titleTemplate="%s · Nimbus" og:image="/og.png" twitter="@nimbus">`. Any other tag goes in a `<head>` block of
the view (merged, deduplicated by name/property). `[id]` views list their pages with
`export async function paths() { return [{ id: '1' }] }` so they are prerendered and appear in the sitemap.
Live `[id]` views (SSR, no `paths()`) list their pages with `export async function sitemap({ env }) { return [{ id: '1',
lastmod: '2026-10-01' }] }`: the server adds them to /sitemap.xml on each request (cached 5 min; needs `brain.json` `site`; past 50 000 URLs it splits into an index). Not with `paths()` in the same view (B0117).
Sitemap date: `<meta seo lastmod="2026-10-13">` or the collection's frontmatter `date`; without one it is omitted.
Collections: `brain add collection blog` turns `content/blog/*.md` (frontmatter `title description date updated? tags? cover?
author? draft?`) into pages, tag pages, `/blog/rss.xml` and `/blog/atom.xml`. In `load()`/`paths()`, with no import:
`entries('blog')` `entry('blog', slug)` `tags('blog')` `tagged('blog', tag)` `pages('blog')` `page('blog', n)`; an entry has
`slug url title description date tags html`. Never on the client (B0705).
Generated by `brain build`: robots.txt, sitemap.xml, llms.txt, llms-full.txt, manifest.webmanifest. Google tags come from
`brain.json` (`"google": { "analytics": "G-…", "tagManager": "GTM-…", "siteVerification": "…" }`), never hand-written.
Social profiles go in `brain.json` `"social": { "x": "@acme", "instagram": "…", "linkedin": "…" }`; the build writes
every Open Graph/Twitter tag that is missing. `"seo": { "ai": "allow" | "search-only" | "block" }` sets the AI crawler
policy. Never hand-write `og:*` defaults, robots.txt or llms.txt.

## Agent tools (WebMCP)
Expose what a page can do to browser AI agents. Declarative: `<form tool="search_products" tool-description="Search the
catalog">` (each field's `<label>` becomes its description). Tool names are snake_case and unique per page.
<!--planned:#106-->
Imperative, in a component: `<meta tools="add_task: Add a task (text:str!)">` + a function of the same name; it is
registered while the component is mounted.
<!--/planned-->

## Built-in ui/ kit (use before writing your own; no import needed)
`<Button>` `<Input>` `<Select>` `<Checkbox>` `<Dialog>` `<Table>` `<Tabs>` `<Toast>` `<Menu>` `<Field>` — accessible,
token-styled, every state (loading, disabled, error, empty) covered.
Forms: `let f = form(schema, { submit: async (v) => … })` → `bind:value={f.v.email}`, `f.err.email`, `f.busy`,
`f.error` (form-level), `on:submit={f.submit}`. Schema: `{ email: 'email!', age: 'num>=18', name: 'str!<=80' }`
(`!` required; types `str num bool email url date`, `[T]` list, a nested type object). Server errors: return or
throw `{ errors: { email: 'Taken' } }` from `submit`, or mark them later with `f.errors({ email: 'Taken' })`; any other
failure shows in `f.error`. Initial values:
`form(schema, { init: { email: '' } })`; values that arrive later (edit forms): `f.reset(values)`. Never replace
`f.v` or `f.err` (B0133: the inputs would stay bound to the old fields). Validation messages are English unless
`i18n/<locale>.json` has `form.required`, `form.min_chars`, `form.max_chars`, `form.min_items`, `form.max_items`,
`form.item`, `form.list`, `form.number`, `form.min`, `form.max`, `form.email`, `form.url`, `form.date`,
`form.error`, `form.check_failed` (`{n}` = the limit, except in `form.item`: `{n}` = the 1-based item number and
`{error}` = that item's message; plural objects allowed). A server function that throws a plain `Error('msg')`
answers 500 and the client sees a generic message (the real one is only in the server log): to show a reason, throw
`{ errors: { field: 'msg' } }` (422, the message reaches the field).

## Data
`api/*.jsbrain`: `export default api('/users', { get: { out: '[User]' }, post: { in: 'User', out: 'User' } })` and
`types.jsbrain`: `export const User = { id: 'num!', email: 'email!' }`. Use `let users = resource(usersApi.get)` →
`{#await users}…{:then list}…{:catch e}…{/await}` must cover loading, error and empty. Responses are validated.
`brain run --mock` serves fake data from the schemas.

## Caching (automatic)
`brain build` caches everything for the repeat visit: immutable hashed assets, a service worker (visited pages work
offline; `views/_offline.brain` is optional) and an in-memory `resource()` cache. Never write a service worker or
cache headers yourself. Opt out in `brain.json`: `"cache": { "sw": false }` (also `http`, `offline`, `data`).
Keep data across visits: `api('/posts', { get: { out: '[Post]', cache: '10m' } })` (never for personal data) ·
`store(init, { persist: true })`. Heavy work off the main thread: name the module `*.worker.jsbrain`; its exports
become async functions.

## Errors at runtime
Errors in `fx`, `drv` and handlers are caught: the app stays alive and the error is reported. An effect that re-runs
more than 100 times in one update is stopped (B0902): never write a signal inside an `fx` that reads it.
Per component fallback: `<meta fallback="Could not load">` or `{:catch}`.
Every component is isolated automatically: if it fails, only it shows a fallback with Retry and the rest of the page
keeps working; a failing click handler keeps the component and shows a toast. Do not write try/catch for that.
A component that keeps growing (DOM nodes, list rows, effect runs) or loops is stopped and replaced by its fallback;
`brain build` mounts every view with fake timers and fails (B0907) if anything grows with no user input. Never append
to a list from a timer, interval, bus or `fx` without a limit; create timers only inside `fx` and return their cleanup.
Captured errors (message, stack, recent actions) are listed by `brain_errors` (MCP) and `brain doctor`.

## Server and secrets
`load` runs only on the server. Client code may only read env vars prefixed `PUBLIC_`.
Variables live in `.brain_env` (already created): shared `KEY=value` lines, then `[development]` `[staging]`
`[production]` sections that override them. `brain run` uses development, `brain build` production, `--env x` another.
Read them as `process.env.KEY` (server) or `process.env.PUBLIC_KEY` (anywhere). Never create a `.env`.
`.jsbrain` files starting with `<meta server>` run only on the server and may be used only inside `load` or server
functions. Server function in a `.brain`: `server: async function save(ctx, data) {…}`; the client calls
`await save(data)` (no `ctx`). Validate `data` inside.
Sessions: `ctx.session.set(user)` in a server function signs it into the `__Host-session` cookie (HttpOnly, Secure,
SameSite=Lax, 7 days; `{ maxAge: seconds }` to change it; needs `BRAIN_SECRET` in `.brain_env`); every later request
gets it as `ctx.user` (guards, `load`, server functions). `ctx.session.clear()` logs out. The cookie is signed, not
encrypted: anyone holding it can read it, so store ids and display data, never tokens or secrets.
`load` belongs to views: in `_layout.brain` it is not run.

## Tests (`*.test.brain`)
`test(name, async () => {…})` · `await mount(Comp, props)` · `await click(sel)` · `await type(sel, text)` ·
`text(sel)` `attr(sel, n)` `hasClass(sel, c)` `count(sel)` · `expect(x).toBe(y)|toEqual(y)|toBeTruthy()`.

## CLI (also MCP: `brain mcp`; every command accepts `--json`)
Main: `brain new <app>` · `brain run` (dev server with hot reload, app :5180; views render in the browser, so judge
SEO and the no-JS HTML on `brain build` + `brain start`) · `brain build` (production) ·
`brain doctor` (everything at once: check + tests + over-engineering + leaks; exits non-zero on any error).
Production: `brain start` (serves `dist/`, restarts on crash, logs to `logs/app.log`) · `brain reload` (no downtime) ·
`brain status` · `brain stop`. Never add PM2 or another process manager.
Hosted under a sub-path: `brain.json` `"base": "/app/"`; keep writing app-absolute links and paths (`<a href="/about">`,
`redirect('/login')`); the build adds the base everywhere.
Granular: `brain add view|comp|ui <name> --props "…"` · `brain check --json` · `brain test` · `brain spec [--topic x]`.
Also: `brain preview` (temporary loopback server for `dist/`, Unix) · `brain explain <code>` ·
`brain doctor --fix` (apply mechanical fixes, then run the full doctor). Config: `brain.json`.
<!--planned:#68-->
Unified exit codes across all commands: 0 ok · 1 project errors · 2 usage · 3 internal.
<!--/planned-->
`brain fix` applies every `edits` from `brain check` and re-checks (run it before editing by hand).
`brain map --json`: whole project graph (routes, components, props, channels, tests); read it instead of opening files.
`brain shot <route>` (Unix + Chrome): PNG and accessibility outline for mobile/desktop. Use `--viewport mobile|desktop|both`,
`--text` for the outline, `--baseline <name>` to compare, and explicitly add `--update-baseline` to approve a new image.
`--max-diff <0..1>` sets the allowed changed-pixel fraction (default 0). A mismatch or accessibility issue fails the check.
`brain.json` `{"budget":{"viewKB":30}}` fails the build when a route exceeds 30 KiB of emitted JS + CSS + HTML.
This conservative total includes deferred JS dependencies; it is not a measurement of initial network transfer.
<!--planned:#40-->
CLI `brain undo` and version-to-version `brain migrate`; runtime budgets `mountMs` (p95) and `heapMB`.
<!--/planned-->
`brain edit add-prop <file> <declaration>` · `remove-prop <file> <prop>` · `rename <file> <NewName>` ·
`move <file> <destination>` · `extract <file> <selector> <NewName>` (AST edits; prefer over text edits).
Rename changes a component filename and its resolved imports, preserving local aliases; it does not rename JavaScript
bindings. Changes pass the checker before writing. Unsupported ambiguous references are rejected.
Prefer composing over writing: `brain gen --list --json` shows recipes (app shell, navbar, auth, CRUD from an api schema,
dashboard, settings, pricing, landing sections, wizard, modal); `brain gen <recipe> --params '{…}'` creates files that
already pass `brain doctor`. Layout without CSS: `<Stack gap="3">` `<Row>` `<Grid cols="3">` `<Center>` `<Sidebar>`
`<Container>`. Multi-file changes: `brain apply plan.json` (all or nothing). Read cheaply: `brain read <file> --outline`,
`brain map --focus <file>`, `brain context "<task>" --budget 8000`, `brain shot --text`, `brain diff --semantic`.
`brain build` is production: minifies, mangles, obfuscates and removes dead code and `log()`. No flags needed.
It also adds SRI and a strict CSP with the security headers to every page.

## Loop
1 `brain add` · 2 edit · 3 `brain fix` then `brain check --json` → apply each remaining `fix` literally, repeat until `[]` ·
4 `brain doctor`.
Error: `{"code","file","line","col","msg","fix","edits"}`; `edits` (when present) are exact replacements you may apply as-is. Families: B00 structure · B01 script · B02 markup · B03 css ·
B04 a11y · B05 seo · B07 server/secrets.

## Reference component
```html
<meta props="label:str='Tasks'">
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
