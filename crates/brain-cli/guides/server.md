# guide: server — sessions, errors, environment, deploy with brain start

## When
Sign-in and sessions, server-only code and secrets, per-environment configuration, error pages, and putting the app
in production.

## Experimental resource backend
For typed CRUD resources and durable jobs, read `brain spec --topic server` and the backend part of `brain map`.
Create contracts with `brain add resource posts --fields "title:str! body:str"` or `brain add job notify --fields "message:str!"`.
Inspect one contract with `brain card resource:posts` / `brain card job:notify`, and all resolved schemas with
`brain backend schema --json`. The Rust/Postgres backend has its own sessions and runs separately through
`brain backend serve`. Configure fixed loopback `BRAIN_BACKEND_URL` to connect development/production SSR and
server functions to native identity and resources. `ctx.backend.login({email,password})` relays native HttpOnly
cookies from a same-origin POST; JWT signing keys stay exclusively in the backend. `ctx.backend.resource(name)`
provides list/get/create/update/remove through native schemas and policies. Resource generation includes typed
collection/detail clients, a discoverable canonical contract, server service and repository. Administrative UI remains
planned in 0.4: the command creates no views, guards or controls and preserves the default 30 KiB route budget.
Build application views deliberately using the project design system. Account recovery/verification recipes need explicit
mail setup. Review policy and migrations explicitly before serving. Never remove an existing
production API merely because the experimental contract compiles.

Mandatory before-write validation uses `brain add hook <identifier>` and `before_create|before_update|before_delete`
in resource hooks. Return `ctx.payload`; native schema revalidation and row-conflict checks remain authoritative.
After-write jobs commit with their mutation. See `docs/BEFORE-HOOKS.md` and `docs/BACKEND.md` for limits.

## Sessions
1. Set `BRAIN_SECRET` in `.brain_env` (MCP: `brain_env_set { key, section, value }`).
2. In a `server:` function: `ctx.session.set({ id, email })` signs it into the `__Host-session` cookie (HttpOnly,
   Secure, SameSite=Lax, 7 days; `ctx.session.set(user, { maxAge: seconds })` to change it).
3. Every later request sees it as `ctx.user` in guards, `load` and server functions. `ctx.session.clear()` logs out.
4. The cookie is signed, not encrypted: store ids and display data, never tokens, passwords or secrets.
5. Protect a folder: `brain add guard account` → `export default (ctx) => ctx.user ? true : redirect('/login')`.
6. Never set the session cookie from browser code (B0713); never keep the signed-in user in `store(…, { sync: 'storage' })`.

## Server-only code
- `load` and `server:` functions run only on the server. Modules that touch a database or secrets start with
  `<meta server>` (`brain add server users-db`) and are imported only by `load` or server functions (B0701).
- The client never calls `fetch()` to your own server: use `server:` functions or `api()` (`brain guide data`).
- Do not build server fetch URLs from request data (B0711); redirect only to same-origin paths (B0710).

## Environment (`.brain_env`, already created; never a `.env`)
```
API_URL=https://api.example.dev
PUBLIC_SITE_NAME=Acme
[development]
API_URL=http://localhost:4000
BRAIN_SECRET=dev-only-random-value
```
In production set `BRAIN_SECRET` (32+ random bytes) in the `[production]` section or in the host environment.
- Shared `KEY=value` lines first, then `[development]` `[staging]` `[production]` sections that override them.
  `brain run` uses development, `brain build` production, `--env staging` another.
- Server: `process.env.KEY`. Client: only `process.env.PUBLIC_KEY` (B0702 otherwise; a secret in a `PUBLIC_` key is B0723).
- `.brain_env` is never committed (B0725); `.brain_env.example` lists the keys (B0727 when out of sync).
- An empty key in the build environment fails the build (B0720). MCP `brain_env_keys` lists keys, never values.

## Errors
- `throw notFound()` in `load` → 404 with `views/404.brain`; `throw redirect('/x')` → 303; `deny('reason')` in a
  guard → `views/403.brain`. `_error.brain` is the folder's error boundary (retry button, request id, no stack in production).
- A server function that throws a plain `Error` answers 500 with a generic message; the real one goes to the server log.
  To show a reason, throw `{ errors: { field: 'msg' } }` (422, field errors) or `{ message: 'msg' }` (400, the calling
  form's `f.error`); an Error subclass with a 4xx `status` (`class Conflict extends Error { status = 409 }`) is public too.
- Components are isolated: a failing component shows its fallback (`<meta fallback="Could not load">`) with Retry and
  the page keeps working. Do not wrap everything in try/catch.
- Browser errors captured in development: `brain doctor` and MCP `brain_errors`.

## Deploy (one way: `brain build` then `brain start`; never PM2 or another process manager)
```bash
brain build                       # production into dist/ (minify, CSP + SRI, sitemap, robots, llms.txt)
brain start --port 3000 --instances 2   # static apps served from Rust; apps with server code run Node children
brain status                      # mode, instances, restarts, last exit reason
brain reload                      # zero-downtime: new instances ready before the old ones stop
brain stop
brain start --install             # writes a systemd unit (Linux) or launchd plist (macOS) and prints the enable command
```
Logs go to `logs/app.log` (rotated). Hosted under a sub-path: `brain.json` `"base": "/app/"`.
`brain preview` serves `dist/` on a temporary loopback port to look before deploying.

## Verify
```bash
brain check --json                # [] (B07xx are server and secret rules)
brain build --env production      # fails on empty variables
brain start && brain status       # "ok": true
```

## Common mistakes
| Mistake | Code | Fix |
|---|---|---|
| creating a `.env` file | B0732 | put the key in `.brain_env` |
| `process.env.API_KEY` in client code | B0702 | read it in `load`/server function, or rename to `PUBLIC_` if it is not secret |
| token-looking literal in client code | B0703 | move it to `.brain_env` |
| server module imported by a component | B0701 | call it from a `server:` function |
| `document.cookie = 'session=…'` | B0713 | `ctx.session.set(…)` on the server |
| empty `BRAIN_SECRET` in production | B0720 | set it in the `[production]` section |
| invalid `server:` function | B0704 | `server: async function name(ctx, data) {…}` at the top level of the script |

## Minimal correct example
```html
<script>
  server: async function logout(ctx) {
    ctx.session.clear()
    return { redirect: '/' }
  }
  const out = async () => navigate((await logout()).redirect)
</script>
<Button on:click={out}>Sign out</Button>
```
