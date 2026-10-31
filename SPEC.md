# BRAINLLM SPEC
Normative. Anything not here does not exist. Internals: docs/INTERNALS.md (not needed to write apps).
Before each kind of task run `brain guide <topic>` (`brain guide` lists them): steps, checks, mistakes, example.

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
  text once there are 2+ locales. Add a language with `brain add locale es`: it moves every fixed text (text, visible
  attributes, `<meta seo>` title and description, literals in markup expressions) to keys named
  `<view>.<first_words>`, then translate that file.
  `fmt.date/number/money(v)` format for the current locale; `<LangSwitch>` links to the other languages.
  Content per language: `content/blog/es/*.md`; a missing translation shows the default text and `post.fallback`
  is true, so the template can say so.
- HTML first: use real `<a href>` and `<form>` (they work before JS loads and are enhanced after). Only a component
  that cannot render on the server (canvas, map) gets `<meta client>`.
- `lib/*.jsbrain` logic, no markup. `tokens.cssbrain` design tokens (root).
- `brain new` already creates the whole layout: `views/ comp/ ui/ channels/ services/ api/ lib/ server/ content/
  public/`, `brain.json`, `.brain_env`. Put each file in its folder (each holds a README saying what goes there);
  never create other top-level folders.
- Import only files that exist (`brain map --json` lists them; B0119). A store, helper or server module the app does
  not have yet is declared in the file you are writing, or created first with `brain add`. Primitives need no import.
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
| `export async function load(ctx)` | views only, runs on server; result arrives as `props`. `throw notFound()` → 404 with `views/404.brain`; `throw gone()` → 410 (removed for good; `views/410.brain`, else the 404 view); `throw redirect('/x')` → 303, `redirect('/new-slug', 301)` for a moved record |
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
runs only when `q` changes (`{ immediate: true }` as third argument also runs it once with the initial value) · `await tick()` after DOM updates · `every('30s', fn)` timer stopped on unmount ·
`media('(max-width: 600px)')` `visible(ref)` `debounce(v, '300ms')` `online()` are read-only signals, read like any other ·
`<component is={C}>` · `<Portal>` · `transition:fade` on elements inside `{#if}`/`{#each}` ·
`ai` is Chrome's on-device model when present (`brain guide ai`).
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
Every view: `<meta seo keyword="…" title="…" description="…" canonical? og:image? schema? robots?>`.
Pick the query first; put it in the title, H1, first sentence and description. `keyword` is never rendered; an
indexable view without it is B0502 (guarded, `robots="noindex"`, `_x`, `[id]` and error views are exempt).
Values may be expressions from `load` props: `<meta seo title={props.user.name} description={props.user.bio}
jsonld={{ '@type': 'Person', name: props.user.name }}>`. Site defaults live in `app.brain`:
`<meta seo titleTemplate="%s · Nimbus" og:image="/og.png" twitter="@nimbus">`. Any other tag goes in a `<head>` block of
the view (merged, deduplicated by name/property). `[id]` views list their pages with
`export async function paths() { return [{ id: '1' }] }` so they are prerendered and appear in the sitemap.
Live `[slug]` views (SSR, no `paths()`) export `sitemap({ env })` instead, returning `[{ slug, lastmod? }]` (key = the
bracket name; `lastmod` a `YYYY-MM-DD` string or Date; an item without the key is skipped). `brain guide seo`; never both, B0117.
A public dynamic view (no guard, not `noindex`) with neither export is listed nowhere: `brain build` warns B1837 with the route.
Sitemaps need the deployed origin (`PUBLIC_SITE_URL` in `.brain_env`): with none, or a placeholder (example.com, `*.test`,
localhost), the build publishes no sitemap and warns B0736.
Sitemap date: `<meta seo lastmod="2026-10-13">` or the collection's frontmatter `date`; without one it is omitted.
Collections: `brain add collection blog` turns `content/blog/*.md` (frontmatter `title description date updated? tags? cover?
author? draft?`) into pages, tag pages, `/blog/rss.xml` and `/blog/atom.xml`. In `load()`/`paths()`, with no import:
`entries('blog')` `entry('blog', slug)` `tags('blog')` `tagged('blog', tag)` `pages('blog')` `page('blog', n)`; an entry has
`slug url title description date tags html`. Never on the client (B0705).
Generated by `brain build`: robots.txt, sitemap.xml, llms.txt, llms-full.txt, manifest.webmanifest, rss.xml (site feed;
`<section>/rss.xml` per section) and sections.json (every section with its pages and feeds; `seo.feed` / `seo.sections`
false turn them off). Every page names its generator, home and source. Google tags come from
`brain.json` (`"google": { "analytics": "G-…", "tagManager": "GTM-…", "siteVerification": "…" }`), never hand-written.
Social profiles go in `brain.json` `"social": { "x": "@acme", "instagram": "…", "linkedin": "…" }`; the build writes
every Open Graph/Twitter tag that is missing. `"seo": { "ai": "allow" | "search-only" | "block" }` sets the AI crawler
policy. Never hand-write `og:*` defaults, robots.txt or llms.txt.

