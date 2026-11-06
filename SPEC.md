# BRAINLLM SPEC
Normative. Anything not here does not exist. Internals: docs/INTERNALS.md (not needed to write apps). Any other `docs/…` path named here: `brain guide docs/<path>` prints it, so it works from an npm install.
Before each kind of task run `brain guide <topic>` (`brain guide` lists them): steps, checks, mistakes, example. `brain capabilities` lists what exists.

## Files
- `app.brain` entry: `<html lang>`, common `<head>`, layout, `<slot />` for the view. No `.html` anywhere.
- `views/*.brain` routes: `users.brain` → `/users`. `comp/*.brain` stateful. `ui/*.brain` stateless.
- `views/users/[id].brain` → `/users/:id` (`ctx.params.id` in `load`) · `views/users/_layout.brain` wraps its folder · `views/404.brain`.
- `views/docs/[...rest].brain` matches `/docs/a/b/c` with `ctx.params.rest = 'a/b/c'` (ranked after static and `[id]` routes).
- Route files per folder: `_guard.jsbrain` (`export default (ctx) => ctx.user ? true : redirect('/login')`, runs on server
  and before client navigation; `deny('reason')` renders `views/403.brain`) · `_error.brain` · `_middleware.jsbrain`.
  `ctx` = `{ params, query, user, session, cookies, url, env, signal }`; pass `ctx.signal` to `fetch` so the work stops when the client leaves or the deadline passes. In the browser there is no `ctx.user` (the session
  cookie is HttpOnly): a guard that reads it and would stop the navigation makes a full page load, and the server guard decides.
- In code: `navigate('/x')` (router, no reload; never `location.assign`) · `let r = route()` → `r.path`, `r.params.id`,
  `r.query.q` (reactive).
- `services/*.jsbrain` are injectable: `provide(Api, realApi)` in `app.brain`, `use(Api)` anywhere; tests swap them with
  `mount(Comp, props, { provide: [[Api, fakeApi]] })`.
- `_loading.brain` shows while `load` runs. `<meta cache="60">` in a view = rebuild at most every 60 s.
- Client navigation always gives feedback and never flashes: the current page stays until the next one is ready; the
  region gets `aria-busy="true"` and `route().navigating` is true at once; past 150 ms `#brain-nav` shows (a top
  `<progress>` bar; color it with `accent-color` on `#brain-nav` in `tokens.cssbrain`, or put your own
  `<div id="brain-nav" hidden>…</div>` in `app.brain`); past 400 ms `_loading.brain` replaces the region. A failed
  navigation is a full page load.
