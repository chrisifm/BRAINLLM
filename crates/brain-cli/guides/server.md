# guide: server — sessions, errors, environment, deploy with brain start

## When
Sign-in and sessions, server-only code and secrets, per-environment configuration, error pages, production deploys.

## Experimental resource backend
For typed CRUD resources and durable jobs, read `brain spec --topic server` and the backend part of `brain map`.
Create contracts with `brain add resource posts --fields "title:str! body:str"` or `brain add job notify --fields "message:str!"`.
Inspect one contract with `brain card resource:posts` / `brain card job:notify`, and all resolved schemas with
`brain backend schema --json`. The Rust/Postgres backend has its own sessions and runs separately through
`brain backend serve`. Configure fixed loopback `BRAIN_BACKEND_URL` to connect development/production SSR and
server functions to native identity and resources. `ctx.backend.google.challenge()` and `.login({credential})` bind Google's nonce to the browser. `ctx.backend.login({email,password})` relays native HttpOnly
cookies from a same-origin POST; JWT signing keys stay exclusively in the backend. `ctx.backend.resource(name)`
provides list (filters, sort, cursor, include, aggregates)/get/create/update/patch/remove and `ctx.backend.slugRedirect(name,slug)`
through native schemas and policies (docs/backend/server-queries.md). Resource generation includes typed
collection/detail clients, a discoverable canonical contract, server service and repository. Administrative UI remains
planned in 0.4: the command creates no views, guards or controls and preserves the default 30 KiB route budget.
Build application views deliberately using the project design system. Account recovery/verification recipes need explicit
mail setup. Review policy and migrations explicitly before serving. Never remove an existing
production API merely because the experimental contract compiles.

Mandatory before-write validation uses `brain add hook <identifier>` and `before_create|before_update|before_delete`
in resource hooks. Return `ctx.payload`; native schema revalidation and row-conflict checks remain authoritative.
After-write jobs commit with their mutation. See `docs/BEFORE-HOOKS.md` and `docs/BACKEND.md` for limits.

## Limits
RPC and form bodies, call deadlines and the call rate come from brain.json, the same in `brain run` and production:
`"server": { "maxBodyKB": 1024, "timeoutMs": 10000, "rpcPerMinute": 60 }` (the defaults). `"earlyHints": true` adds 103 responses; keep it off behind nginx below 1.29. A body past `maxBodyKB`
gets 413 naming the setting; a file sent as base64 grows by a third, so a 5 MB file needs about `"maxBodyKB": 7000`
(at most 65536). `timeoutMs` bounds each server function and `load()` (504), `rpcPerMinute` the calls per client IP
and function (429). `BRAIN_MAX_BODY_KB`, `BRAIN_TIMEOUT_MS` and `BRAIN_RPC_LIMIT` in `.brain_env` override them per
environment. `brain check` reports an invalid value as B0707. Pass `ctx.signal` to `fetch`: it aborts on disconnect or deadline.

## Sessions
1. Set `BRAIN_SECRET` in `.brain_env` (MCP: `brain_env_set { key, section, value }`).
2. In a `server:` function: `ctx.session.set({ id, email })` signs it into the `__Host-session` cookie (HttpOnly,
   Secure, SameSite=Lax, 7 days; `ctx.session.set(user, { maxAge: seconds })` to change it).
   That cookie is signed, not encrypted: a token or password in it warns B0737, and one in `console.*`, browser
   storage or a password in a returned value warns B0783. Keep such values in
   `ctx.session.setSecret(value, { maxAge })`, read them with `ctx.session.secret()` and remove them with
   `ctx.session.clearSecret()` (`clear()` too). It is a separate AES-256-GCM cookie, limited to 3.5 KB, that never fills
   `ctx.user`. With `BRAIN_BACKEND_URL` the backend owns the session, so `ctx.session.set` answers 409 and `brain check` warns B0786:
   sign in with `ctx.backend.login(credentials)`, or set `BRAIN_BACKEND_SESSION=app` to keep the signed session and use
   the backend only for data, where the visitor is anonymous unless a service key acts. `setSecret` works in both.
   `ctx.backend.signup(credentials)` creates the account without a session (call `login` after it) and fails with
   `errors.email` or `errors.password`.
   Rotate the key with `BRAIN_SECRET_PREVIOUS=<old>` (comma separated) until old cookies expire; bump
   `BRAIN_SESSION_VERSION` to sign everybody out; `BRAIN_SESSION_SECONDS` sets the default length. Logout invalidates a
   copied cookie only inside one server process: with several processes, the version is the way to revoke.