## Agent tools (WebMCP)
Expose what a page can do to browser AI agents. Declarative: `<form tool="search_products" tool-description="Search the
catalog">` (each field's `<label>` becomes its description). Tool names are snake_case and unique per page.
Imperative, in a component: `<meta tools="add_task: Add a task (text:str!, due:date)">` + a function of the same name
that destructures exactly those parameters (`const add_task = async ({ text, due }) => …`; types `str num bool list obj
email url date any`, `!` required; tools separated by `;`). It is registered while the component is mounted; its return
value (or thrown message) is the agent's answer, and invalid arguments never reach it.

## Built-in ui/ kit (use before writing your own; no import needed)
`<Button>` `<Input>` `<Select>` `<Checkbox>` `<Dialog>` `<Table>` `<Tabs>` `<Toast>` `<Menu>` `<Field>` — accessible,
token-styled, every state (loading, disabled, error, empty) covered.
Forms: `let f = form(schema, { submit: async (v) => … })` → `bind:value={f.v.email}`, `f.err.email`, `f.busy`,
`f.error` (form-level), `on:submit={f.submit}`. Schema: `{ email: 'email!', age: 'num>=18', name: 'str!<=80' }`
(`!` required; types `str num bool email url date`, `[T]` list, a nested type object). Server errors: return or
throw `{ errors: { email: 'Taken' } }` from `submit`, or mark them later with `f.errors({ email: 'Taken' })`; any other
failure shows in `f.error`. Initial values: `form(schema, { init })`; later values: `f.reset(values)`; never replace
`f.v` or `f.err` (B0133). Translated messages, edit forms and server errors: `brain guide forms`.

## Data
`api/*.jsbrain`: `export default api('/users', { get: { out: '[User]' }, post: { in: 'User', out: 'User' } })` and
`types.jsbrain`: `export const User = { id: 'num!', email: 'email!' }`. Use `let users = resource(usersApi.get)` →
`{#await users}…{:then list}…{:catch e}…{/await}` must cover loading, error and empty. Responses are validated.
`brain run --mock` serves fake data from the schemas. Use GET `cache: 'no'` for an endpoint that must never use the data cache.

Native live invalidation: `const updates = liveResource(rows, {rooms:[{resource:'posts'}], scope:()=> session.user ? session.user.id : null})`
passes an existing resource handle; `updates.state` / `updates.connected` are reactive getters. Optional room `id` is a UUID;
`{room:'user:self'}` subscribes only to the authenticated owner. The server requires explicit brain.json
`backend:{origins:['https://app.example.com'],sockets:{allowedOrigins:['https://app.example.com']}}`; socket origins must
be a subset of HTTP origins (1–8 canonical origins). Absent/false sockets disables it. It connects only to the app's
`<base>/_brain/socket`, sends no credentials in the URL, refetches after JOIN acknowledgement/reconnect/invalidation,
and closes on unmount. Scope is a public user/revision key or null, never a token; null disconnects. HttpOnly cookie
changes are not detected: call `updates.reconnect()` after same-scope login/session rotation and clear scope on logout.
`updates.close()` stops it. Metadata observers `onInvalidate`/`onResync` are synchronous, coalesced and receive no row
bodies. Native backend policy remains authoritative; this ephemeral single-process channel is not a durable queue.

## Project Design System and agent feedback
New projects include `design.brain.json`, project-owned tokens and extendible `ui/` basics. Before UI work, inspect
`brain design blueprint --json`, `brain design query --json` and `brain manifest query --json`. Use this project's
components/tokens throughout the app: no external CSS frameworks. Build mobile first, then tablet/desktop; preserve
safe areas, readable input text, touch targets, focus, reduced motion and all interactive states.
`brain design seed|configure --request <JSON>` prepares guarded source operations; apply them with `brain changes`.
`brain design check --json` reports drift; `brain design gallery --json` prepares searchable project UI documentation.
B2201 warns when one exact project token already represents a component style value; review its cascade before editing.
Existing projects use `brain design init` and `brain upgrade <exact-version>` previews to adopt without deleting old data.
`brain quality refresh --json` persists actionable JSON, an improvement file and local panel. `brain feedback query|watch`
reads compiler events published before the browser overlay; watch is finite NDJSON. MCP exposes the corresponding tools.
`brain history` records project-local tasks, evidence and proposed lessons with revisions; reported evidence is not proof.

## Caching (automatic)
`brain build` caches everything for the repeat visit: immutable hashed assets, a service worker (visited pages work
offline; `views/_offline.brain` is optional) and an in-memory `resource()` cache. Never write a service worker or
cache headers yourself. Opt out in `brain.json`: `"cache": { "sw": false }` (also `http`, `offline`, `data`).
Every response is compressed for you (zstd, Brotli or gzip by `Accept-Encoding`; precompressed assets, per-response
HTML and JSON); never compress bodies or set `Content-Encoding` in app code (docs/PERFORMANCE.md).
Keep data across visits: `api('/posts', { get: { out: '[Post]', cache: '10m' } })` (never for personal data) ·
`store(init, { persist: true })`. Heavy work off the main thread: name the module `*.worker.jsbrain`; its exports
become async functions. Their native bridge bounds pending work, deadlines, bytes and cancellation.
Public navigable state: `urlState('overview', {key:'tab'})`; selections and native Dialog integrate with query history.
`{key:'tab', values:['overview','details']}` limits it: an unknown query value reads as the first allowed value; `<Tabs>`
with an unknown value selects its first tab.
Preserve unrelated query parameters; never serialize credentials or private form input. Use distinct keys in repeated UI.
Structured local cache: `database('navigation', {namespace:'my-app',build:'v1'})` supports get/put/batch/scan/prune/close
with native IndexedDB, quotas, expiry and atomic batches. `storage()` and `cookie()` are small public preferences.
`storageInfo()` reports capabilities; `persistStorage()` requests persistence from a user action. Storage remains evictable.
Optional `sqlite(officialInitializer, options)` runs official SQLite Wasm in an owned OPFS worker; it is not a core dependency.
`threadCapabilities`, `sharedWorker`, `messageChannel`, `transfer`, `offscreen`, `sharedMemory` and `loadWorklet` expose
bounded native thread operations and explicit unsupported-capability errors. See `docs/BROWSER-DATA.md` for contracts.

## Errors at runtime
Errors in `fx`, `drv` and handlers are caught: the app stays alive and the error is reported. An effect that re-runs
more than 100 times in one update is stopped (B0902): never write a signal inside an `fx` that reads it.
Per component fallback: `<meta fallback="Could not load">` or `{:catch}`.
Every component is isolated automatically: if it fails, only it shows a fallback with Retry and the rest of the page
keeps working; a failing click handler keeps the component and shows a toast. Do not write try/catch for that.
A component that keeps growing (DOM nodes, list rows, effect runs) or loops is stopped and replaced by its fallback;
`brain build` mounts every view with fake timers and fails (B0907) if anything grows with no user input. Never append
to a list from a timer, interval, bus or `fx` without a limit; create timers only inside `fx` and return their cleanup.
Public production messages contain safe references, never raw traces, arbitrary exception text or internal paths.
Private captured errors (message, stack, recent actions) are listed by `brain_errors` (MCP) and `brain doctor`.

## Server and secrets
`load` runs only on the server. Client code may only read env vars prefixed `PUBLIC_`.
Variables live in `.brain_env` (already created): shared `KEY=value` lines, then `[development]` `[staging]`
`[production]` sections that override them. `brain run` uses development, `brain build` production, `--env x` another.
Read them as `process.env.KEY` (server) or `process.env.PUBLIC_KEY` (anywhere). Never create a `.env`.
`.jsbrain` files starting with `<meta server>` run only on the server and may be used only inside `load` or server
functions. Server function in a `.brain`: `server: async function save(ctx, data) {…}`; the client calls
`await save(data)` (no `ctx`). Validate `data` inside. `throw { message: 'Title already used' }` (400) or an Error
subclass with a 4xx `status` reaches the calling form's `f.error` as is; other exceptions stay private.
Server-only modules may signal a deliberate client failure with `throw {status:403}` (standard 4xx only); RPC returns
the status with a generic public message. Arbitrary exception messages remain private and are excluded from server logs too.
Sessions: `ctx.session.set(user)` in a server function, `ctx.user` on later requests, `ctx.session.clear()` logs out
(signed cookie, not encrypted: ids and display data only; setup in `brain guide server`).
`load` belongs to views: in `_layout.brain` it is not run.
Server limits (dev and production alike): brain.json `"server": { "maxBodyKB": 1024, "timeoutMs": 10000,
"rpcPerMinute": 60 }` bounds RPC/form bodies (413 past it, at most 65536), function calls and `load` (504) and calls
per IP and function (429). Raise `maxBodyKB` to send a file as base64 in an RPC argument; invalid values are B0707.

### Experimental Rust/Postgres backend
First read `brain backend schema --json` (resolved resource/job contracts + resource OpenAPI); inspect one with
`brain card resource:posts` / `brain card job:notify`. Generate with `brain add resource posts --fields "title:str! body:str"`
or `brain add job notify --fields "message:str!"`; fields: `str num bool email obj`, `!` required. Resources have explicit
`schema/policy/hooks.jsbrain`; jobs have `schema/handler.jsbrain` under `server/resources/` and `server/jobs/`.
Resource generation also creates collection/detail clients under `api/`, an executable native repository and
server service, and `client-contract.json` pointing to the canonical schema revision. Add `--admin-ui` when creating
a resource to generate project-owned `/admin/<resource>` list/search, new, detail and edit/delete routes plus an
administrator guard and missing project DS controls. These files and the generated contract commit together through
validated file leases and compare-and-swap revisions. Existing files are never overwritten; without this flag no
routes or controls are created. Native backend policy remains authoritative. Review policy, migrate explicitly,
then build and doctor the application. The app's default 30 KiB route budget remains unchanged.
Set `DATABASE_URL`; explicitly `brain backend migrate`, then `brain backend serve` (loopback REST at `/api/<resource>`).
`PATCH /api/<resource>/<uuid>` changes only the sent fields (`null` clears an optional one); see `docs/backend/patch.md`.
`server/operations/<name>.jsbrain` declares typed input and steps that `POST /_brain/operations/<name>` runs in one transaction (all or nothing); see `docs/backend/transactions.md`.
`actions.jsbrain` declares guarded atomic writes (`when`, `set`, `increment`, `decrement`) at `POST /api/<r>/<uuid>/actions/<name>`; `POST /api/<r>/upsert` inserts if absent on a unique key; see `docs/backend/atomic-updates.md`.
`POST /api/<r>/batch` (`op`: create|update|upsert|delete, `items`) is all-or-nothing, max from `batch.jsbrain { max }`; see `docs/backend/batch.md`.
`transitions.jsbrain { field, initial, transitions }` makes an `enum` field a state machine changed only by `POST /api/<r>/<uuid>/transitions/<name>`; see `docs/backend/state-machines.md`.
`server/jobs/<name>/queue.jsbrain { priority, attempts, backoff, timeout, lease, drain, concurrency }` tunes one queue; `brain jobs dead|retry|stats <queue>`; see `docs/backend/queues.md`.
`server/cron/<name>.jsbrain { schedule: '0 7 * * *', job, payload?, catchUp?, priority? }` enqueues once per slot (UTC); run `brain jobs cron`; see `docs/backend/cron.md`.
`brain jobs enqueue|status|work|prune` manages persistent, at-least-once work. Handlers must be idempotent by `ctx.jobId`.
After-write hooks commit `{resource,operation,record}` with their mutation. `brain add hook validate_order` creates
`server/hooks/validate_order.jsbrain`; reference it as `before_create`, `before_update` or `before_delete` in a resource's
hooks contract. Its default function receives `{resource,operation,payload,existing,user,rid}` and must return the
accepted payload; the backend revalidates it and rejects conflicting writes. See `docs/BEFORE-HOOKS.md` for limits.
See `brain guide server` for the workflow.
Behavior analytics is on by default with a native backend: the served page loads `/_brain/analytics.js` (no build
bytes), events go to the project's Postgres, and `brain analytics report [--since 7d] [--json]` resolves elements to
`file:line`. Mark custom events with `data-track="name"`; turn it off with `"analytics": false` or `BRAIN_ANALYTICS=off`.
See `docs/backend/analytics.md`.

`ref:<resource>` fields keep a foreign key (`on_delete: restrict|cascade|set_null`; missing id → 422 `REFERENCE_NOT_FOUND`,
referenced on restrict delete → 409 `REFERENCE_IN_USE`). Optional `query.jsbrain` declares list filters (`?f=v`, `?f[op]=v`,
`now-7d`), `sort` (`?sort=-price,title`, ties by id), `include` (one level), named `aggregate`s and related `count`s;
undeclared or repeated parameters → 422. Lists return `total` and `next_cursor` (pass back as `cursor` or `after`).
Policy grants `owner.<ref>[.<ref>]` scope rows through relations; field option `visible: ['owner','admin']` hides
a field from other readers and from the query surface. Modifiers `'str writeonly'`, `readonly`, `private` control
client writes and reads. `owner` is returned only to owners and named read roles unless `expose_owner: true`.
See `docs/backend/*.md`.
Set `BRAIN_BACKEND_URL` to the fixed loopback native backend URL to connect native API requests and SSR session verification.
Native JWT accounts use short-lived access and rotating refresh cookies, persisted revocation, configured roles and Google Sign-In.
`brain recipe auth-ui|password-reset|email-verification|google-signin` prepares project-owned account UI;
mail needs private `BRAIN_MAIL_RESEND_KEY`, `BRAIN_MAIL_FROM` and `PUBLIC_SITE_URL` (HTTPS, optional hosting prefix).
`brain backend bootstrap-admin --user <existing-uuid>` explicitly assigns the first administrator; signup never grants it.
API keys: add `server/api-keys.jsbrain` `{enabled:true}`; signed-in users manage keys at `/_brain/keys` (secret shown once), callers send `X-API-Key: bk_live_...` to `/api/<resource>` only; scopes `posts:read|write|*` never exceed the owner. See `docs/backend/api-keys.md`.
See `docs/AUTH-FLOWS.md`; mail responses deliberately do not disclose account existence or delivery failures.
In a same-origin POST server function, `await ctx.backend.login({email,password})` establishes native JWT or opaque
HttpOnly cookies and returns the sanitized user; the backend chooses the configured mode. Never expose signing keys
or copy tokens into browser storage. `ctx.session.clear()` revokes native sessions. See `docs/NATIVE-AUTH-SSR.md`.
With native mode configured, server `load(ctx)` and server functions can use `ctx.backend.resource('posts')`:
`list({filter:{price:{gte:'10.00'},category:{in:['a','b']}},sort:['-price'],limit,after,include,search})` (same contract
as `GET /api/<name>`; undeclared filters reject with `error.status 422` and `error.errors`), `get(uuid,{include})`,
`create(values)`, `update(uuid,values)`, `patch(uuid,partial)`, `remove(uuid)`; `ctx.backend.slugRedirect(name,slug)` → current
slug or `null`. Records carry `created_at`/`updated_at`. See `docs/backend/server-queries.md`.
`query.jsbrain` may set `search: { language: 'english', weights: { body: 'A' } }` (built-in Postgres configurations,
weights A-D on searchable fields; `docs/backend/search.md`). Record ids are UUIDs; keep old numbers as a unique field
(`docs/backend/ids.md`).
Job handlers get the same `ctx.backend` when the worker has `BRAIN_BACKEND_URL` and `BRAIN_JOBS_API_KEY` (a scoped
project API key; it acts as its owner under each policy). See `docs/backend/job-backend.md`.
Reads and writes run through the same native schemas, policies and authenticated cookies, not a second database client.
Writes require a same-origin POST server function; they are rejected from GET/load. Put domain logic in server-only
service/repository `.jsbrain` modules and pass `ctx` explicitly. Request JSON is limited to 64 KiB, responses to 2 MiB,
with a shared eight-request cap and ten-second total deadline; no automatic mutation retries.
Files of any type: `server/files/invoices.jsbrain` `export default {types:['application/pdf'],maxBytes:'10mb',create:['user']}`
→ multipart `POST /_brain/files/invoices` (field `file`), `GET /_brain/files/invoices/<id>` (Range, ETag). Type comes
from content, not the name; empty `types` accepts any type. See `docs/backend/files.md`.
Edge: `server/platform.jsbrain` `export default {trustProxy:['127.0.0.1'],cors:{origins:[...],maxAge:600},limits:{maxInFlight:256},shutdown:{readyDelay:'5s',drain:'25s'},logging:{requests:'errors'|'all'|'off',slow:'1s'}}`.
Probes `GET /health/live` and `GET /health/ready`; overload answers `503 OVERLOADED` + `Retry-After`. Production
requires `sslmode=verify-full` for a remote `DATABASE_URL`. See `docs/backend/proxy-cors.md`, `docs/backend/lifecycle.md`.
Several processes: `cluster:{shared:true}` in `server/platform.jsbrain` (automatic when `BRAIN_BACKEND_PROCESSES>1`) keeps route/API-key rate windows in Postgres and relays socket events between processes. See `docs/backend/cluster.md`.
Caching: API GETs send `ETag` (304 on `If-None-Match`); `server/cache.jsbrain` `export default {products:{maxAge:'1m',staleWhileRevalidate:'10m'}}` sets a public `Cache-Control` for anonymous reads of public-read resources; `If-Match` on PUT/PATCH/DELETE → 412 `PRECONDITION_FAILED` when stale. See `docs/backend/caching.md`.
Routes: `server/routes/<name>/route.jsbrain` `export default {method:'POST',path:'/orders/{id}/quote',auth:'user',params:{id:'uuid'},query:{currency:'str!'},bodyLimit:'16kb',timeout:'5s',rate:'20/min per user'}`
+ `handler.jsbrain` `export default async (ctx) => ({status:201,body:{...}})`; throw `{errors:{field:'msg'}}` → 422.
Signed: add `body:'raw',verify:{scheme:'timestamped'|'hmac',header,secret:'ENV_NAME',tolerance:'5m'}`. See `docs/backend/routes.md`, `docs/backend/webhooks.md`.
Outbound: `await httpCall(url,{method:'POST',json,idempotencyKey:true,timeout:8000,retries:2})` → `{status,ok,json,text}`; throws `HttpCallError` (`TIMEOUT`, `HTTP_STATUS`, `UNSAFE_URL`, `REDIRECT`…). Every resolved address is checked: private, loopback, link-local, CGNAT and multicast targets are refused unless `allowPrivate` names the host; `redirects:n` follows n hops, each checked again. See `docs/backend/outbound.md`.
Keep existing production routes until their behavior and policies pass a tested migration slice.

## Tests (`*.test.brain`)
`test(name, async () => {…})` · `await mount(Comp, props, { slot, slots, provide }?)` (`slot`: default slot text;
`slots: { name: text }`: named slots) · `await click(sel)` · `await type(sel, text)` ·
`text(sel)` `attr(sel, n)` `hasClass(sel, c)` `count(sel)` · `expect(x).toBe(y)|toEqual(y)|toBeTruthy()`.

## CLI (also MCP: `brain mcp`; every command accepts `--json`)
Main: `brain new <app>` · `brain run` (dev server with hot reload, app :5180; views render in the browser, so judge
SEO and the no-JS HTML on `brain build` + `brain start`) · `brain build` (production) ·
`brain doctor` (everything at once: check + tests + over-engineering + leaks; exits non-zero on any error).
Production: `brain start` (serves `dist/`, restarts on crash, logs to `logs/app.log`) · `brain reload` (no downtime) ·
`brain status` · `brain stop`. Optional PM2/nginx configurations: `brain deploy config pm2|nginx --request <JSON>`; see `docs/DEPLOYMENT.md`.
Hosted under a sub-path: `brain.json` `"base": "/app/"`; keep writing app-absolute links and paths (`<a href="/about">`,
`redirect('/login')`); the build adds the base everywhere.
Granular: `brain add view|comp|ui <name> --props "…"` · `brain check --json` · `brain test` · `brain spec [--topic x]` ·
`brain guide [topic]`.
Also: `brain preview` (temporary loopback server for `dist/`, Unix) · `brain explain <code>` ·
`brain doctor --fix` (apply mechanical fixes, then run the full doctor). Config: `brain.json`.
<!--planned:#68-->
Unified exit codes across all commands: 0 ok · 1 project errors · 2 usage · 3 internal.
<!--/planned-->
`brain fix` applies every `edits` from `brain check` and re-checks (run it before editing by hand).
`brain map --json`: whole project graph (routes, components, props, channels, tests); read it instead of opening files.
`brain docs [--check] [--json] [--out docs/app]`: generated app docs (routes, components, data, backend, env keys, SEO); `brain build` refreshes them when `docs/app/index.md` exists or `brain.json` has `"docs": true`; prose goes in `docs/app/notes/<page>.md`.
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
`brain recipe list` lists experimental `resource-ui` (existing resource contract) and `auth-ui` previews; pass recipe
params as a JSON argument. It returns operations without writing. `brain changes read|lease|plan|apply|recover <json>`
coordinates per-file leases and revision checks, validates a private snapshot, and supports conditional recovery.
Each file replacement is atomic; a multi-file change is a recoverable logical transaction. Unrelated editors can race.
`brain graph` gives bounded static dependency evidence; `brain context "<task>"`, `brain why <target>` and
`brain preflight` expose focused context and preventive checker diagnostics. Dynamic unknowns remain explicit.
MCP full profile: `brain_changes`, `brain_intelligence`, `brain_recipe`. See project developer docs for JSON contracts.
`brain upgrade <exact-version>` previews package pin operations and unresolved migration/install decisions; it does not
verify the registry, install packages or run source migrations. Apply reviewed operations with the changes protocol.
`interop(() => element, adapter, props, {onError})` owns an external SDK/DOM mount during component setup; adapters
return cleanup or `{update, destroy}` synchronously. Updates are explicit; cleanup runs on unmount. SDKs stay app-owned.
Browser dependencies may use installed ESM packages resolved from exact package metadata/lock evidence. CommonJS,
TypeScript/JSX source and unknown dynamic graphs are rejected explicitly; use a reviewed ESM distribution.
`brain deps analyze --json` reports actual graph/bytes, side effects and unknowns. `brain deps preview --request <JSON>`
validates an agent source proposal against its revision before guarded application. Explicit `brain deps fetch --request <JSON>` acquires one exact pinned HTTPS CDN ESM artifact with SHA-256 and lock CAS. Production builds resolve this
verified local vendor lock offline; they never fetch mutable URLs or execute an LLM to rewrite dependencies.
Worker/SDK URL resources, including reached Wasm assets, retain content-addressed paths and required sibling layout.
Use `brain integration list` and `brain integration add|update <provider> --version <exact>` for official OpenAI, Claude,
Gemini and Maps SDK adapter previews; backend SDK credentials stay private and frontend callers use server functions.
See `docs/DEPENDENCIES.md` for the supported ESM/CDN subset and explicit graph limits.
`brain build` is production: minifies, mangles, obfuscates and removes dead code and `log()`. No flags needed.
It also adds SRI and a strict CSP with the security headers to every page.

Native helpers need no imports or packages: `dateParse/dateAdd/dateDiff/dateRange/dateZone` use UTC arithmetic and
explicit timezone projection; `hashText/shortHash` are async Web Crypto. `brain lib list` (MCP `brain_lib`) gives the
catalog and limits; `brain lib new <name>` atomically creates a pure named-export project helper. Named functions use
existing tree shaking; inspect emitted bytes in `brain build --json` → `ledger`.
Native type findings B2001–B2005 and browser-target findings B2101–B2103 are warnings. Unknown flows remain unknown.
`brain.json` `targets` selects `baseline-widely` (default), `baseline-newly`, or `baseline-YYYY` up to the embedded data.
Feature guards are required for known newer browser APIs; no polyfill or syntax lowering is promised.

MCP registration: new apps include Claude Code `.mcp.json`, Cursor `.cursor/mcp.json`, and `CLAUDE.md` → `@AGENTS.md`.
Existing app: `brain agent setup [--hooks]`; restart the client and verify its tools. Optional hooks check after source edits,
with no tests, downloads or auto-fixes. `brain mcp --profile minimal` exposes spec/check/fix/map/test/doctor; `backend` adds
focused resource/job contract tools. Default is `full`.
`brain test` and `brain doctor` run declared scenarios in owned headless Chrome; no browser extension is required.
A connected Claude Viewer/Codex Viewer is opt-in, only when the user asks: `brain e2e probe --json`, then `brain test
--viewer` (or `BRAIN_VIEWER=1`, or `brain e2e run`); with several connected profiles set private BRAIN_VIEWER_PROFILE, and BRAIN_VIEWER_SERVICE=claude-viewer|codex-viewer to pin one Viewer (an explicit open refusal moves to the next connected Viewer; docs/VIEWER-E2E.md). The
opted-in lane keeps the separate full Chrome AX/optional heap replay, an opted-in Viewer failure fails the run, and an
unavailable one falls back explicitly. Follow `docs/VIEWER-E2E.md` to author complete disposable-data E2E scenarios. `doctor` warning B0734 checks local registration, not connectivity.

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