- After a client navigation focus moves to the URL's fragment target, else the page's `<h1>`, else the region, unless
  the new page already holds focus, and `#brain-announce` (polite live region) reads the new title. `app.brain` starts
  with `<a class="skip" href="#brain-view">Skip to content</a>`; keep it first in `<body>`.
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
- `brain new --static` creates a portable site without server/login examples; read `brain guide static`.
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
| auth | `auth` (no name) | `views/{signup,login,account/*}.brain`, `channels/session.jsbrain`, `server/auth.jsbrain`, `server/platform.jsbrain` | accounts on the native backend; existing files are kept |
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
Forbidden: class this var with eval new-Function generators get/set labels for-in for(;;) while switch, dynamic
`import()` (import statically; each view is its own chunk, and `<Comp lazy />` defers a component).
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
`{expr}` · `attr={e}` · `class:x={bool}` · `style:prop={v}` · `on:ev={fnRef | (e) => … | statement}` (`on:submit` prevents default)
· `bind:value|checked|ref={x}` · `{#if c}…{:else if d}…{:else}…{/if}` · `{#each list as it (it.id)}…{:empty}…{/each}`
(key required; list of primitives → key by value: `{#each perks as perk (perk)}`) · `{#await p}…{:then v}…{:catch e}…{/await}`
· `<slot />` `<slot name="x" />` · `<Comp a={b} on:ev={f} />`.
Native non-bubbling element events (dialog close/cancel, disclosure toggle, validation invalid, pointer enter/leave
and media events) listen on their element, preserve cancellation, and release listeners with the component owner.
Lazy components: B0225 warns when this file also renders the same imported component eagerly (potentially inlined `ui/` components are excluded); lazy delays mounting but cannot remove its eagerly required JavaScript. Total route budgets include lazy chunks. `<Chart lazy />` loads when visible · `lazy="idle"` · `lazy="interaction"`; `lazyLabel="…"` or `lazyLabel={text}` sets the placeholder's words (none by default, the button says "Load component"). Images: `<Img src alt width height>`
(lazy, decoding async, LCP priority set by the compiler). Interaction mode renders an accessible load button;
CSS remains eager to preserve styles when the deferred component appears.
Text interpolation is escaped. `id`s in `comp/` and `ui/`, static or computed (`id={'faq-' + it.id}`), keep their name
in the first instance (so `/page#id` links work); later instances get `~2`, `~3`… and their `for`, `aria-*` and
`href="#…"` references follow. A direct prop (`id={headingId}`, `aria-labelledby={labelId}`) is a caller-owned
document identity and stays unchanged; callers must supply unique IDs. An omitted/nullish prop uses its local
default with the instance suffix. Derived expressions and each-local IDs
remain component-owned. Ids in views and layouts stay as written.
`<pre raw>` and `<code raw>` keep their text literal (code samples with `{` `}`).
`bind:value|checked` also accepts a field of a sig object: `bind:checked={prefs.email}`. `<select multiple bind:value={tags}>` binds an array of option values.
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
Optional blank SEO strings are omitted. Root-relative social images follow `base`; bare relative images resolve against the final canonical page URL. Existing percent escapes are preserved.
Pick the query first; put it in the title, H1, first sentence and description. `keyword` is never rendered; an
indexable view without it is B0502 (guarded, `robots="noindex"`, `_x`, `[id]` and error views are exempt). A guarded route is never indexed; a public page that only redirects some visitors does it in `load()` (B0505).
A page rendered per request can list the parameters that make a different page, such as real pagination: `query="page"`
keeps them in the canonical. Any other parameter (a dialog, `urlState`, a tracking tag) stays out, so it restores the UI
without a new page for search. It must be literal, and it has no effect on a prerendered page (B0503).
A parameter named `page` in that list is checked: `?page=1` answers 301 to the base URL, and a value that is not a page number answers 404. A page past the last one stays your `load`'s `notFound`.
Values may be expressions from `load` props: `<meta seo title={props.user.name} description={props.user.bio}
jsonld={{ '@type': 'Person', name: props.user.name }}>`. Site defaults live in `app.brain`:
`<meta seo titleTemplate="%s · Nimbus" og:image="/og.png" twitter="@nimbus">`. Any other tag goes in a `<head>` block of
the view (merged, deduplicated by name/property). `[id]` views list their pages with
`export async function paths() { return [{ id: '1' }] }` so they are prerendered and appear in the sitemap. A listed page that throws or renders nothing stops `brain build`.
Live `[slug]` views (SSR, no `paths()`) export `sitemap({ env })` instead, returning `[{ slug, lastmod? }]` (key = the
bracket name; `lastmod` a `YYYY-MM-DD` string or Date; an item without the key is skipped). `brain guide seo`; never both, B0117.
A public dynamic view (no guard, not `noindex`) with neither export is listed nowhere: `brain build` warns B1837 with the route.
`brain.json` `"seo": { "focus": ["term"] }` (up to 3): an editorial report of where each term lives and links, not a ranking
factor. `seo-urls.txt` (opt-in, `brain seo urls --write`): a listed URL that disappears with no redirect fails the build (B1879).
Sitemaps need the deployed origin (`PUBLIC_SITE_URL` in `.brain_env`): with none, or a placeholder (example.com, `*.test`,
localhost), the build publishes no sitemap and warns B0736. Public pages rendered per request without params are listed
too; a site whose sitemap would list no URL stops at B1900.
Sitemap date: `<meta seo lastmod="2026-10-13">`, a loaded page expression such as `lastmod={props.updated}`,
or the collection's frontmatter `date`. Evaluated prerender dates reach the corresponding sitemap URL; absent/empty
dates are omitted, never inferred from file modification times.
Collections: `brain add collection blog` turns `content/blog/*.md` (frontmatter `title description date updated? tags? cover?
author? draft?`) into pages, tag pages, `/blog/rss.xml` and `/blog/atom.xml`. In `load()`/`paths()`, with no import:
`entries('blog')` `entry('blog', slug)` `tags('blog')` `tagged('blog', tag)` `pages('blog')` `page('blog', n)`; an entry has
`slug url title description date tags html`. Never on the client (B0705).
**Portable static export:** `brain new my-site --static` creates a site with brain.json `"output":"static"`.
`brain build --static` enforces the same policy for any project; normal build, doctor and MCP honor the saved policy.
It emits ordinary HTML, CSS, JavaScript and SEO files into dist/ for an HTTP static host: index.html at the root,
route/index.html for each route, and 404.html. No Node/Rust/BRAINLLM server is needed on the host. Assets are minified
and get gzip/Brotli siblings when smaller; raw files remain for hosts without precompression support.
Pure static pages need no page JS; browser interactions still work. Dynamic routes must enumerate paths(); their load,
seo and head run at build time. Request-time load, guards/middleware, server: functions, server forms, live sitemap,
native backend contracts and gateway policies stop the export instead of being silently omitted. Failed prerenders
and invalid paths() parameters also fail. Static output always uses .html/.js/.css; --ext brain is incompatible.
Static-only brain.json `staticOutput` optionally selects `js`, `css`, `images` and `assets` folders (safe relative URL
paths; `.` means the export root). Defaults preserve the usual output. `inlineCss:false` emits small stylesheets too;
`compression:'fast'|'small'` chooses encoding effort (small default), with actual-byte budgets. Original public/ URLs
stay stable and packaged SDK trees retain their relative structure. llms.txt and llms-full.txt are generated by default.
Upload dist/ only; optional generated host rules do not replace the host's routing, redirects, 404 status or HTTP
security/cache configuration. For subdirectory hosting set brain.json base. Static-only `staticOutput.host:"github-pages"` emits root .nojekyll and 404.html, requires an HTTPS origin without a path, and defaults hostFiles to false; set base separately for repository sites. `brain guide github-pages` explains the artifact and reviewed workflow preview. This is not a file:// or single-file
inline export. See `brain guide static` for the complete workflow; MCP `brain_build` accepts `static:true`.

Generated by `brain build`: robots.txt, sitemap.xml, llms.txt, llms-full.txt, manifest.webmanifest, rss.xml (site feed;
`<section>/rss.xml` per section) and sections.json (every section with its pages and feeds; `seo.feed` / `seo.sections`
false turn them off). Every page names its generator, home and source, and closes with a small "By BRAINLLM x.y.z" line
linking to the home page (`seo.credit: false` removes it). Google tags come from
`brain.json` (`"google": { "analytics": "G-…", "tagManager": "GTM-…", "siteVerification": "…" }`), never hand-written.
Explicit `google.mode: "basic"` enables the consent-gated declared `data-track` event bridge; use one GA4/GTM destination per environment, `events` mappings and optional `banner`/`funnels`. No Google loader or event queue before literal analytics consent. `brain google setup --env production --json` is a read-only review plan. See `docs/GOOGLE-ANALYTICS.md`; preserve legacy behavior unless opting in.
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
`Dialog` accepts `dismissible:bool=false`: opt in to close on a primary pointer click that starts and ends on the
backdrop. Content clicks, inside-to-outside drags and canceled pointer sequences keep it open. Native Escape,
modal focus/restoration and the close event remain intact; `urlKey` history is updated on dismissal.
Forms: `let f = form(schema, { submit: async (v) => … })` → `bind:value={f.v.email}`, `f.err.email`, `f.busy`,
`f.error` (form-level), `on:submit={f.submit}`. A rejected submit puts focus on the first rejected field, or back on the control that held it. Leaving a dirty form asks first, unless it holds a password field. Schema: `{ email: 'email!', age: 'num>=18', name: 'str!<=80' }`
(`!` required; types `str num bool email url date tel file`, `file(image/*,.pdf)!<=5000000` bytes and accepted types, `[T]` list, a nested type object). Cross-field: option `validate: v => ({ field: message })`, an empty message is valid; `docs/FORM-VALIDATION.md`. Server errors: return or
throw `{ errors: { email: 'Taken' } }` from `submit`, or mark them later with `f.errors({ email: 'Taken' })`; any other
failure shows in `f.error`, and `f.status` holds its HTTP status (0 with no response). Initial values: `form(schema, { init })`; later values: `f.reset(values)`; never replace
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
be a subset of HTTP origins (1–8 canonical origins; `brain check` reports B0744/B0745). Absent/false sockets disables it. It connects only to the app's
`<base>/_brain/socket`, sends no credentials in the URL, refetches after JOIN acknowledgement/reconnect/invalidation,
and closes on unmount. Scope is a public user/revision key or null, never a token; null disconnects and `state` says `anonymous` (sockets admit only signed-in users; a public page uses `resource(..., {every:'30s'})`, B0775). A session that ended becomes `auth-required`. HttpOnly cookie
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
Public API data offline: `"cache": { "routes": [{ "match": "/api/products/*", "strategy": "stale-while-revalidate",
"maxAge": "1h", "maxEntries": 50 }] }` (also `cache-first`, `network-first`, `network-only`); personal data → `network-only`.
`"update": "prompt"` waits for the user: `{#if updateReady()}<button on:click={() => applyUpdate()}>Reload</button>{/if}`.
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
Web Vitals (opt-in, 0 bytes unused): `vitals(m => …)` gives `{ name, value, rating }` for LCP, CLS, INP, FCP and TTFB where the browser has them; `reportVitals('/api/vitals')` sends `{ path, metrics }` by beacon when the page is hidden. Write that route yourself.
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
Links to public static pages that load a full document are prefetched on hover (Speculation Rules); `data-prefetch="false"` on a link or brain.json `"speculation": false` opts out, and `ctx.prefetch` is true on such a request.
Dev panel (`brain run` only, never in builds): Alt+Shift+O or `?brain-observe` shows vitals, requests, long frames, errors and live components; Pick names an element's component file. `__brainObserve.snapshot()` returns it as JSON.
Private captured errors (message, stack, recent actions) are listed by `brain_errors` (MCP) and `brain doctor`.