3. Every later request sees it as `ctx.user` in guards, `load` and server functions. `ctx.session.clear()` logs out.
4. Protect a folder: `brain add guard account` → `export default (ctx) => ctx.user ? true : redirect('/login')`.
5. Never set the session cookie from browser code (B0713); never keep the signed-in user in `store(…, { sync: 'storage' })` (not machine-checked).

## Server-only code (hosted models: `brain guide llm`)
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
- `.brain_env` is never committed (B0725); `.brain_env.example` lists its keys and the keys the code reads (B0727 when out of sync; `brain doctor --fix` writes it).
- An empty key in the build environment fails the build (B0720). A variable the app works without is declared `KEY?=`: it may stay empty, and the program sees it as unset. A route or webhook whose secret is absent starts disabled: it answers 503, runs no handler and logs a `route-disabled` event; a route secret held too short stops the server (B0790). The line that wins for an environment decides, so `[production]` can require what the shared block leaves optional. MCP `brain_env_keys` lists keys, never values.

## Errors
- `throw notFound()` in `load` → 404 with `views/404.brain`; `throw redirect('/x')` → 303; `deny('reason')` in a
  guard → `views/403.brain`. `_error.brain` is the folder's error boundary (retry button, request id, no stack in production).
- A server function that throws a plain `Error` answers 500 with a generic message; the real one goes to the server log.
  To show a reason, throw `{ errors: { field: 'msg' } }` (422, field errors) or `{ message: 'msg' }` (400, the calling
  form's `f.error`); an Error subclass with a 4xx `status` (`class Conflict extends Error { status = 409 }`) is public too.
- Components are isolated: a failing component shows its fallback (`<meta fallback="Could not load">`) with Retry and
  the page keeps working. Do not wrap everything in try/catch.
- Browser errors captured in development: `brain doctor` and MCP `brain_errors`.

## Deploy (`brain build` then `brain start`, which supervises its own instances; a host that requires PM2 or nginx gets its file from `brain deploy config pm2|nginx`; not machine-checked)
```bash
brain build                       # production into dist/ (minify, CSP + SRI, sitemap, robots, llms.txt)
brain start --port 3000 --instances 2   # static apps served from Rust; apps with server code run Node children
brain status                      # mode, instances, restarts, last exit reason
brain reload                      # zero-downtime: new instances ready before the old ones stop
brain stop
brain start --install             # writes a systemd unit (Linux) or launchd plist (macOS) and prints the enable command
```
An npm-created app pins the compiler in devDependencies, which `NODE_ENV=production` skips: install with `npm ci --include=dev` or `npm run build` stops with `brain: command not found`, and do not prune it afterwards (`brain start` is the same package). Logs go to `logs/app.log` (rotated). Hosted under a sub-path: `brain.json` `"base": "/app/"`. `brain preview` serves `dist/` on a temporary loopback port to look before deploying. Backend deploys run `brain backend migrate` before serving. It does not run backwards, so back up first. A contract change that removes fields stops it with B0787 until `brain backend approve-contract --accept <resource> --apply` records the review: an unattended deploy stops there on purpose. Under PM2, `pm2 restart` keeps the saved `args` and environment: after changing `node_args`, `stop_exit_codes` or `NODE_OPTIONS`, run `pm2 delete <name>` and start it again.

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

Row conditions: `read: [{ public: { where: { status: 'published' } } }, 'owner.shop', 'admin']` limits a grant to matching
rows; grants are OR-ed, and hidden rows answer 404 like missing ones on every read channel. `brain check` warns B0749 on
an unconditioned public read with a state field; add a `where` or declare `expose_all_rows: true`. `api: false` makes a
resource server-only (no routes, no OpenAPI); reach it with `ctx.backend.service(name)` and `BRAIN_SERVICE_API_KEY`. `ctx.backend.files(bucket)` needs that key with a `files.<bucket>` scope. Jobs and routes (`runAs: 'key'|'system'`) get `ctx.backend` too: see docs/backend/job-backend.md, which defines what a system key bypasses.
See docs/backend/ownership.md; for organizations, audit, facets, geo, captcha and credits see docs/backend/<name>.md. Server-side calls from a page should pass the request context, or all visitors share one rate-limit bucket (log event `shared-rate-limit-bucket`). Size the limit with `limits.requestsPerSecond` and `limits.burst` in server/platform.jsbrain. A `load` that throws an error with status 429 or 503 answers 503 with `Retry-After`.