## Server and secrets
`load` runs only on the server. Client code may only read env vars prefixed `PUBLIC_`.
Variables live in `.brain_env` (already created): shared `KEY=value` lines, then `[development]` `[staging]`
`[production]` sections that override them. `brain run` uses development, `brain build` production, `--env x` another.
Non-production build environments automatically emit `noindex,nofollow` on static and per-request pages and omit
sitemaps; robots stays crawlable so crawlers can read noindex. Rebuild with `--env production` to restore indexing.
`PORT=5190` fixes the project's port for `brain run` and `brain start` (`--port` wins). A declared port never moves: a previous brain of this project holding it is stopped; any other owner is left running and named in the error. With no declaration, `brain run` takes the next free port from 5180.
An empty key fails the build (B0720); `KEY?=` declares one the app works without: it may stay empty and reads as unset (a route or webhook whose secret is absent answers 503 until it is set, B0790).
Read them as `process.env.KEY` (server) or `process.env.PUBLIC_KEY` (anywhere; an undeclared one reads as `undefined`). Never create a `.env`.
`.jsbrain` files starting with `<meta server>` run only on the server and may be used only inside `load` or server
functions. Server function in a `.brain`: `server: async function save(ctx, data) {…}`; the client calls
`await save(data)` (no `ctx`); only `server:` functions are callable from the browser, never `load`. Validate `data` inside.
RPC calls do not run the page route's guards or middleware. Authorize each server function using ctx.user and its
resource policies; a same-origin request is not authentication. `throw { message: 'Title already used' }` (400) or an Error
subclass with a 4xx `status` reaches the calling form's `f.error` as is; other exceptions stay private.
Server-only modules may signal a deliberate client failure with `throw {status:403}` (standard 4xx only); RPC returns
the status with a generic public message. Arbitrary exception messages remain private and are excluded from server logs too.
Sessions: `ctx.session.set(user)` in a server function, `ctx.user` on later requests, `ctx.session.clear()` logs out
(signed cookie, not encrypted: ids and display data only; a credential passed to it warns B0737; one in `console.*`, browser storage or a password in a returned value warns B0783). A server-only value
such as a provider token goes in `ctx.session.setSecret(value, {maxAge})`, read with `ctx.session.secret()`, removed with
`ctx.session.clearSecret()` or `clear()`: its own encrypted cookie, never `ctx.user`, also available with `BRAIN_BACKEND_URL`
(setup in `brain guide server`).
`load` belongs to views: in `_layout.brain` it is not run.
Model calls: `await ctx.llm.complete({ prompt, system, messages, model, provider, maxTokens, timeoutMs })` returns
`{ text, stopReason, usage, model, provider, requestId }`; `for await (const text of ctx.llm.stream(o))` reads the answer on the server. Set
`ANTHROPIC_API_KEY` + `ANTHROPIC_MODEL` or `OPENAI_API_KEY` + `OPENAI_MODEL`; `ctx.llm` in browser code or a `PUBLIC_` provider key is B0920 (`brain guide llm`).
Server limits (dev and production alike): brain.json `"server": { "maxBodyKB": 1024, "timeoutMs": 10000,
"rpcPerMinute": 60 }` bounds RPC/form bodies (413 past it, at most 65536), function calls and `load` (504) and calls
per IP and function (429). Raise `maxBodyKB` to send a file as base64 in an RPC argument; invalid values are B0707. `"earlyHints": true` sends 103 responses (default off: nginx below 1.29 breaks on them; `Link` headers are always sent).

### Experimental Rust/Postgres backend
`brain data defaults [--resource name]` previews missing declared defaults in stored rows; `--apply` fills bounded
batches (`--limit` 1..10000, default 1000 per field), preserving existing values and explicit nulls. `--include-null`
also fills nulls when explicitly requested. Inspect `hasMore`/`remainingRows` before another run. See `docs/backend/import.md`.
Computed values use `server/backfills/<name>.jsbrain` with `{ resource, field, compute: row => expression }`:
`brain data backfill <name> --env development --limit 100 --json` previews one read-only batch after migration 0029;
`--apply --request plan.json` applies an explicitly approved request with durable progress. Read
`docs/backend/backfills.md` before authoring expressions or applying a batch; no imports, SQL or application hooks run.

Native session users expose optional `name` and `picture` profile hints when providers share them, including `ctx.user`
and `/_brain/auth/me`; run `brain backend migrate` after upgrading. Hints never affect identity, roles or verification.
`brain backend capabilities` lists what this version supports (field types, filter operators, policy grants, limits), generated from the code.
`brain db audit --env development --json` observes only the selected current database/role in read-only transactions;
it reports bounded diagnostics and review proposals, not server-wide changes. See `docs/backend/database-audit.md`.
`brain backend conform --url <origin> [--path /api/example] --json` observes anonymous GET/HEAD with explicit JSON
negotiation. It sends no writes or credentials and proves only its observed response contract; see `docs/HTTP-RESPONSES.md`.

First read `brain backend schema --json` (resolved resource/job contracts + resource OpenAPI); inspect one with
`brain card resource:posts` / `brain card job:notify`. Generate with `brain add resource posts --fields "title:str! body:str"`
or `brain add job notify --fields "message:str!"` (a resource starts admin-only; `--policy owner|public-read` starts open); fields: `str num bool email obj`, `!` required. Resources have explicit
`schema/policy/hooks.jsbrain`; jobs have `schema/handler.jsbrain` under `server/resources/` and `server/jobs/`.
Resource generation also creates collection/detail clients under `api/`, an executable native repository and
server service, and `client-contract.json` pointing to the canonical schema revision. Add `--admin-ui` when creating
a resource to generate project-owned `/admin/<resource>` list/search, new, detail and edit/delete routes plus an
administrator guard and missing project DS controls. These files and the generated contract commit together through
validated file leases and compare-and-swap revisions. Existing files are never overwritten; without this flag no
routes or controls are created. Native backend policy remains authoritative. Review policy, migrate explicitly,
then build and doctor the application. The app's default 30 KiB route budget remains unchanged.
Set `DATABASE_URL`; explicitly `brain backend migrate`, then `brain backend serve` (loopback REST at `/api/<resource>`). After a bulk import, `brain data prepare --apply`.
An error response carries a `rid`; `brain backend logs <rid>` returns its cause (SQLSTATE, resource, operation, code location).
`PATCH /api/<resource>/<uuid>` changes only the sent fields (`null` clears an optional one); see `docs/backend/patch.md`.
`server/operations/<name>.jsbrain` declares typed input and steps that `POST /_brain/operations/<name>` runs in one transaction (all or nothing), `ctx.backend.operation(name, input)` from server code; see `docs/backend/transactions.md`.
Actions, transitions and operations may explicitly declare `systemOnly:true` and omit `roles`. Only a verified
operator-issued system key with the required named resource/operation scope can invoke them. Admin sessions,
ordinary API keys and a user role named `system` do not qualify. Empty ordinary `roles` remains invalid; mixing
`systemOnly:true` with role grants is invalid. An operation invoking a system-only step must also be system-only.
`actions.jsbrain` declares guarded atomic writes (`when`, `set`, `increment`, `decrement`) at `POST /api/<r>/<uuid>/actions/<name>`; `POST /api/<r>/upsert` inserts if absent on a unique key; see `docs/backend/atomic-updates.md`.
`POST /api/<r>/batch` (`op`: create|update|upsert|delete, `items`) is all-or-nothing, max from `batch.jsbrain { max }`; see `docs/backend/batch.md`.
`transitions.jsbrain { field, initial, transitions }` makes an `enum` field a state machine changed only by `POST /api/<r>/<uuid>/transitions/<name>`; see `docs/backend/state-machines.md`.
`server/jobs/<name>/queue.jsbrain { priority, attempts, backoff, timeout, lease, drain, concurrency }` tunes one queue; `brain jobs dead|retry|stats <queue>`; see `docs/backend/queues.md`.
`server/cron/<name>.jsbrain { schedule: '0 7 * * *', job, payload?, catchUp?, priority? }` enqueues once per slot (UTC); run `brain jobs cron`; see `docs/backend/cron.md`.
`brain jobs enqueue|status|work|prune` manages persistent, at-least-once work; `brain jobs enqueue <queue> --payload '<json>'` takes the payload. Handlers must be idempotent by `ctx.jobId`. Each queue needs a running `brain jobs work <queue>` (or `brain backend serve --with-workers`); `brain jobs stats` lists every queue with its live workers.
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
`now-7d`), `sort` (`?sort=-price,title`, ties by id), `include` (dotted paths up to three levels), `or` groups (`?group.field=v`), `?fields=a,b`, `?owner=me`, named `aggregate`s and related `count`s;
Named aggregate `group` accepts one scalar field or 1–4 distinct scalar/list field names (at most one `list<str>` dimension), deduplicating list elements per row. A scalar group keeps its scalar key; field arrays return object keys. It uses the full policy-filtered row set, not the current list page; see `docs/backend/aggregates.md`.
undeclared or repeated parameters → 422. Lists return `total` and `next_cursor` (pass back as `cursor` or `after`).
`facets: [field]` adds `?facets=a,b` counts over readable rows; a `geo` field (`{lat,lng}`) filters with `near` (`lat,lng,km`) and `bbox`.
A field of kind `org` makes a resource tenant data, with grants `org`, `org.<role>`, `org.own`; `audit: true` in a policy records changed field names; `server/auth/captcha.jsbrain` requires a token on protected public writes (docs/backend/captcha.md); `docs/backend/sessions.md` covers idle timeout, list and revoke, and route `reauth`; `/_brain/credits/{subject}` holds an atomic ledger.
Read and update grants are independent: a declared update grant can authorize a write even when GET is hidden; its response redacts unreadable data. Remove or narrow update permission to disallow blind writes.
Read grants take row conditions (`{ public: { where: { status: 'published' } } }`, OR-ed; hidden rows answer 404 on every read channel; B0749 warns on an unconditioned public read with a state field, `expose_all_rows: true` declares it); `api: false` makes a resource server-only (`ctx.backend.service(name)`).
Policy grants `owner.<ref>[.<ref>]` scope rows through relations; field option `visible: ['owner','admin']` hides
a field from other readers and from the query surface. Modifiers `'str writeonly'`, `readonly`, `private` control
client writes and reads; `access: { write: ['admin'] }` limits writes by role. Constraint `unique: [['$owner']]` allows one record per owner. `owner` is returned only to owners and named read roles unless `expose_owner: true`.
See `docs/backend/*.md`.
Set `BRAIN_BACKEND_URL` to the fixed loopback native backend URL to connect native API requests and SSR session verification.
Native JWT accounts use short-lived access and rotating refresh cookies, persisted revocation, configured roles and Google, Apple or LinkedIn sign-in (`docs/APPLE-LINKEDIN.md`).
Native Apple clients instead opt in with `APPLE_NATIVE_APP_IDS` and use the separate `/auth/apple/native` challenge/token/refresh endpoints under `/_brain`; explicit token JSON, no browser cookies or Origin. See `docs/APPLE-LINKEDIN.md` for nonce binding and migration 0028.
Roles: `admin` and `user` by default, signup role `user`; `brain add roles` writes `server/auth/roles.jsbrain` for your own names (only `admin` is required). A role grant admits only that role; route `auth: 'user'` means any signed-in account.
`brain recipe auth-ui|password-reset|email-verification|google-signin|apple-signin|captcha` prepares project-owned account UI;
mail needs private `BRAIN_MAIL_RESEND_KEY`, `BRAIN_MAIL_FROM` and `PUBLIC_SITE_URL` (HTTPS, optional hosting prefix). The emails are in English unless the default locale defines `auth.email.verify`, `auth.email.reset` and `auth.email.notice` (B0941).
`brain backend bootstrap-admin --user <existing-uuid>` (or `--email <address>`) explicitly assigns the first administrator; signup never grants it.
API keys: add `server/api-keys.jsbrain` `{enabled:true}`; signed-in users manage keys at `/_brain/keys` (secret shown once; `selfService:false` closes those routes and leaves only the operator CLI), callers send `X-API-Key: bk_live_...` to `/api/<resource>` only; scopes `posts:read|write|*` never exceed the owner. See `docs/backend/api-keys.md`.
`brain backend users disable|enable|revoke-sessions|disable-identity|enable-identity` and their administrator routes act without SQL; a route with `auth:'internal'` takes a rotatable secret in a header (`docs/backend/routes.md`).
See `docs/AUTH-FLOWS.md`; mail responses deliberately do not disclose account existence or delivery failures.
In a same-origin POST server function, `await ctx.backend.login({email,password})` establishes native JWT or opaque
HttpOnly cookies and returns the sanitized user; the backend chooses the configured mode. Never expose signing keys
or copy tokens into browser storage. `await ctx.backend.signup({email,password})` creates the account without a session; `login` after it in the same function signs in. A refused address is a 422 with `errors.email` (invalid and taken read the same, by design) and a bad password `errors.password`, so `form()` marks the field. `ctx.backend.google.challenge()` and `.login({credential})` do the same for Google. `ctx.session.clear()` revokes native sessions. See `docs/NATIVE-AUTH-SSR.md`.
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
Queues and routes may declare `runAs: 'system'` to use an operator-created system key (`keys create --system`), which bypasses policy, owner scope and `private`/`readonly` fields within its scopes. Routes get `ctx.backend` only with `runAs`. `ctx.backend.files(bucket)` works from jobs, routes and server code with a `files.<bucket>` scope.
Reads and writes run through the same native schemas, policies and authenticated cookies, not a second database client.
Writes require a same-origin POST server function; they are rejected from GET/load. Put domain logic in server-only
service/repository `.jsbrain` modules and pass `ctx` explicitly. Request JSON is limited to 64 KiB, responses to 2 MiB,
with a shared eight-request cap and ten-second total deadline; no automatic mutation retries.
Files of any type: `server/files/invoices.jsbrain` `export default {types:['application/pdf'],maxBytes:'10mb',create:['user']}`
→ multipart `POST /_brain/files/invoices` (field `file`), `GET /_brain/files/invoices/<id>` (Range, ETag). Type comes
from content, not the name; empty `types` accepts any type. See `docs/backend/files.md`.
Edge: `server/platform.jsbrain` `export default {trustProxy:['127.0.0.1'],cors:{origins:[...],maxAge:600},limits:{maxInFlight:256},shutdown:{readyDelay:'5s',drain:'25s'},access:{perMinute:300,concurrency:128,mode:'open'|'own-host',origins:[...],hosts:[...],ips:[...],local:true},logging:{requests:'errors'|'all'|'off',slow:'1s'}}`. Nothing is limited by default; `access.perMinute` (429) and `access.concurrency` (503) budget page renders, form posts and the native API, and `mode:'own-host'` refuses the native API to other sites and plain scripts with 403 B0752 (`docs/backend/access.md`).
Probes `GET /health/live` and `GET /health/ready`, also answered on the `brain start` port; overload answers `503 OVERLOADED` + `Retry-After`. Production
requires `sslmode=verify-full` for a remote `DATABASE_URL`. See `docs/backend/proxy-cors.md`, `docs/backend/lifecycle.md`.
Several processes: `cluster:{shared:true}` in `server/platform.jsbrain` (automatic when `BRAIN_BACKEND_PROCESSES>1`) keeps route/API-key rate windows in Postgres and relays socket events between processes. See `docs/backend/cluster.md`.
Caching: API GETs send `ETag` (304 on `If-None-Match`); `server/cache.jsbrain` `export default {products:{maxAge:'1m',staleWhileRevalidate:'10m'}}` sets a public `Cache-Control` for anonymous reads of public-read resources; `If-Match` on PUT/PATCH/DELETE → 412 `PRECONDITION_FAILED` when stale. See `docs/backend/caching.md`.
Routes: `server/routes/<name>/route.jsbrain` `export default {method:'POST',path:'/orders/{id}/quote',auth:'user',params:{id:'uuid'},query:{currency:'str!'},bodyLimit:'16kb',timeout:'5s',rate:'20/min per user'}`
B0816 warns on literal handler reads missing from the route's `headers` allowlist, including proven aliases. Read lowercase names; explicitly declare safe headers and use ctx.user for authentication.
+ `handler.jsbrain` `export default async (ctx) => ({status:201,body:{...}})`; throw `{errors:{field:'msg'}}` → 422.
Signed: add `body:'raw',verify:{scheme:'timestamped'|'hmac',header,secret:'ENV_NAME',tolerance:'5m'}`. See `docs/backend/routes.md`, `docs/backend/webhooks.md`.
For another provider protocol, `body:'raw',auth:'public',verify:{scheme:'custom'}` requires a private sibling `verify.jsbrain` default function. Only boolean `true` admits the handler; use exact raw bytes, declared headers and bounded cryptographic verification. It has no visitor/backend authority and shares the handler's total deadline. See `docs/backend/routes.md`.
Outbound: `await httpCall(url,{method:'POST',json,idempotencyKey:true,timeout:8000,retries:2})` → `{status,ok,json,text}`; throws `HttpCallError` (`TIMEOUT`, `HTTP_STATUS`, `UNSAFE_URL`, `REDIRECT`…). Every resolved address is checked: private, loopback, link-local, CGNAT and multicast targets are refused unless `allowPrivate` names the host; `redirects:n` follows n hops, each checked again. See `docs/backend/outbound.md`.
Keep existing production routes until their behavior and policies pass a tested migration slice.

## Tests (`*.test.brain`)
`test(name, async () => {…})` · `await mount(Comp, props, { slot, slots, provide }?)` (`slot`: default slot text;
`slots: { name: text }`: named slots) · `await click(sel)` · `await type(sel, text)` ·
`text(sel)` `attr(sel, n)` `hasClass(sel, c)` `count(sel)` ·
`expect(x).toBe(y)|toEqual(y)|toBeTruthy()|toBeFalsy()|toContain(y)` (string or array), each also as `expect(x).not.…` ·
`await waitFor(fn, { timeout: 3000 }?)` retries `fn` after each render until it neither throws nor returns `false`; use it instead of a fixed sleep.
A `<meta server>` module cannot be imported by a test (it runs only on the server): test what calls it, with `stub`.
No server is needed: `stub('save', { ok: true })` answers a `server:` function, `stub('GET /api/posts', [{ id: 1 }])` an endpoint (add `?query` to match it exactly). The answer is a value (200), `reply(422, { errors: { email: 'Taken' } })`, or a function of the call's arguments returning either; `stub()` returns `{ calls }`. It lasts one test, and one never called fails it. `mount`, `click` and `type` wait for stubbed answers.
`brain test` fails on a path that selects no test. It runs beside `brain run`: each uses its own port, and with native sockets, whose port is fixed, tests use the running dev server.
Before opening the browser or reusing dev, it compiles the selected tests' current local Brain import graph, including
transitive and automatically discovered components. Current compilation errors stop assertions; unrelated components
are outside a selected test's preflight. Test scripts retain their browser-provided test API globals.
Component tests stay on `/@brain/test.html`: stub navigation instead of reloading or leaving that document.
A navigation that destroys a pending headless test fails that named test; the runner restores the same registered
test list in a fresh document for later tests and keeps later files running. Interrupted-document diagnostics are
incomplete and never count as a passing test. Verify real page navigation on an actual app page.

## Living brandbook
Every new site includes `/brandbook` and public editorial entries in `design.brain.json.brandbook.sections`.
Before brand/copy/UI changes, read `brain guide brandbook`. Keep strategy, voice, identity/product, assistant,
channels/governance and research aligned; use explicit draft/proposed/approved/implemented/not-applicable statuses,
concrete tone examples and traceable public evidence. Never invent approval, research, metrics or legal claims.
The exact empty `<section data-brain-brandbook="content"></section>` on its own line in views/brandbook.brain
(or views/brandbook/index.brain) is expanded consistently by check, dev and build. Current token declarations and
UI inventory come from actual project sources; editorial entries remain untouched. Generated facts do not verify
computed contrast, accessible behavior or editorial approval. Authored pages without the marker remain authored.
An older design-enabled app missing the route gets B2202 (warning); `brain add view brandbook` adopts it without
replacing existing routes. Invalid explicit managed content is B2203. Draft references start noindex; publish their
indexing deliberately after review. The guide defines every stable section ID, limits and the authoring loop.

## CLI (also MCP: `brain mcp`; every command accepts `--json`)
Main: `brain new <app>` · `brain run` (dev server with hot reload, app :5180; views render in the browser, so judge
SEO and the no-JS HTML on `brain build` + `brain start`) · `brain build` (production) ·
`brain doctor` (everything at once: check + tests + over-engineering + leaks; writes no versioned file; exits non-zero on any error; suspected leak B0601 is a warning unless `doctor.failOnSuspectedLeaks`).
Doctor builds and audits fresh private temporary output, then removes it; the app's dist/ stays unchanged.
`brain seo preview /route --url <origin> --json` inspects one live HTML response, including per-request routes.
`brain seo diff --from <sitemap.xml|http(s) sitemap URL> --json` compares previous and built sitemap path+query
membership without writes; host changes alone preserve membership. It does not prove content or redirects.
See `docs/SEO-SITE.md` for input/request limits and stratified live sampling.

Run `brain build` explicitly to produce deployment output after doctor passes.
Optional frontend route constraints use `brain.json` `routeParams: {"/:code":{"code":"[2-9a-z]{7}"}}`, keyed by exact `brain map` routes. Matching uses the whole original encoded segment before guard/load dispatch, with a bounded deterministic ASCII grammar; `/%33abcdef` does not match. `redirects.json` accepts the same rule `params` predicates. Constrained redirects require the server evaluator and reject portable static export; finite valid constrained `paths()` remain exportable. See `docs/ROUTE-CONSTRAINTS.md` for grammar, B1880 and host export capability failures. Constraints do not grant authorization.

Optional native document-performance aggregates use `vitals: {enabled:true,routes:["/","/items/:id"],retentionDays:30}` in brain.json, existing Node/SSR output and a fixed native backend. Capture requires explicit analytics consent and stops on SPA navigation. Only declared patterns and bounded numeric estimates are stored as daily aggregates; no visitor/session IDs, raw URLs, exact p75 or unique-visitor claim. `brain vitals report --env development --days 7 --json` is read-only. Static builds reject the collector; custom `reportVitals(path)` remains available. See `docs/backend/vitals.md` for bounds, shared quota, lazy retention and dev transport limits.

Optional production startup reads: `server.warmup` declares up to eight exact public cached SSR paths with a shared 100–10000 ms budget, anonymous GET only, and `onError: continue|abort`; static output rejects it. See `docs/STARTUP-WARMUP.md`.
Optional browser error reporting: `sentry: {enabled:true}` requires existing Node/SSR output, private server configuration and explicit analytics consent. No automatic artifact upload or static support; see `docs/SENTRY.md` for privacy, maps, limits and measured overhead.
New theme scaffolds include print and forced-colors defaults; `content-visibility` stays an explicit authored CSS choice (`docs/BROWSER-DEFAULTS.md`). Compiler upgrades preserve existing themes.

Production: `brain start` (serves `dist/`, restarts on crash, logs to `logs/app.log`) · `brain reload` (no downtime) · `brain status` · `brain stop`.

`brain bench --url <HTTP(S)-URL/path> [--requests 3] --json` makes 1–20 sequential anonymous GET observations at one URL, with no query, credentials, fragment, redirects or retries. Limits: 3 seconds per request, 30 seconds total, 256 KiB per body. Results distinguish firstObserved/repeatObserved; they do not prove cold/warm cache state, rendering performance, field Web Vitals or build-budget compliance. Non-2xx and failed observations exit 1; invalid inputs/deadline failures exit 2.
`brain adopt --audit [source-directory] --json` inventories legacy frontend source metadata without executing or rewriting it: known dependency-key framework hints, file counts/extensions, route/test file-name candidates and a per-slice parity checklist. It reads package.json only (256 KiB), skips hidden/generated/dependency trees and symlinks, and bounds 20000 entries, 2048 source files, depth 20 and 512-byte relative paths. No source bodies, environment values, package scripts or dependency versions enter its report. Inventory does not establish behavioral parity; use docs/MIGRATION.md.
`brain connect --json` prints a local stdio configuration proposal using the current executable and project directory, and completes initialize with one owned minimal-profile MCP child (5-second handshake, 64 KiB response, 1-second termination grace). It reaps the child, invokes no tools and writes no client settings. clientConnected remains false: an external agent client must still adopt its configuration. The proposal includes local filesystem paths; keep it private.
`brain manifest practices --request '{"family":"performance","limit":30,"cursor":0}' --json` pages actual compiler diagnostic records (maximum 100), with optional family all/html/css/seo/performance/webmcp/language and codePrefix filters. It exposes default level, explanation, correct example and allowed lint levels without executing source, building or adding UI. Family labels are navigation hints; built/live SEO B18xx remains in its own audit catalog.
Compiler advice B1301/B1303/B1305 identifies serial awaited iteration, repeated nested traversal of the same stable collection, and proven-array work repeated in each-item rendering. B1312 requires a direct unshadowed imported API GET with a real declared contract and an item-dependent argument; it never infers database query count. B1313 requires a stable DOM receiver from an unshadowed document querySelector/getElementById or an unshadowed ref() bound to a native element and a straight-line write-before-layout-read inside a loop. Warnings are conditional review guidance with no edits, timing claims, purity proof or automatic reordering. Known aliases, mutations, shadowing, loop-carried dependencies and unknown receiver types are skipped conservatively where they invalidate the structural case.
`brain quality refresh --json` projects actual static findings through bounded manifest reachability into perViews review actions for existing images, page metadata, rendering, performance and interaction states. Its graph is capped at512 documents/4096 edges/depth32,128 views and256 finding references; omissions are explicit. Existing build feedback adds up to128 actual route byte records without another build. Existing brain run impact records add viewAdvice from that save's diagnostics, affected views and measured artifact/SEO deltas. These tools do not judge screenshots, execute a visual evaluator, generate images/pages, or infer unmeasured browser/field performance. JSON remains the agent interface; no additional watcher or external service starts.

Optional WebMCP discovery advice B1706/B1707/B1708 accepts lint levels off/info/warning in brain.json; error overrides retain the existing default. Coverage is by route and never requires every component or form to expose a tool. B1701–B1705 contracts/confirmation and B1709 duplicate-tool safety cannot be disabled or lowered; existing default severities remain unchanged.

Optional PM2/nginx/GitHub Pages configurations: `brain deploy config pm2|nginx|github-pages --request <JSON>`, and `brain deploy check --url <origin>` for what only the live server shows (headers sent twice, proxy, `/health`); `brain edge plan|check|audit` compares declared delivery with bounded anonymous observations; explicit Cloudflare plans/apply use a reviewed request (`docs/EDGE.md`); see `docs/DEPLOYMENT.md`. `brain release-check [--env production] [--url <origin>] [--json]` is the one gate before a release: it runs `brain doctor`, the secrets of that environment (a route left disabled by B0790 fails it), `brain build --env <env>` and `brain seo audit`, plus the live checks with `--url`, and exits non-zero if any fails.
`brain stop`, `brain reload` and `brain status` accept an optional project directory from any working directory;
relative/absolute paths and existing project subdirectories resolve the nearest app. An invalid explicit path fails
without controlling the current app. Without a path, the current app is used.
Reload never recompiles source. Run `brain build` first; successful reload JSON can include bounded timestamp advice
for newer sources or incomplete freshness analysis. This is advisory, not a source-hash comparison.
Optional Node public-SSR shared caching requires both private `REDIS_URL` and `BRAIN_REDIS_NAMESPACE` at runtime;
only eligible anonymous `<meta cache>` pages participate. It does not change native resource/session caches or
provide distributed single-flight. Failure falls back locally; read `docs/REDIS-CACHE.md` before enabling it.
Hosted under a sub-path: `brain.json` `"base": "/app/"`; keep writing app-absolute links and paths (`<a href="/about">`,
`redirect('/login')`); the build adds the base everywhere.
Granular: `brain add view|comp|ui <name> --props "…"` · `brain check --json` · `brain test` · `brain spec [--topic x]` ·
`brain guide [topic]`.
`brain add comp <name> --blank` emits minimal noninteractive markup and a slot, preserving typed props and its
colocated test while omitting example state, controls and CSS. Other kinds reject this switch. MCP `brain_add`
accepts the equivalent `blank: true` only for kind `comp`.
Also: `brain preview` (temporary loopback server for `dist/`, Unix) · `brain explain <code>` ·
`brain doctor --fix` (apply mechanical fixes and update `.brain_env.example`, then run the full doctor). Config: `brain.json`.
<!--planned:#68-->
Unified exit codes across all commands: 0 ok · 1 project errors · 2 usage · 3 internal.
<!--/planned-->
`brain fix` applies every `edits` from `brain check` and re-checks (run it before editing by hand). It writes only inside
the path you give it; without a path it skips info-level normalizations; a file with uncommitted changes from before
the run, reached through a folder or the whole project, is listed in `skipped` and left alone unless `--force` (a file
you name is always written).
`brain trace [<requestId>|issues] [--since 5m] [--failed] [--module m] [--limit n] [--json]`: what `brain run` recorded (MCP `brain_trace`): requests with payload and response, errors at their `.brain` line, CSP, cross-origin and socket failures, backend queries, with B-codes and fixes. Development only and redacted; `brain run --no-trace` turns it off.
`brain map --json`: whole project graph (routes, components, props, channels, tests); read it instead of opening files.
`brain docs [--check] [--json] [--out docs/app]`: generated app docs (routes, components, data, backend, env keys, SEO); `brain build` refreshes them when `docs/app/index.md` exists or `brain.json` has `"docs": true`; prose goes in `docs/app/notes/<page>.md`. New starters ignore generated docs/app output in Git and keep authored notes visible; existing ignore policies are preserved.
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
`brain risk <target> [--measure] --json` (MCP `brain_risk`) scores an edit before it is made: a deterministic 0..100
`score`, `level` (low/medium/high/critical), `factors` (dependents, routes, public contract, backend schema, covering
tests, checker errors, SEO/i18n/a11y, styles, budget headroom with `--measure`) and `requiredChecks` to run afterwards.
MCP full profile: `brain_changes`, `brain_intelligence`, `brain_risk`, `brain_recipe`. See project developer docs for JSON contracts.
`brain upgrade [--from V] [--to V] [--dry-run|--apply] [--json] [--no-doctor]` (MCP `brain_upgrade`) migrates the app
from the version its source targets (`brain.json` `"brainVersion"`, written by `brain new`; else the package pin) to the
installed compiler or `--to`. The dry run (default) writes nothing and reports, from the change manifests in between:
codemods, manual steps with `file:line`, capabilities not used yet with their adoption snippet and guide, and gates.
`--apply` runs the safe codemods (package pin, `brainVersion`, `brain fix` for new codes), `brain check` and `brain
doctor` (skip with `--no-doctor`). Exit 0 = nothing left, 1 = manual steps or a failing gate remain, 2 = cannot run.
`brain upgrade <exact-version>` only previews package pin operations and unresolved install decisions; it does not
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
Generated builds retain the BRAINLLM landing attribution by default. HTML/XML/Markdown, own JS/CSS, robots and host hints use
format-valid comments or existing generator links. Attribution precedes final content hashes, SRI and compression;
XML declarations and sitemap limits remain valid. `brain-build.json` identifies the generator for the whole artifact,
including binary assets and compressed variants, without changing copied assets or arbitrary JSON schemas. A visible
page credit setting does not remove this source/build attribution. Only when the user explicitly requests removal, set the top-level brain.json boolean `compilerBranding:false`. It disables generated visible credit, generator metadata, comments, compiler landing/repository links and attribution headers across static/ordinary builds; `seo.credit:true` cannot override it. LLMs must never disable it automatically. Authored content, copied assets, third-party licenses and functional build/runtime identifiers remain intact. Changing this policy drops obsolete generated chunks rather than rewriting old content hashes; publish the complete rebuilt artifact.
When the user explicitly requests one attribution per page instead of repeated browser asset comments, set
`compilerBrandingScope:"page"` in brain.json. The default `"file"` preserves existing per-file comments. Page scope
keeps HTML attribution, generator metadata, visible credit, discovery documents, build metadata, host headers and
standalone service-worker/server attribution; only generated browser JS/CSS comments are omitted before hashing and
compression. It works for static and ordinary builds. This does not disable branding or rewrite authored/copied bytes.
Existing unreferenced chunks may remain during the normal cache grace period; deploy the complete rebuilt output.
An absolute HTTP(S) `<a rel="external" href="https://example.com/live-only/">` may deliberately target a separate site
on the same public origin. SEO does not require that navigation destination in this build; router interception and
predictive prefetch leave it native. Relative links and resource assets keep their checks. Static doctor reports the
external origin as unchecked, not verified reachable.
`brain build` is production: minifies, mangles, obfuscates and removes dead code and `log()`. No flags needed. `--strict` fails the build when a page ships a component fallback or a route is left out of `dist/`.
It also adds SRI and a strict CSP with the security headers to every page. `"csp": { "frameSrc": ["https://www.youtube-nocookie.com"] }` lists the origins an `<iframe>` may load, and `"scriptSrc"` the exact hosts scripts may load from (B0811 flags a literal one outside them). `brain.json` `"csp": { "connectSrc": ["https://api.example.com"] }` lists the origins pages may call besides the site's own; `[]` allows only the own origin, and omitting it allows any HTTPS origin and warns B0795.
`seo.siteName` sets the public display name used by SEO, feeds, manifest and assistant discovery, independently of the root package `name`. It must be a nonempty string up to 512 bytes without control characters. `seo.markdown:false` omits Markdown twins and makes llms.txt link to HTML pages.
`"csp": { "formAction": ["https://forms.example.com"] }` allows native `<form action="https://forms.example.com/contact" method="post">` submissions to exact HTTPS origins, in static and SSR output. The own origin stays allowed; omission or `[]` keeps own-origin only. HTTP is loopback-only; no wildcard, path, blob or socket entries (B0796). Keep named controls, browser constraints and server-side validation; omit `on:submit={f.submit}` for native submission. See `brain guide forms`.
`"csp": { "mediaSrc": ["blob:", "https://media.example.com"], "workerSrc": ["blob:"] }` explicitly permits media and worker sources, independently of script sources. Both lists accept `blob:` and exact HTTPS origins (HTTP only on localhost/127.0.0.1); no wildcards, paths, broad schemes or `data:`. `[]` means the own origin only; omission preserves the standard directive fallback. Invalid entries are B0796 and never widen the policy. A player using MediaSource may need `mediaSrc: ["blob:"]`; add `workerSrc: ["blob:"]` only if it creates a blob worker.

`range(from,to)` returns safe integers from inclusive `from` to exclusive `to` (at most 10000; reversed endpoints give
`[]`). Use `for (const i of range(0,10))` for bounded numeric iteration. `text.fold(value)` applies NFD, removes
U+0300–U+036F and lowercases; `text.distance(a,b,max=2)` gives case-sensitive Unicode OSA edit distance, counting an
adjacent swap as one, or `max+1` when past the cutoff (`max` 0..64). `text.fuzzy(word,list)` returns original candidates
in input order after folded comparison: 0 typos below 4 characters, 1 from 4, 2 from 8. Text inputs are bounded to 4096
Unicode characters and candidate lists to 10000, with at most 1048576 UTF-16 code units across candidate text.
Invalid inputs throw RangeError. These helpers work in client and
server code without imports; see `docs/NATIVE-LIBRARIES.md`.

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
