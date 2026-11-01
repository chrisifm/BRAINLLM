# Changelog

All notable changes to BRAINLLM are recorded here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
and versions follow [Semantic Versioning](https://semver.org/). While the major version is 0, a minor bump may add
diagnostics that reject code an earlier version accepted.

Every npm package (`@christianirack/brainllm`, its seven native platform packages, `brainllm-runtime`,
`create-brainllm` and `create-brainll`) ships the same version as `[workspace.package]` in `Cargo.toml`.

## [0.8.0] - 2026-11-01

Upgrade with `npm install -D @christianirack/brainllm@0.8.0 --save-exact`, then run `brain upgrade`, which measures the
changes against your app. `brain guide whats-new` lists every change with how to start using it, and `brain guide migrate`
is the full upgrade playbook.

### Act on these before upgrading

- **`ne` includes records without a value.** `field[ne]=x` now returns rows where `field` is empty. Nothing fails, so review every use; add `field[exists]=true` for the old result.
- **A file bucket must declare `types`.** Without them, `brain check` reports B0770 and `brain backend serve` refuses to start. Write the accepted types, or `types: '*'` to keep accepting any file.
- **103 Early Hints are off by default.** Set `server.earlyHints` or `BRAIN_EARLY_HINTS=1` to turn them on. Page responses still carry the `Link` header, so a host can produce them.

### What is new and how to use it

| Goal | Start with | Read |
|---|---|---|
| Accounts in an existing or new app | `brain add auth`, `brain new <app> --backend` | `brain guide backend` |
| Frontend and backend from one command | `brain run --backend` | `brain guide backend` |
| Tenant data isolated by the server | a field of kind `org` | `brain guide docs/backend/organizations.md` |
| Who changed which fields | `audit: true` in a policy | `brain guide docs/backend/audit.md` |
| Counts, radius and box filters | `facets`, a `geo` field | `brain guide docs/backend/geo.md` |
| Bot defense on public writes | `server/auth/captcha.jsbrain` | `brain guide docs/backend/captcha.md` |
| Usage balances that cannot overdraw | `/_brain/credits/{subject}` | `brain guide docs/backend/credits.md` |
| Move data from another system | `brain backend import` | `brain guide docs/backend/import.md` |
| Model calls from server code | `ctx.llm.complete`, `ctx.llm.stream` | `brain guide llm` |
| Limit where pages can send data | `csp.connectSrc` in `brain.json` | `SPEC.md` |
| Measure real pages | `vitals()`, `reportVitals(path)`, Alt+Shift+O in `brain run` | `SPEC.md` |
| See what already exists before building it | `brain capabilities`, `brain recipe <id> --apply` | `brain guide docs/CAPABILITIES.md` |

### Known limits

- Organizations reject actions and batch writes, and an organization cannot be deleted yet.
- Credits are administered by an admin only.
- Captcha does not cover email verification or Google sign-in.
- `ctx.llm` does not stream to the browser or call tools, and a call ends at `server.timeoutMs`, 10 seconds by default.
- Speculation Rules apply only to full document navigations.
- Auth emails go out in one language, the default locale.

### All changes

- Fix: `onclick={f}` on a native element compiles to `on:click` (B0223)
- `brain check` warns B0224 on `onclick="..."`, which the Content-Security-Policy blocks
- Fix: `brain start` refuses a native-socket build without BRAIN_BACKEND_URL before printing a URL
- Fix: `brain build` and `brain doctor` agree on native sockets when the site is loopback or a placeholder
- Fix: `brain run` moves focus and announces the new title after a client navigation, and `navigate()` no longer reloads the page
- Fix: `brain upgrade` after `npm install` reads the previous pin from the last commit
- Fix: `brain add channel session` writes the same store as `brain new`
- Fix: `form()` shows `form.rejected` instead of the backend's developer `fix`
- Fix: `brain fix` no longer rewrites `location.assign` to `navigate` in a handler that signs out: B0129 still warns, and the full reload that clears client state stays your decision
- B0777 also warns when a title ends in the bare site name
- Fix: B0765 waits 10 seconds for a worker started at the same time as the server
- Fix: MCP `brain_run_start` reports and opens the declared `localhost` origin
- `brain check` warns B0788 on a member read of a `key()` channel
- `brain check` warns B0789 when a route handler imports `pg`
- Organizations and memberships: a field of kind `org` makes a resource tenant data, isolated by the server on every read and write, with `org`, `org.<role>` and `org.own` grants and invitations.
- An append-only audit log: `audit: true` in a policy records who changed which fields of a row, never their values; organization changes are always recorded.
- Lists count declared `facets` (`?facets=status,tags`) over the rows the reader may list.
- A `geo` field filters by radius and box (`near`, `bbox`) on a built-in index, with no database extension.
- `server/auth/captcha.jsbrain` requires a verified token on signup, login, password reset requests and declared public write routes, and fails closed.
- A credits ledger debits atomically with idempotency keys at `/_brain/credits/{subject}`, and a balance cannot go negative.
- `brain backend import` brings accounts and records from another system, keeping ids, owners, references and dates, with a rollback preview and whole-file refusal (B0798); imported scrypt hashes verify at login and become Argon2.
- **Breaking:** the `ne` list filter now keeps records without a value, matching action guards; add `[exists]=true` to exclude them.
- `brain build --strict` fails when a page ships with a component fallback or a route is left out of `dist/`; the output stays on disk and the JSON report still prints.
- `brain mcp` refuses a stdin line over 32 MiB with a JSON-RPC error and keeps serving; invalid UTF-8 now answers a parse error instead of ending the session.
- A request body that is aborted by the client or cut past its limit now settles its reader, so no request stays pending and no false "body too large" is logged.
- `brain check` warns B0797 on a field named like an internal (`password_hash`, `api_key`, `internal_notes`, `legacy_id`) that every reader receives by default; `docs/backend/ids.md` now says a public legacy number must be declared public, and how to keep one private
- `csp.connectSrc` in brain.json lists the origins pages may call: it replaces the broad `https: wss:` of the production `connect-src`, the site's own origin always stays allowed, and an origin a page calls outside the list warns B0746. Without the list nothing changes and `brain check` warns B0795; an invalid entry is B0796. New apps start with `[]`
- Breaking: a file bucket without `types` (or with `types: []`) is now error B0770 in `brain check`, and `brain backend serve` refuses to start. Write the types it holds, or `types: '*'` to keep accepting any file
- `brain build --json` and the `brain_build` MCP tool report each stage's wall and CPU time and the work it executed, with or without terminal progress
- The build cache scans its directory a logarithmic number of times per build and keeps the entries the running build uses
- A build reads each file's dependency hash and import summary once per source revision, and warns when a source changes while it runs
- The SEO audit compares a page only with the topics it can mention; 4000 pages went from about 23 s to about 2 s of CPU with the same findings (measured on one machine under load)
- Prerendering inserts fragments and walks siblings in linear time
- Fix: a build that fails on a file `brain check` does not reach names its location and real fix; `brain build --json` and `brain_build` report `phase` and `diagnostics` with the fields of `brain check`, marking locations outside the app as `generated`
- Fix: a page rendered per request carries the site's head and its social card (`og:title`, `og:description`, an absolute `og:image`, `twitter:card`, the robots hint, theme color, manifest and feed links) like a prerendered page; a page behind a guard answers `x-robots-tag: noindex` and stays without the site's search and social tags
- `<meta seo query="page">` keeps the listed request parameters in the canonical of a page rendered per request; every other parameter stays out, and a contradictory declaration warns B0503
- `brain add resource --policy owner|public-read` starts a resource with a common policy instead of admin-only.
- B0735 now points at the policy or roles file that is wrong instead of `server/`.
- `BRAIN_VIEWER_PROFILE` and `BRAIN_VIEWER_SERVICE` can live in `.brain_env`; the process environment still wins.
- The server guide no longer forbids PM2 while `brain deploy config pm2` generates its file.
- Form rules gain `tel` and `file`: a phone shape, `file(image/*,.pdf)!<=5000000` for accepted types and size in bytes, with `form.tel` and `form.file*` messages.
- A cross-field `validate` that returns an empty message no longer blocks the submit with no visible error; the option is now documented.
- `brain capabilities` prints the catalog of playbooks, generators, recipes and backend types from the compiler's own tables; `docs/CAPABILITIES.md` is its output, a test fails on drift, and a new app's `AGENTS.md` lists a summary.
- `brain guide whats-new` prints what changed from the release manifests, `brain guide migrate` is the upgrade playbook, and `brain guide docs/<path>` serves every public reference page from the binary, so npm installs read them too.
- `brain guide backend` and `brain guide analytics` are new playbooks with steps, verification and common mistakes.
- `brain add auth` adds signup, login and account pages, the session channel, the auth configuration and the environment keys to an existing app and overwrites nothing; `brain new <app> --backend` starts an app with them.
- `brain run --backend` (and `brain dev --backend`) starts the native backend beside the frontend, waits for it to be ready, refuses a port another process answers, follows `--port` with the backend's public URL and allowed origin, and stops it on exit. `brain backend migrate --create-db` creates a missing local database.
- `brain recipe <id> --apply` writes the files a recipe previews and never overwrites one.
- Speculation Rules for document navigations: pages prefetch public static targets on hover where a click loads a new document, and never a target the router takes; opt out with brain.json `"speculation": false` or `data-prefetch="false"`.
- AVIF media renditions: a preset can use `format: "avif"` with a required PNG, JPEG or WebP fallback; the default plan is unchanged and the native binary grows about 1.2 MB.
- Dev observability panel: Alt+Shift+O or `?brain-observe` in `brain run` shows Web Vitals, requests, long frames, errors and live components, and names an element's source file; a production build holds none of it.
- Core Web Vitals reporting: `vitals()` and `reportVitals(path)` measure LCP, CLS, INP, FCP and TTFB, opt-in with no bytes when unused.
- Server LLM module: `ctx.llm.complete` and `ctx.llm.stream` call Anthropic and OpenAI from server code with keys from the server environment; `ctx.llm` in browser code or a `PUBLIC_` provider key fails with B0920, and streaming to the browser is documented as absent.

- Fix: the Node server no longer sends 103 Early Hints by default; set `server.earlyHints` or `BRAIN_EARLY_HINTS=1` to opt in, never for HTTP/1.0
- Fix: page responses carry the route preloads in a `Link` header, so hosts can still produce early hints
- Fix: with `NODE_ENV=production` and no `--env`, `brain jobs`, `brain backend`, `brain media` and `brain analytics` stop with B0785 instead of B0762 about a variable set under `[production]`; `brain analytics` used to skip the check, and `--env=production` counts as stating it
- B0762 names `[production]` when `DATABASE_URL` is only set there
- Fix: a missing or short route or webhook secret stops `brain backend serve` and `brain jobs work` once, before any database connection, with B0790 and exit status 78; the generated PM2 file and `--with-workers` do not restart that status
- Verification and password emails take their words from `auth.email.*` in the default locale, and `brain doctor` reports B0941 when an app with a non-English locale leaves them in English
- `brain backend serve`, `brain jobs work` and `brain jobs cron` wait up to 60 s on a refused role or missing database, instead of a restart storm under a process manager
- B0940 warns when `app.brain` has no `<head>`, where a build or the server silently dropped every title and head tag
- A JSON body of the wrong shape answers 422 `INVALID_INPUT` naming the key in `error.fields`, on signup and on every other fixed-shape endpoint
- The deploy guide and docs/DEPLOYMENT.md state that a server with `NODE_ENV=production` installs with `npm ci --include=dev`
- `brain explain B0720` names `KEY?=`, and B0720 and B0790 say that it does not make a route or webhook secret optional
- Fix: `brain start` answers `/health`, `/health/live` and `/health/ready`, reserved by B0735, on its own port: forwarded to the native backend, or a plain healthy response without one

## [0.7.0] - 2026-10-31

- `brain dev` recompiles the component when a CSS edit changes its scope identity (a customized copy of a kit file); it used to keep the old script with the new `data-b` CSS.
- In production `ctx.session.set` under a native backend answers visitors with a generic message; the configuration detail
  goes to the log.
- A route handler or before-hook that fails at startup names the import failure instead of only asking for a callable export.
- A native socket no longer loses its final bytes when the backend closes first: the bridge used to drop up to 64 KB still
  queued for a slow browser.
- Guides say which prohibitions no diagnostic enforces ("not machine-checked"), and a text color using `color-mix()` is described as judged by its contrast, not forbidden
- `brain seo audit` warns B1855 on a redirect by meta refresh, which answers 200 with the old URL; a refresh that only reloads the page is not reported
- `brain doctor` warns when the app targets a version older than the installed compiler and names the `brain upgrade` command to run; it never fails the run
- Fix: `brain check` points B0786 at the exact column, B0750 at the route's `route.jsbrain` instead of `server/routes/`, and a handler holding a secret reports B0773 once, without a second B0735
- Fix: the `brain upgrade` Markdown report lists the steps and reason of changes with no location in the app, and why a gate did not run; they appeared only in `--json`
- Fix: a file whose bytes could not be removed is no longer orphaned; `_brain_file_deletions` keeps the delete and `brain backend serve` retries it until the bytes are gone. Run `brain backend migrate`
- `brain check` warns B0790 when a route or webhook secret is missing or too short, instead of `brain backend serve` failing later; a route without its secret is still documented
- Fix: `brain check` and `brain backend schema` report an invalid route contract as B0735 (it was dropped silently)
- `brain check` reports invalid `brain.json` analytics settings and a route handler or before-hook file that does not exist as B0735, instead of `brain backend serve` failing later
- Fix: a non-numeric `analytics.retentionDays` or `eventsPerMinute` is refused instead of silently keeping the default
- Fix: internal links, forms, `navigate()` and `redirect()` on a page outside the default language stay in that language
- Fix: a link to a server-rendered page under a language prefix no longer fails the build with B1819
- B1850 warns when a page shows framework messages and the app's non-English default language lacks their `form.*` or `error_state.*` keys
- Fix: `brain run` loads the dictionary of a single-language app, so `form.*` messages and `t()` match the production build
- `brain new` puts the menu in `views/_layout.brain`, so it can be translated; `brain add view` links there, or in `app.brain` for apps that keep it there
- B1403 also reads `<script>` literals a person sees: variables markup only displays, `alert`, `confirm`, `prompt`, `toast.send` and `.textContent`
- The sitemap warning without a site URL points at `PUBLIC_SITE_URL` and lists the `sitemap()` routes it skips
- Fix: reading an undeclared `PUBLIC_*` variable in the browser gives `undefined` instead of throwing
- Fix: B0721 and B0722 see keys read by destructuring `process.env` or `ctx.env`
- `ctx.signal` in `load()`, guards and server functions aborts when the client disconnects or the deadline passes
- Fix: a newer client navigation aborts the page request it replaced
- Fix: `t()`, `fmt` and `locale()` inside `load()`, guards and form handlers read the page's language, in production and in `brain run`
- Security: `GET /api/<r>/<id>` of a row outside the reader's scope answers 404 like a missing id instead of 403, and a role with no read grant answers 403 for every id, so neither confirms which ids exist
- Security: a write whose author may not read the row (`update: ['owner']` with `read: ['admin']`) returns the `id` and `data: {}` instead of the row
- `brain backend logs <rid>` names the operation and resource of every 5xx, and keeps the stack of a `BACKEND_INTERNAL`, which only reached stderr; a binary without symbols says so instead of listing frames
- Fix: a route handler that throws a 4xx `status`, `notFound()`, `gone()` or `redirect()` answers that status instead of a 500 or a `200` holding the helper object
- Lists name aggregates that hit the 100-group cap in `aggregates_truncated`
- Request log lines carry `dbMs` and `dbWaitMs` for resource reads, so database time and pool wait are visible apart from `ms`
- Fix: plain lists and `created_at`/`updated_at` sorts use their index instead of sorting the whole table on every request
- Declared sorts get an index for their other direction on the next `brain backend migrate`
- `?search=` with one or two punctuation characters finds nothing without scanning, like longer punctuation
- `brain backend serve` warns B0771 when `/_brain/api` is open to anonymous callers, unless you chose `--env development`
- `brain check` reports a handler with no default export (B0772) or a secret-shaped literal (B0773) that used to fail only at boot
- `brain jobs work` refuses those two handlers at start, as `brain backend serve` already did for routes
- Declared `or` groups in `query.jsbrain` (`?group.field[op]=value`), indexed and bounded, validated by `brain check`
- `?owner=me` lists the reader's own rows; the id comes from the session, never from the client
- `?fields=a,b` narrows the `data` of each list item, after policy and visibility; hidden fields answer like undeclared ones
- `?include=a.b.c` follows references up to three levels; each level declares it and applies its own read policy, at most 500 records per request
- Fix: a project API key needs a read scope for every resource reached by `include`, related filters and `count` (they answered with only the list's scope)
- System identity for jobs and routes: `keys create --system` (CLI only), `runAs: 'system'` in `queue.jsbrain` and route contracts; each request is marked `system` in the key's events
- `ctx.backend` in route handlers with `runAs: 'key'|'system'` in the contract
- `ctx.backend.files(bucket)` in jobs, routes and server code; `/_brain/files` accepts keys with a `files.<bucket>` scope
- `brain run` now records requests, payloads, responses, timings and errors in a local store, with one id from browser to server to backend; read it with `brain trace`, `brain trace <requestId>` or `brain trace issues`.
- `brain trace` shows server exceptions and browser errors at their `.brain` line, CSP violations production would block, cross-origin failures, broken sockets, backend queries and leak findings, with B1840-B1844 and a fix.
- The capture never reaches a production build, hides passwords, tokens, cookies and secrets before writing, caps size and age, and turns off with `brain run --no-trace` or `"trace": { "enabled": false }`.
- A new module records with `__brainTrace(module, kind, fields)` or one line in `.brain/trace-spool/`, with no schema change.
- Roles are general: only `admin` is required, a new app has `admin` and `user`, and people who sign up get `user`, so `create: ['user']` admits them. Apps with no roles file whose grants still name `company` or `candidate` keep the old implicit roles and signup role, with warning B0739.
- `brain add roles` declares the roles an app already has; `brain backend rename-role --from <role> --to <role> [--apply]` moves accounts explicitly, and `brain backend migrate` reports accounts whose role is not declared.
- B0735 names the real file and a concrete fix for role errors. B0738 warns when a grant names an undeclared role in field visibility or a route, or when no grant names the signup role.
- Domain vocabulary from one app removed from defaults, messages, docs and examples; `scripts/check-general.sh` keeps it out.
- A bucket can state `types: '*'` to accept any file; a bucket that omits `types` keeps accepting everything and now warns with B0770. `.docx`, `.xlsx`, `.pptx`, `.doc`, `.xls` and `.ppt` are told apart from plain ZIP and OLE containers, and common type aliases (`image/jpg`, `audio/mp3`, `application/x-pdf`) match their canonical type.
- `console.error` and `console.warn` stay in the build (B1308 only removes the debugging calls). B0722 counts keys named as quoted literals under `server/`, and `BRAIN_JOBS_API_KEY` and `BRAIN_SERVICE_API_KEY`.
- A `str` field can declare `pattern`: the whole value must match on every write, or the write answers 422. Adding one needs no migration.
- `transitions.jsbrain` accepts `editable: { field: [states] }`: that field changes only in those states, on every write path.
- Actions and transitions accept `when_any` (at least one guard group holds) and `owner.<ref>[.<ref>]` in `roles`.
- API keys work on `/api/<resource>/<id>/actions/<name>` and `/transitions/<name>`, and `ctx.backend.resource(r)` gains `.action()` and `.transition()`, so a job can update a counter atomically.
- A route handler can return `{ file: { base64, name, type } }`: always an attachment, with `nosniff` and a sandboxing policy.
- Backend docs corrected: route `ctx.user` is `{id, role, verified}`, `ctx.env` is documented, OpenAPI lives at `/_brain/api`, migration does not add constraints to a registered resource, and API keys cover batch and upsert.
- `brain <command> | head -1` ends quietly: a closed pipe no longer reports an internal error, on any command.
- `--help` on any command, at any position, prints its usage and acts on nothing; `brain build --help` used to build and `brain new x --help` used to create the app.
- `brain add` without arguments lists every kind it can create.
- B0780 warns on a `}` in markup that closes nothing (`<p>{x}}</p>`) and on `{{x}}`, with the exact edit; an unclosed `{` now names the tag it ran into.
- B0781 reports `await` with no enclosing `async` function, including nested functions and the top of a component script: that code emitted JavaScript that does not parse.
- B0737 names every credential in `ctx.session.set` and reads `d.password`; B0783 warns on a credential in `console.*` or browser storage, and on a stored password returned by a server function, `load` or a route handler.
- `*.test.brain` under `views/` no longer gets B0501 when it is checked on its own: a test file has no route.
- Every command refuses a flag it does not know, and acts on nothing: `brain test --port 5280` used to read the value as a path and report green.
- `brain test` fails on a path that does not exist, is not a test file or selects no test, on a file that declares no `test()`, and on more than one path.
- With native sockets, `brain test` and `brain doctor` run against the app's own `brain run` on the declared port; another process there is named, with how to find it.
- `brain doctor` no longer says no errors were captured when the tests did not run.
- `--env=<name>` and `brain seo --since=<date>` take effect; they used to be ignored in that spelling.
- `/_brain/analytics.js` answers under `brain run` with native sockets and under `brain start`, and its link keeps the base path.
- `stub()` and `reply()` in `*.test.brain` answer a `server:` function or an endpoint without a server.
- In `*.test.brain`, `#id` finds a component's element on every mount of the file, not only the first.
- A rejected submit puts focus on the first rejected field, or back on the control `disabled={f.busy}` took it from; it used to fall to the page body.
- Leaving a form that holds a password field no longer asks to discard changes.
- `ctx.backend.signup(credentials)` creates a native account from a server function; a refused address is `errors.email` and a bad password `errors.password`.
- `brain card resource:<name>` lists the response shapes, including when `owner` is present.
- A view over `budget.viewKB` is reported once, at its largest page, with JS, CSS and HTML broken down.
- B0736 calls a loopback site URL this machine's own address instead of a placeholder.
- A build refused by `sockets.allowedOrigins` names its environment and where the site URL came from.
- A `DATABASE_URL` without a user connects as the system account, as `psql` does; it used to try the role `anonymous`. B0762 connection failures now say the host, database and user tried and why it failed, never the password.
- B0721 and B0722 read `process.env`, `ctx.env` and `import.meta.env` from tokens: a comment, a string, prose or a CSS rule that mentions a key no longer counts as a read, and a guard in a later statement (`const k = ctx.env.K; if (!k) ...`) makes the key optional.
- A session write without `BRAIN_SECRET` fails with B0782 and how to set it.
- `brain dev` refuses the redirects production refuses (an invalid status, an off-site or malformed location), thrown or returned from `load`; it used to answer them and production then failed with a 500.
### Security
- `brain backend capabilities` and the MCP tool `brain_backend_capabilities` list field types, filter operators, policy grants and limits, generated from the code.
- `brain backend approve-contract` previews a changed resource contract with the rows it touches and records it on `--apply`; changes that can invalidate stored rows also need `--accept <resource>`.
- `brain backend bootstrap-admin --email <address>` selects the account by email.
- `ctx.backend.resource()` failures carry the backend's `code` and `fix` for server code to branch on; the message stays generic.
- `BRAIN_BACKEND_SESSION=app` keeps an app's own signed session beside the native backend; `ctx.session.set` that would answer 409 now warns B0786.
- A batch or write that hits a database time or size limit answers an actionable code (503 `BATCH_TOO_SLOW` or `DATABASE_BUSY`, 413 `VALUE_TOO_LARGE`) that says nothing was written, never `BACKEND_INTERNAL`; upserting an existing row uses the unique index instead of scanning the table.
- `brain backend` commands stop with B0785 when no `--env` was given and the database is remote or `NODE_ENV=production`, `users` actions honor production TLS, and a proxied `/_brain/api` request needs an administrator in every environment.
- The refresh grace window is configurable with `BRAIN_AUTH_REFRESH_GRACE_SECONDS` (0..60, default 10; 0 turns it off), and reuse detection stays on.
- `brain doctor` and `brain check` warn on a public `trustProxy` range or none behind a proxy (B0784), a production `brain backend serve` refuses a public range, and `trustPublicProxy` declares a CDN.
- In production, a refused list query no longer lists the declared filters and sorts in its 422; outside production it still does.
- Google sign-in links an existing account when Google hosts the address (`gmail.com` or a Workspace `hd`); a never-verified account also loses its password, sessions and recovery links, and any other address is denied with the same `AUTH_DENIED`.
- Concurrent JWT refresh inside a 10-second grace no longer revokes the family, so tabs and prefetches keep their session, and the SSR bridge renews an expired access cookie during render.
- Failed sign-ins and reset mail are counted per email and client with a cap per email, so a third party can no longer lock out the owner from another client.
- `ctx.session.setSecret` stores server-only values encrypted; `BRAIN_SECRET_PREVIOUS` rotates the key, `BRAIN_SESSION_VERSION` ends every signed session and `BRAIN_SESSION_SECONDS` sets their length. B0737 warns about a credential passed to `ctx.session.set`.
- Optional variables: `KEY?=` in `.brain_env` declares a variable the app works without. It silences B0720 and B0721, may stay empty, and reads as unset on the server and in the browser.
- B0721 detects reads with a default or a condition (`process.env.X || d`, `?? d`, `if (process.env.X)`, `ctx.env` too) and proposes `KEY?=` instead of asking for a value per environment.
- `brain doctor` no longer rewrites `.brain_env.example`: B0727 reports the pending change and `brain doctor --fix` or `brain fix` writes it. The template is the union of `.brain_env` and the keys the code reads, so it never drops a key in use.
- `BRAIN_MAIL_FROM` accepts `Name <sender@domain>`, the friendly form Resend documents; control characters and unquoted address punctuation in the name are refused.
- Backend startup errors for mail, object storage, the database, pools and the public URL carry a code (B0760–B0764), the variable name without its value, and a fix. `brain check` and `brain doctor` warn about them before start, and `brain backend serve` stops on them before connecting.
- `brain doctor` no longer fails on suspected leak B0601 unless `brain.json` sets `doctor.failOnSuspectedLeaks`; its summary names the gates that failed, and the report has a `failed` list.
- Leak checks wait for in-flight requests before judging an instance (`brain test --heap` and `brain doctor` settle, then abort what remains), and the B0601 fix names in-flight promises and the requests pending at unmount.

- `brain doctor`, `brain backend serve`, `brain start` and `brain run` now list what nothing is running: B0765 a queue with no live worker, B0767 schedules with no scheduler, B0768 active jobs in a queue with no declaration, and B0766 when the database cannot answer so workers were not checked. All are warnings; they never fail doctor.
- `brain check` warns B0769 for a job with a schema and no `handler.jsbrain`, which could be enqueued but never run.
- `brain backend serve` logs a `queue-stalled` line every minute for each queue with due jobs and no worker, or with a job waiting over 5 minutes: queue, pending count and oldest wait, never a payload value. `BRAIN_QUEUE_CHECK_SECONDS` changes the period.
- `brain backend serve --with-workers` also runs one supervised worker per declared queue and a scheduler when there are schedules, for a deployment small enough to run them together.
- `brain jobs stats` with no queue lists every declared queue, and any other queue that still holds jobs, with `oldestWaitSeconds`, live `workers` and `declared`. `brain jobs work`, `enqueue`, `dead` and `retry` without a queue name list the declared queues instead of `invalid queue`.
- A job finishes when its handler settles. A handle left open, such as an idle database pool, kept each job alive until it closed, about 10 s per job; a job that held one now takes under 200 ms. Upgrading: work the handler does not await is cut off 100 ms after it settles, so await everything.
- An idle worker starts a new job within about 50 ms through Postgres `LISTEN/NOTIFY`, with polling as a fallback. Upgrading: run `brain backend migrate` to add the `_brain_workers` table.
- A restart under a process manager no longer fails with `Address already in use`: `brain start`, `brain backend serve` and the generated `server.mjs` wait up to 10 s for the port, and `brain run` waits 1 s before moving to the next port. Two live servers never share a port, and the error now names the live holder and how to find it.
- A job or server module with a local variable named like a runtime helper, such as `const key`, no longer fails the build with `imports /@brain/brain-ch.js, which the build did not emit`.
### Navigation, forms and views
- After a client navigation focus moves to the page heading (or the URL's fragment target, or the region) and `#brain-announce` reads the new title; a focus the new page set itself is respected and scroll behaviour is unchanged.
- New apps start with a skip link to `#brain-view` in `app.brain`.
- A rejected form no longer says "Check the highlighted fields." when nothing is highlighted: a 422 without fields shows the backend's fix or a plain sentence, errors naming no field of the form become `f.error` (`form.rejected` when empty), also without JS, and `<ErrorState>` has `error_state.rejected`.
- Error pages always say what happened: `_error.brain` and `views/403|404|410|5xx.brain` without a `<meta seo title>` get a default title and `noindex`, in the build, in production and in dev; a view's own values win.
- A status page opened directly answers its status: `/403`, `/404`, `/410`, `/5xx` and their `.html` return that code instead of 200 when the app has the page, and the new app's `views/403.brain` no longer says "Missing permission: not specified" when no permission was named. Static hosts keep their own rules.
- `liveResource` no longer looks stuck when the session is gone: a refused connection reads the resource once per outage, and a 401 ends in `auth-required` instead of spending the retries.
- `liveResource` says why a public page has no socket: rooms with a null scope report `anonymous` instead of `idle`, `scope: () => null` is reported by `brain check` (B0775), and `brain guide sockets` covers who can connect, every state and session changes.
- `<Input>`, `<Select>` and `<Checkbox>` bound to a form field take its name, as a native field does, so a form without JavaScript sends them too.
- `/favicon.ico` redirects to `public/favicon.svg` when the app has only the SVG icon, in production and in dev.
- `brain check` reports a link nothing answers (B0776, one finding per source line) and a view title that repeats what `titleTemplate` adds (B0777); before, the first waited for `brain build` and the second went unnoticed.
- A link inside an island whose chunk failed still navigates instead of losing the click, and a lazy component in `interaction` mode acts on the key that loaded it and never replays a press from before a failed import.
- The new app's `/login` sends a signed-in user to `/account`, and `brain guide views` has the recipe for session-dependent navigation.

### Signature
- HTML documents now carry `X-Generated-By` with the official site and repository in `brain start`, `_headers`, nginx and Caddy. It replaces `X-Powered-By` and adds nothing visible to the page.

### Added
- `brain check` and `brain doctor` catch origin mistakes before they run: B0744 for a trailing slash, path, missing scheme, credentials or wildcard, with the exact replacement; B0745 for a socket origin outside the HTTP origins; B0746 for a page calling an undeclared origin.
- `brain check` warns B0749 on an unconditioned public read of a resource with a state field; `expose_all_rows: true` declares it intentional.
- `api: false` declares a server-only resource with no public routes or OpenAPI, reached with `ctx.backend.service(name)` and `BRAIN_SERVICE_API_KEY`.
- `brain backend users disable|enable|revoke-sessions|disable-identity|enable-identity` and matching administrator routes disable users and identities and end sessions without SQL.
- Declared routes accept `auth: 'internal'` with a rotatable secret compared in constant time, limitable to loopback from the manifest.
- `ctx.backend.google.challenge()` and `.login({credential})` bind Google's nonce to the browser over a server function.
- `BRAIN_JWT_SECRET_PREVIOUS` rotates the JWT signing key and `BRAIN_AUTH_SESSION_SECONDS` sets the native session length.
- B0751: `brain check`, `brain doctor` and `brain backend serve` name the exact backend setting that is missing or out of range, such as `GOOGLE_CLIENT_ID` without `BRAIN_JWT_SECRET`.
- `brain upgrade [--from V] [--to V] [--dry-run|--apply] [--json] [--no-doctor]` (MCP `brain_upgrade`) migrates an
  app between versions: from the recorded `brainVersion` in `brain.json` (now written by `brain new`) to the installed
  compiler, it reports codemods, manual steps with `file:line`, capabilities to adopt with their snippet and the gates;
  `--apply` runs the safe codemods and `brain check`/`brain doctor`. `brain upgrade <exact-version>` keeps the
  package pin preview.
- Build-time zstd: `brain build` writes a `.zst` (level 19) next to a text asset when it is smaller than the `.br`;
  `brain start` and `server.mjs` send the smallest precompressed sibling the client accepts and the emitted Caddy
  block lists `precompressed zstd br gzip`.
- Client navigation always gives feedback: the region gets `aria-busy="true"` and `route()` gains `navigating`
  at once; past 150 ms `#brain-nav` shows (the app's own element in `app.brain`, else a native top `<progress>` bar);
  past 400 ms (was 150 ms) `_loading.brain` replaces the region, so fast pages never flash a skeleton.
  `brain guide views` documents a custom spinner.
- Prefetch runs from an ordered queue: hover and focus first, then visible links in the main content from the top of
  the page down, then the compiler's plan, then `footer` and `aside` links. A link that leaves the viewport before its
  turn is dropped, visible links never warm `fullLoad` routes, and a click never waits on the queue.

### Security
- Read grants accept row conditions: `read: [{ public: { where: { status: 'published' } } }, 'owner.shop', 'admin']`. They are OR-ed and enforced as bound SQL on lists, totals, filters, sort, search, aggregates, counts, includes, relational filters, get, slug history, sockets, `related` files, API keys and `ctx.backend`; a hidden row answers exactly like a missing one.
- Writes no longer return a row their author may not read: create, update, patch, upsert, batch, action and transition responses keep the `id` and send `data: {}`.
- `related` file buckets evaluate the same rule as a get on the linked record, through one shared evaluator, so an `owner.<ref>` owner who can read the row can read its file.
- A write the policy does not grant answers `403 POLICY_DENIED` when the writer may read the row, and exactly like a missing row when not, on update, patch, delete, batch, actions, transitions and owned operation steps.
- Anonymous `/meta` of a public file bucket no longer returns `owner`, `resource`, `record`, `field` or `sha256`.

### Fixed
- `POLICY_DENIED` explains which rule classes were evaluated, without revealing data.
- `brain backend logs [rid]` (MCP `brain_backend_logs`) returns failure events kept by `brain backend serve` in
  `.brain/backend/failures.jsonl`: SQLSTATE, resource, operation, branch and code location. Stack frames that a stripped
  binary cannot symbolize are omitted, and production events never quote values.

### Fixed
- Backend search over long text no longer answers 500: typo matching reads the first 256 characters of each field under
  a 400 ms budget, `brain backend migrate` rebuilds older rows once, and a search that cannot finish answers
  `503 DATABASE_BUSY` with `Retry-After`. `degraded: true` marks a typo search that ran out of budget.
- Backend search `total` is exact for full-text results and stop words find nothing. Typo matches now appear only when
  full text finds nothing, and `total_approximate: true` marks a capped typo total. Full-text pages no longer rescan long
  text for a tie-break.
- A route handler returning `{status: 5xx, body}` now answers that status instead of 200, replacing the earlier
  assertion that handlers cannot fake server errors. A status outside 200-599 answers `500 ROUTE_INVALID_STATUS`, and a
  lone `{status: 'open'}` is plain data rather than an empty response.
- A malformed id through `ctx.backend` is now 404 on pages and in jobs, not a 400 or an error view with the raw message;
  development pages honor deliberate `HttpError` statuses as production does. `GET /api/<r>/<bad-id>` answers
  `400 INVALID_ID` and names the UUID format.
- Text Postgres cannot store (a NUL character) answers `422 INVALID_INPUT` on reads and writes, and a malformed cursor
  answers 422. Include failures keep their SQLSTATE and can answer `503 DATABASE_BUSY`.
- B0119 reports an imported module that exists but does not parse as such, with its first error, instead of a missing file.
- The B0110 fix for a classic `for` shows the form for bounded retries.
- A refused origin no longer receives `Access-Control-Allow-Credentials`, `-Expose-Headers`, `-Allow-Methods`, `-Allow-Headers` or `-Max-Age`; only allowed origins do, and `Vary: Origin` stays.
- `brain run` with `localhost` in `backend.sockets.allowedOrigins` no longer stops on its own port: a loopback bind serves that origin. A real mismatch prints `computed=… allowed=[…]`.
- Auth errors tell bad input from a busy server: a busy database answers 503 `SIGNUP_UNAVAILABLE` or `LOGIN_UNAVAILABLE`, a malformed email at sign-in answers the same 401 as any failed sign-in, and a sign-in refused for its origin says B0740 instead of "Wrong email or password".
- Every origin refusal now names its code, the seen and allowed origins, the key and the exact fix in server logs and development answers (B0740–B0743); production answers carry only the code and never the allowlist.
- `brain dev` explains the browser's opaque CORS, `connect-src` and socket failures in the console (B0747, B0743), and production bundles do not carry that code.
- `brain backend serve` names the auth public URL, what it resolves to and the `--origin` values when they do not match (B0748), and origin errors from `brain.json` name the value and key without echoing credentials (B0744, B0745).
- The same-origin gateways now forward the client address: each appends the peer it observed to `X-Forwarded-For`, so with `trustProxy: ['127.0.0.1', '::1']` every visitor gets its own rate-limit bucket and a browser-supplied header cannot choose one. The generated `server.mjs` resolves `ctx.req.ip` the same way from `trustProxy` or `BRAIN_TRUST_PROXY` and ignores `X-Forwarded-For` without them; generated PM2 files set the variable. Trust ranges wider than `/8` (IPv4) or `/16` (IPv6) are refused.
- `/_brain/files` now streams through the gateways up to the bucket limit, with `Range`, `If-Range`, conditional requests, `206`, `304` and a client disconnect closing the backend connection, instead of failing above 20 MiB and refusing uploads above 64 KiB.
- Routes declared in `server/routes` are now reachable through the gateways, with the raw body byte for byte; only declared routes are proxied. New warning B0750 reports a route that hides a view, a `public/` file or a framework prefix, or starts with a parameter; the gateway does not proxy that route, so the page or file keeps answering.
- The gateways now relay `ETag`, `Last-Modified` and `Cache-Control`, forward `If-Match` and `If-None-Match`, and accept the 1 MiB batch body.
- API keys now cross the gateways and the dev proxy: `X-API-Key` was dropped, so a valid key got `403 ORIGIN_DENIED` on writes and an invalid key read public rows as anonymous.
- `brain start` answers 404 for a request path starting with `//`. It answered 500 for `//`.
- Service worker caches are scoped to the app: under a base path they are named `brain/app-<sig>` and `brain/app:*`, so
  activating or opting out one app no longer deletes another app's caches on the same origin, and `sw: false` deletes
  only the app's own caches instead of every cache on the origin. An app at the root keeps its cache names.
- Service worker cache writes for hashed chunks and public files are registered with `waitUntil`, so they finish before
  the worker can stop; a failed write no longer surfaces as an unhandled rejection.
- Client navigation no longer flickers or swaps a stale page: one navigation token guards every late callback, so a
  superseded navigation never swaps, never runs its View Transition callback, never scrolls or moves focus and never
  mounts its loading view; a fetch, parse or swap that throws becomes a full page load; a click reuses a prefetch
  still in flight (even a no-cache page) instead of fetching the page twice.
- The first click on an internal link always navigates: links are no longer held until the view's chunk mounts, a page
  without a mount script no longer inherits the previous page's click hold, a failed prefetch only costs a new request,
  and a navigation that gets no answer in 8 s becomes a full load.
- Client navigation no longer breaks the CSP after the first page: the router keeps the nonce the document was served
  with, instead of reading the first `script[nonce]`, which could be a data script adopted from an earlier response.
- Stopping the History router now removes its service worker `message` listener, as the Navigation API branch
  already did, so repeated start/stop keeps no listener.
- Scoped CSS keeps pseudo-elements (`::backdrop`, `::before`, `:after`, `::-webkit-*`) outside the root anchor's
  `:is()`, so Chrome no longer drops the whole rule list for components that style them at their root.
- Conditional content passed to a named slot (`{#if}<a slot="footer">…{:else}<span slot="footer">…{/if}`) now
  renders in that named slot instead of the default one, and the `slot` attribute no longer reaches the DOM.
- `npm/smoke-registry.mjs` reports invalid arguments, versions and deadlines as structured `invalid-arguments`,
  `invalid-exact-version` and `invalid-deadline` failures instead of only printing the usage text.
- `brain seo indexnow` decodes every XML entity in sitemap `<loc>` values (`&apos;`, `&quot;`, `&lt;`, `&gt;`, not only
  `&amp;`), reads a child listed twice once, and fails with a fix when a sitemap index lists no children or a child is
  not a complete `<urlset>` (S02), instead of sending escaped URLs or silently skipping a truncated child.

### Tests
- S01: sitemap byte limits are covered with image, video and hreflang children, escaped non-ASCII text and an entry
  larger than a whole file (rejected, never written into an oversized child).
- S03: a paginated listing with a translated page-2 slug gets self and reciprocal hreflang per page number in the
  built head tags and the sitemap; page 2 never points at page 1.

## [0.6.1] - 2026-10-31

### Fixed

- `brain doctor` on a newly created app passes again: the placeholder production origin in `.brain_env` is reported
  once by B0736, and SEO audits the pages as they would ship with a real domain instead of also scoring relative
  canonical and `og:image` URLs, which pushed the home page below `seo.minScore` (0.6.0 regression).

## [0.6.0] - 2026-10-31

### Added

- Versioned change manifests ship inside every binary (`crates/brain-cli/releases/<version>/<id>.json`): for each
  release, the new capabilities with why they matter, the smallest adoption snippet and the guide to read, plus
  deprecations and breaking changes with the files they affect, manual steps, diagnostic codes and a codemod id. A test
  fails when a CHANGELOG bullet has no entry (CONTRIBUTING "Release: add a change manifest entry").
- `ctx.backend` speaks the full HTTP list contract from server code: `list({filter, sort, cursor|after, include,
  aggregate, count, search, limit, offset})`, `get(id, {include})`, `patch(id, partial)` and
  `slugRedirect(resource, slug)`. Field errors from the backend reject with `status 422` and `errors`, so a server
  function can hand them back to its form. `list()` without options now uses the backend defaults (25 rows). See
  `docs/backend/server-queries.md`.
- Native backend multi-process mode: `cluster: { shared: true }` in `server/platform.jsbrain` (on by itself when
  `BRAIN_BACKEND_PROCESSES` is above 1) counts route `rate` windows and the API-key per-minute limit in Postgres
  (`_brain_rate_windows`, subjects stored as digests) and relays socket invalidation events between processes with
  `LISTEN/NOTIFY`. See `docs/backend/cluster.md`.
- Response compression in the production server: static and SSR HTML (streamed pages included), JSON and error
  pages are negotiated per request (zstd on Node 22.15+, Brotli, gzip by `Accept-Encoding` weight) with
  `Vary: Accept-Encoding`; static files get the smallest precompressed sibling the client accepts. Before, every HTML
  document left `server.mjs` uncompressed; the scaffold home page drops from 10,492 to 3,741 bytes with Brotli. Opt
  out with `createServer({ compress: false })` or `BRAIN_COMPRESS=off` (docs/PERFORMANCE.md).
- Native backend HTTP caching: every successful `GET /api/...` sends an `ETag` and answers `If-None-Match` with 304;
  `server/cache.jsbrain` declares `maxAge`, `staleWhileRevalidate` and `staleIfError` for anonymous reads of
  public-read resources; `PUT`/`PATCH`/`DELETE` honor `If-Match` and answer 412 `PRECONDITION_FAILED` on a stale
  record. See `docs/backend/caching.md`.
- Job handlers get `ctx.backend` with the same calls, authenticated by a scoped project API key in
  `BRAIN_JOBS_API_KEY` (removed from `ctx.env`). API keys now also work on the read-only slug redirect route
  `/api/<resource>/redirects/<slug>`. See `docs/backend/job-backend.md`.
- Full-text search takes a language and field weights per resource: `query.jsbrain`
  `search: { language, weights }` picks a built-in Postgres configuration (stemming, stop words) and A-D weights.
  Changing either reindexes rows in place on `brain backend migrate`; resources without it are untouched. See
  `docs/backend/search.md`.
- Guide for keeping numeric ids from another backend as a unique, filterable field (`docs/backend/ids.md`).
- `brain risk <target> [--measure] --json` and MCP `brain_risk` measure the risk of an edit before it is made: a
  deterministic 0..100 score with named factors (dependents, routes, public contract, backend schema, covering tests,
  checker errors, SEO/i18n/a11y surfaces, styles and, with `--measure`, budget headroom per affected view) and the
  exact `requiredChecks` that prove the edit safe. `brain preflight <target>` now fills `risks` with the static score,
  and the scaffold AGENTS.md tells agents to run it before editing shared code.

- Service worker data routes: brain.json `"cache": { "routes": [...] }` declares `cache-first`, `network-first`,
  `stale-while-revalidate` or `network-only` per same-origin data path, with `maxEntries`, `maxAge` and a
  network-first `timeout`. Personal responses (Authorization, `Vary: Cookie`, `Set-Cookie`, `private`/`no-store`,
  non-200) are never stored; removed routes drop their caches. See `docs/BROWSER-DATA.md`.
- User-controlled app updates: `"cache": { "update": "prompt" }` keeps a new service worker waiting; `updateReady()`
  and `applyUpdate()` let the page show a reload prompt (0 bytes for views that do not use them).
- WebMCP imperative tools ship (#106): `<meta tools="add_task: Add a task (text:str!, due:date)">` plus a same-named
  function registers the tool with the browser's model context while the component is mounted and unregisters it on
  unmount. Nothing is registered without WebMCP, during SSR or prerender; arguments are validated against the
  declared types before the function runs; a name another mounted instance holds is skipped instead of breaking the
  page. `brain check` no longer reports B0128 for `<meta tools>`, and the new B1704 reports a function that
  destructures other parameters than its tool declares. Only components with tools load `brain-mcp.js`.
- B1837 (warning, `brain build`): a public dynamic view (no guard, not `noindex`) that exports neither `paths()` nor
  `sitemap()` renders per request but no sitemap lists its pages; the warning names the view and the route with the
  fix (`sitemap({ env })`, `paths()` or `noindex`).
- B0309 (warning): a style rule outside any `@layer` in `tokens.cssbrain` beats every view and component rule whatever
  its specificity; `brain check` now reports it with the fix (wrap it in `@layer reset`). Custom properties and
  `color-scheme` stay unlayered.
- brain.json `"server": { "maxBodyKB", "timeoutMs", "rpcPerMinute" }` sets the RPC/form body limit, the call and
  `load` deadline and the per-IP, per-function call rate, with the same values in `brain run` and the production
  server (the build writes them into the server manifest). `BRAIN_MAX_BODY_KB`, `BRAIN_TIMEOUT_MS` and
  `BRAIN_RPC_LIMIT` still override them. The schema documents the keys and `brain check` reports an invalid value as
  B0707.

- Behavior analytics on by default for apps with a native backend: a served collector (`/_brain/analytics.js`,
  zero build bytes) records page views with UTM, dwell, scroll depth, sessions, SPA navigations and every interaction
  kind (clicks, keys, focus, input, toggles, submits, clipboard, drag, hover intent, media, rage and dead clicks) in
  batched beacons. It honors GPC, DNT and webdriver, sends no field values, and stores no IP.
- `POST /_brain/analytics` ingestion with size, batch and per-visitor rate limits, bot filtering, anonymous visitor and
  session ids, configurable retention (default 180 days) and parameterized SQL into `_brain_analytics_*` tables.
- `brain.json` `analytics` switch (`enabled`, `consent: "required"`, `retentionDays`, `countryHeader`,
  `eventsPerMinute`), `data-track` attributes and `brain:track` custom events.
- `brain analytics report|funnel|segment|export|map|purge|erase`: per-route and per-element usage resolved to
  `file:line` through `.brain/analytics/elements.json`, never-used UI, time to first interaction, funnels, segments,
  JSON/CSV export and per-user erasure. MCP tool `brain_analytics_report`; doctor info `B0729` when no backend is set.
- Postgres connections set `application_name` to `brain:<app>:<http|worker:<queue>|migrate|cli>`.
- Native project API keys (opt-in `server/api-keys.jsbrain`): `bk_live_<prefix>_<secret>` keys hashed with SHA-256,
  accepted as `X-API-Key` or `Authorization: Bearer` on `/api` resource routes only, `resource:read|write` scopes that
  never exceed the owner's role, per-key daily/monthly quotas in Postgres with `X-Quota-*` headers, a per-key burst
  limit, an append-only request trace with retention, session endpoints under `/_brain/keys`, OpenAPI security
  schemes and `brain backend keys create|list|rotate|revoke|events|summary`. See `docs/backend/api-keys.md`.
- `brain docs [--json] [--out docs/app] [--check]` and MCP `brain_docs` generate deterministic app documentation from
  source: routes with SEO metadata and guards, component contracts with usage examples, stores and server functions,
  every backend contract key, environment key names (never values) and sitemap sources. Hand-written notes in
  `docs/app/notes/<page>.md` are merged. `brain build` keeps existing docs fresh, new apps opt in with
  `"docs": true`, and `brain doctor` warns about stale pages.
- `docs/INDEX.md` is generated by `scripts/gen-docs-index.mjs` from SPEC sections, CLI guides, `docs/*.md` and
  `docs/backend/*.md`; a script test fails when it is stale or a `docs/backend` page is not linked from `docs/BACKEND.md`.
- JSON-LD checks follow Google's rich-result requirements (Search Central, fetched 2026-10-31). Product and
  SoftwareApplication offers need a price, and AggregateOffer needs `lowPrice` + `priceCurrency`. SoftwareApplication
  needs `name` and a rating or review. JobPosting is checked for `address.addressCountry` and its
  `employmentType`/`unitText` enums. An Event Place location needs an `address`.
- Zero-config icons: the build links the icon the app declares in `public/` (`favicon.svg`, else an icon PNG) as
  every page's `rel=icon`, plus an icon PNG as `apple-touch-icon`, unless the page links its own.
- `brain run` serves /robots.txt and /sitemap.xml with no build step: static routes plus the URLs of `paths()` and
  live `sitemap({ env })` views, on the request's origin. A file in `public/` still wins.
- hreflang keeps page identity on paginated listings (S03): page 2 pairs with page 2 in each language (translated
  slugs included) in head tags, the sitemap and the audit, instead of being dropped or paired with page 1.
- `brain seo indexnow` reads a split sitemap through its index (S02): the local `sitemap-N.xml` children, deduplicated
  and sent 10 000 URLs per request. Child sitemap URLs are never sent as pages; a missing, nested or off-site child
  is an error with a fix.
- B0736: a placeholder site URL (the scaffold's `https://example.com`, `*.test`, localhost) is no site in a production
  build. The build warns, names the skipped `sitemap()` routes and publishes no sitemap or robots `Sitemap:` line
  pointing at it; `brain doctor` flags it before deploy. SPEC and `brain guide seo` show a runnable `[slug]`
  `sitemap({ env })` example.
- Sitemaps respect the 50 MB limit as well as 50 000 URLs (§32, S01): the build and the server's dynamic
  `sitemap()` URLs split into more child files by escaped UTF-8 bytes, and a URL over 2 048 characters is left out
  with a warning.
- Client navigation replaces every route-owned head tag (§32): description, canonical, hreflang and Markdown
  alternates, and `prev`/`next` now follow the page; a page without them no longer keeps the previous page's tags.
  Icons, stylesheets and the RSS feed stay.
- Crawler-safe answers (§33): `throw gone()` in `load` answers 410; a redirect rule, the trailing-slash policy
  (`brain.json` `seo.trailingSlash`: `never` by default, `always`, `ignore`) and the canonical host
  (`seo.canonicalHost`) are one 301 hop with the query kept, in `brain start` for SSR and static apps. Guard
  redirects and 403 pages carry `X-Robots-Tag: noindex, nofollow`; a streamed route waits 50 ms for `load()` so a
  quick 404/410/30x keeps its real status instead of a soft 200.

- Backend contracts accept exact `int`, `decimal(p,s)`, `date`, `datetime`, `enum(a|b)` and `list<str[,N]>` fields,
  object field options `{type, default, unique}` and declarative checks in `constraints.jsbrain`. Decimals travel and
  are stored as canonical strings with a lossless database CHECK; unique and check violations answer 409/422. Records
  expose `created_at`/`updated_at`. Existing contracts record byte-identically and gain no database objects.
- `PATCH /api/<resource>/<uuid>` updates only the sent fields; `null` clears an optional field. The merged record is
  revalidated against the full contract and written with the same conflict check as `PUT` (`docs/backend/patch.md`).
- Declarative operations (`server/operations/<name>.jsbrain`, `POST /_brain/operations/<name>`) run read, create,
  update, upsert, delete, action and transition steps in one Postgres transaction and roll all of them back on any
  failure, with step-prefixed errors (`docs/backend/transactions.md`).
- Atomic actions (`server/resources/<r>/actions.jsbrain`, `POST /api/<r>/<uuid>/actions/<name>`) lock the row and
  apply guarded `set`, `increment` and `decrement` (compare-and-set via `when`), and `POST /api/<r>/upsert` inserts
  if absent on a declared unique key; both are race-safe under concurrent requests (`docs/backend/atomic-updates.md`).
- `POST /api/<r>/batch` creates, updates (PATCH semantics), upserts or deletes up to `batch.jsbrain { max }` items
  (default 100) in one transaction; every item is validated first and errors are keyed `items.<i>.<field>`
  (`docs/backend/batch.md`).
- `transitions.jsbrain` turns one `enum` field into a state machine: records start in a declared initial value and
  change only through `POST /api/<r>/<uuid>/transitions/<name>` with `from` guards, roles, input and side effects
  (`docs/backend/state-machines.md`).
- Job queues accept `server/jobs/<name>/queue.jsbrain` (priority, attempts, fixed or exponential backoff, timeout up to
  one hour with automatic lease renewal, drain on shutdown, concurrency); `brain jobs enqueue --priority/--delay`,
  `dead`, `retry` and `stats` inspect and revive dead letters (`docs/backend/queues.md`).
- `server/cron/<name>.jsbrain` schedules a declared job (`brain jobs cron`, `--once`); each slot is enqueued once even
  with several schedulers, and missed slots outside `catchUp` are skipped (`docs/backend/cron.md`).

- `urlState(initial, { key, values })`: a query value outside `values` falls back to the first allowed value (also
  on the server and on back/forward) without writing history; `<Tabs>` selects its first tab for an unknown bound
  value; `watch(src, cb, { immediate: true })` also runs once with the initial value.
- A `server:` function that throws `{ message }` (400) or an Error subclass with a 4xx `status` reaches the calling
  form's `f.error` through RPC and through a form posted without JS; other exceptions stay private.
- Native file buckets (`server/files/<name>.jsbrain`) accept uploads of any type by default, streamed to private
  disk with a per-bucket size limit (default 25 MiB), magic-byte type detection, optional type allowlists,
  private/public visibility, owner/role/related-record download policy and `Range`/`ETag` downloads.
- Native backend `trustProxy` takes a list of trusted proxy IPs/CIDR ranges and resolves the real client from
  `X-Forwarded-For`, so rate limiting works per visitor behind nginx; `cors.origins` adds origins and `cors.maxAge`
  caches preflights.
- Native backend health is split into `GET /health/live` and `GET /health/ready`; overload (in-flight cap or a 2 s
  database pool wait) answers `503 OVERLOADED` with `Retry-After`, and shutdown fails readiness first, then drains
  open requests within a bound.
- Production refuses a remote `DATABASE_URL` without `sslmode=verify-full` (`BDB_TLS_UNVERIFIED`); `serve` and
  `jobs work` retry the boot connection with full-jitter backoff for up to 60 s.
- Native backend routes (`server/routes/<name>/route.jsbrain` + `handler.jsbrain`) declare method, path, typed
  path/query parameters, forwarded headers, raw or JSON body, and per-route body limit (≤ 1 MiB), timeout, rate
  limit and auth; JSON keys that reach prototypes are refused.
- Signed webhook routes verify HMAC-SHA256 signatures (`t=…,v1=…` with timestamp tolerance, or `sha256=…` in hex
  or base64) over the exact raw body, with constant-time comparison and secret rotation from environment variables.
- `httpCall(url, options)` is auto-imported in server code for outbound API calls: per-attempt timeout, jittered
  retries only for idempotent requests or those with an `Idempotency-Key`, `Retry-After`, a response size cap,
  refusal of private targets and errors that never include query strings or header values.
- The backend OpenAPI document (`/_brain/docs`, `brain backend inspect`) now covers app-declared routes and file
  bucket upload, download, delete and metadata operations, with a shared error schema.
- `platform.logging` selects per-request JSON lines (`errors`, `all` or `off`) and a `slow` threshold; lines carry the
  request id and route template only, never paths, queries, headers, bodies or client addresses.
- Production backend answers send `Strict-Transport-Security` (one year, subdomains included).

### Changed

- `brain test`, `brain doctor` and MCP `brain_test` run in owned headless Chrome by default and never probe or open a
  connected Claude Viewer/Codex Viewer unless the user opts in with `--viewer`, `BRAIN_VIEWER=1`, `brain e2e run` or
  MCP `viewer: true`; the scaffold's agent guide no longer requires an extension.
- Adding the first extra language moves every fixed text to keys: text, sentences with values as one message with
  placeholders, visible attributes, `<meta seo>` title and description, and string literals in markup expressions.
  Keys are scoped by view path (`account.your_account`, not `index.your_account`), folded to ASCII, unique per text
  and never duplicated in the locale files. `brain add locale en` on a site in another language writes the framework
  texts in English.
- The kit's `<ErrorState>` reads its texts from the app's dictionary (`error_state.*` keys, documented in the i18n
  guide and INTERNALS §37) like the `form.*` messages, with the English text as the default when a key is missing.

- Job enqueue takes a per-queue transaction lock and a bounded count instead of one global lock and a full scan, so
  hook-enabled writes on different queues no longer serialize. The 10000 active-job cap now applies per queue.
- Backend relations: `ref:<resource>` fields install a foreign key with `on_delete` `restrict`, `cascade` or
  `set_null`, reject missing ids with 422 `REFERENCE_NOT_FOUND`, and support one-level `?include=` (docs/backend/relations.md).
- Backend list filters declared in `query.jsbrain`: typed operators, comma lists, related-field filters and relative
  times (`now-7d`); undeclared or repeated parameters answer 422 (docs/backend/filters.md).
- Backend list sorting by declared fields with null placement and a stable id tie-break (docs/backend/sorting.md).
- Backend list `total`, named aggregates with exact decimal sums and averages, and per-row related counts
  (docs/backend/aggregates.md).
- Backend keyset pagination: `next_cursor`, accepted back as `cursor` or `after`, bound to its sort (docs/backend/pagination.md).
- Backend search combines with declared filters, totals, aggregates and cursor paging when sorted (docs/backend/search.md).
- Backend relational ownership grants `owner.<ref>[.<ref>]` for read, create, update and delete (docs/backend/ownership.md).
- Backend per-field `visible` grants project responses and close restricted fields to filters, sorts, search and
  aggregates (docs/backend/visibility.md).
- Backend field access modifiers `readonly`, `writeonly` and `private` (`'str writeonly'` or `{ access }`): rejected from
  client input or never serialized as declared, preserved on full replacement, and marked in OpenAPI.
- Backend slug history: a field `{ type: 'str!', unique: true, slug: true }` keeps earlier slugs, and
  `GET /api/<resource>/redirects/<slug>` returns the current one for a permanent redirect (docs/backend/slugs.md).
- `brain backend serve` uses separate HTTP and worker pools (`BRAIN_DB_HTTP_POOL`, `BRAIN_DB_WORKER_POOL`), checks
  them against `max_connections` at boot, and sends Postgres timeouts as startup options instead of a session `SET`;
  HTTP adds a 1 s lock wait and, on Postgres 17+, a 9 s transaction cap.

### Changed

- Backend responses no longer include the record `owner` id for anonymous or unrelated readers; owners, relational
  owners and roles named in the read policy still receive it. Declare `expose_owner: true` in `query.jsbrain` to
  publish it. The OpenAPI row schema marks `owner` optional.
- The shared lexer decodes ASCII directly, scans ASCII identifier runs as bytes and indexes punctuation by first byte
  in the same longest-first order. Source-map scanning skips slash-free trivia gaps and builds its generated line index
  only to locate a failure. Emitted JS, server JS, CSS, maps and diagnostics are byte-identical; no compiler-speed
  claim is made until paired measurement.

### Fixed

- `throw redirect(url, status)` in `load` answers the redirect under `brain run` as it does in production; it was a
  500 with `[load] …: [object Object]` in the log. A thrown non-Error object is now logged as JSON.

- A form posted without JS is validated in the page's language (`form.*` and `form.error` of its dictionary), in
  `dist/server.mjs` and in `brain run`; in development a POST to `/<locale>/page` reached the wrong view (400).
- Pages that show `<ErrorState>` ship its `error_state.*` texts in their dictionary chunk.
- Tests can pass slot content: `mount(Comp, props, { slot: 'Text', slots: { hint: 'Text' } })` renders the default
  and named slots as text, so a component whose accessible text comes only from its slot no longer fails the B0406
  audit in its own test. The test `mount` now also forwards `provide`, which it previously ignored.
- An `{#if}` block no longer throws when its guarded value turns `null`: the scheduler brings an effect's owners up to
  date first, so `{#if data.file}{data.file.name}{/if}` disposes the block before `data.file.name` would re-run, even
  when another dependency of the condition re-ordered the subscriptions.
- B0910 no longer takes a page down because of native work or a loaded machine. The elapsed-time clock stops before
  an awaited expression is evaluated, so the synchronous start of a process's first `fetch` (or four parallel reads in
  `load()`) is not charged; author functions called inside the expression still are. Because that clock is wall-clock
  time, an overrun of the 200 ms allowance is now a diagnostic: it is logged with `console.warn` and reported once
  with `measured`, `limit` and its source location, without an owner, so no error boundary replaces the view. Steps
  and depth stay fatal and still stop runaway loops and recursion.
- The opt-in Viewer lane no longer fails "at open" when Claude Viewer and Codex Viewer are both connected and
  `BRAIN_VIEWER_PROFILE` names a profile of the second one: an explicit open refusal (nothing opened) moves to the next
  connected Viewer, `BRAIN_VIEWER_SERVICE=claude-viewer|codex-viewer` pins one, and Viewer failures now carry the
  adapter code and fix (`VIEWER_OPEN_REFUSED`, `VIEWER_PROFILE_REQUIRED`) instead of a bare stage.

- `brain build` no longer reports "Production removed unused bindings" for constants and imports of a `.brain` view
  that only its `server:` functions use; such facts are marked `serverOnly` and skipped by `brain quality`. The
  warning now needs a verified app-source removal, so runtime tree-shaking and generated helpers do not raise it.
- `:global(…)` after the first compound of a component selector (`.list :global(a)`) passed `brain check`, but the
  framework does not support `:global`, so the browser dropped the rule. It is now B0301 wherever it appears.
- A `load()` exported from `views/_layout.brain` was silently never run, leaving the layout's props empty. `brain check`
  now reports it as B0117 with the fix (load in the view, or read shared state from a channel).
- Production logs recorded a compiler budget failure (B0910/B0911) thrown by a view `load`, an RPC, a form or SSR as
  `NODE_INTERNAL_ERROR` with a generic fix. The log entry now keeps its `code`, `budget`, `limit`, `file`,
  `component`, `line`, `column` and actionable `fix`; the error message is still never logged.
- Streamed SSR routes (static `<meta seo>` with `load`) answered 200 when `load` redirected, threw `notFound()` from
  an imported helper or threw a deliberate 4xx, because the head went out before `load` settled. The head now waits
  up to 200 ms for `load`; an outcome in that time gets the same 303/404/4xx/500 as a non-streamed route. Slower
  loads keep streaming and handle the outcome inline.

- RPC and form bodies were capped at 64 KB with no setting, so an app sending a small file (for example base64 in an
  RPC argument) failed with 413. The default is now 1 MB, configurable up to 64 MB with `server.maxBodyKB`; the 413
  text names that setting and the production server logs it with the limit. Bodies are still read bounded. The dev
  server answers `Expect: 100-continue` (100 within the limit, 413 past it) instead of 417, and its sidecar now honors
  the configured deadline and rate and carries bodies past its former 16 MB request frame.

- The development compile cache keys each source by modification time, size and (on Unix) inode. A file replaced
  atomically while keeping its timestamp is recompiled instead of serving the previous output until a watch event.
- Deeply nested BrainScript, markup or CSS reports B0002/B0003 instead of overflowing the compiler stack; one small
  file could previously abort `check`, `build` and the MCP server. Span columns are computed in logarithmic time, so a
  single very long line no longer makes diagnostics quadratic.
- Live sockets that are accepted and then closed with "try again later" no longer reconnect and refetch forever; the
  retry budget resets only after a healthy connection. Long-lived reconnects keep working.
- Runtime resources are released: settled handler requests leave the component registry, synchronized stores free
  their channels, listeners and sockets on close and subtree unmount, and cached Prompt API sessions are bounded to
  the eight most recent.
- The development server unregisters closed HMR sockets immediately. `brain start` escalates workers that ignore
  SIGTERM to SIGKILL after reloads, memory restarts and health restarts, so an old worker cannot keep the listener.
- JSON-LD `Article`, `BlogPosting` and `NewsArticle` without `headline` are no longer reported as a warning: Google
  requires no Article property, so `headline` is now recommended (info, no score penalty) like `image`, `datePublished`
  and `author`.
- `brain run` serves `/llms.txt` (and `/<locale>/llms.txt`) like the build, and its dev `/sitemap.xml` now carries
  hreflang alternates with `x-default` for every locale and image entries for a view's literal `<img src>`, generated
  by the same code as `brain build`; a split sitemap answers `/sitemap-N.xml`.

### Security

- `httpCall` now checks every address a hostname resolves to, at connect time, and refuses private, loopback,
  link-local (cloud metadata), CGNAT, benchmark, multicast and reserved IPv4 and IPv6 targets, including IPv4-mapped,
  IPv4-compatible, NAT64 and 6to4 forms of them; before, a public name that resolved or rebound to such an address was
  contacted. Connections are not pooled, so a socket opened for an allowed private host is never reused. New options:
  `allowPrivate: ['billing.internal']` allows only the named hosts, `redirects: n` (default 0, redirects stay
  unfollowed) follows up to n hops and checks each target again, refuses https→http downgrades and drops credential
  headers on cross-origin hops, and `lookup` swaps the resolver whose answers are still checked. The development
  opt-out `BRAIN_OUTBOUND_ALLOW_PRIVATE=1` is ignored when `NODE_ENV` is `production`.

### Tests

- Pin that a native socket rejected at the handshake stops retrying within its bounded budget.

## [0.5.0] - 2026-10-30

### Added

- Opt-in `brain add resource ... --admin-ui` prepares project-owned, role-guarded list/detail/create/edit views,
  progressive forms and missing Design System controls in one revision-checked change plan. Existing files are
  never overwritten. The fixed reference edit fixture has no headroom at 30,720 B; arbitrary layouts are not
  guaranteed to fit that budget.
- Experimental native invalidation sockets with explicit exact-Origin configuration, persisted-session/JWT
  authentication, row/role revalidation, bounded rooms and graceful shutdown. Events carry invalidation metadata,
  not records or a durable replay log; this is a single-backend-process capability.
- Development `liveResource(resourceHandle, options)` attaches owner-scoped, coalesced HTTP refresh to those
  metadata events. Public principal changes release the previous connection; unmount releases owned timers and
  handles. Six actual compiled native/browser cases passed on the release binary.
- One `brain.json` declaration feeds native HTTP/socket origins, private build metadata and fixed development,
  static-production and Node gateways. Explicit false overrides disable Node activation. Disabled projects do not
  acquire a socket listener or live runtime asset.
- nginx/PM2 templates route opt-in sockets through the appropriate fixed Rust or Node authority. Exact route,
  query/method/protocol checks, credential projection and disabled-topology compatibility have component coverage.

### Fixed

- Development thread-admission failures return actionable startup errors or best-effort503 responses instead of invoking the CLI panic hook. Admission permits recover and repeated diagnostics are coalesced; the64-connection cap and production worker behavior remain unchanged.
- Development imports of invalid value modules now reject with their original compiler diagnostics instead of returning a component-shaped fallback. Component fallbacks, durable agent feedback and recovery remain intact; static named-import linking retains its explicit limitation.
- Accept the existing GET `cache: 'no'` runtime contract in the checker, including when app-wide data caching is disabled. Keep that explicit endpoint privacy intent instead of suggesting its removal.
- Source-map scanning requests validated token boundaries without copying discarded identifier and literal values.
  The parser keeps its normal token payloads; marker ownership, source maps and generated-code diagnostics remain intact.
- Build DOM access-path strings by appending instead of repeatedly copying their growing prefixes. Generated traversal expressions remain byte-identical; this is not a claim that browser sibling traversal is already linearized.
- Doctor retains fresh build measurements when the payload budget fails, runs SEO against those current pages, and keeps its overall build gate failed. Source failures never reuse stale SEO output; absent test files no longer appear as a measured zero-leak result.
- Future release publication requires the same-commit source verification job to pass, including when reusing reviewed package artifacts. Canceled runs and manual pack-only dispatch cannot publish.
- Release change-metadata, preview-admission and legacy-writer locks explicitly when their leases end,
  including when a forked child temporarily retains the same descriptor. Contention still fails immediately;
  interrupted lock syscalls retry at most three times and other errors preserve their safe error category.
- Release the project-manifest writer lock explicitly when its lease ends. A child forked concurrently by the same
  process (for example an MCP session or a parallel test) no longer makes the next metadata refresh report busy.
- Preserve author strings, template text, regular-expression literals and generated HTML containing source-marker
  lookalikes such as `/*@L71*/`. Source-map stripping recognizes only exact compiler marker comments in the
  framework lexer's trivia; literal tokens remain data, including inlined components and public environment
  constants. Author JavaScript comments are discarded by the parser before emission and cannot redirect maps.
  Parenthesize regex-leading expression statements so compiler-added blocks preserve lexical context. Generated
  syntax that cannot be scanned now stops compilation with an explicit generated-location diagnostic.

### Internal

- Add an opt-in Node Upgrade bridge for native invalidation sockets, including private production/helper
  packaging and development sidecar copies. It delegates protocol authentication and authorization to Rust,
  preserves trusted Origin and native credentials, and bounds connections, buffering and lifetime. Public
  project configuration, browser subscriptions and Rust gateways are integrated and covered by the combined source
  gates and the native/browser acceptance run. The private manifest boolean and explicit overrides are
  strictly typed; helper presence does not activate transport.

### Upgrading

Socket configuration is disabled by default. To opt in, declare canonical `backend.origins` and its explicit
`sockets.allowedOrigins` subset, select the public site URL, and configure a fixed private backend authority.
Development/test servers fail if their actual Origin is not declared; an automatically chosen port is never
silently authorized. Access-token renewal before socket reconnection is not automatic. Administrative views are
an explicit generator flag and preserve existing project sources.

## [0.4.0] - 2026-10-30

### Added

The following 34 points are the detailed feature inventory for this release. Deferred work is labeled explicitly.

1. **Project-owned Design System foundation.** New projects ship editable design tokens and agent instructions for consistent colors, typography, spacing, radii and component organization. Blueprint, seed, configuration and adoption tools extend the project's own CSS system without installing a CSS framework.

2. **Editable basic components.** Project-owned controls expose explicit prop contracts and loading, busy, disabled and validation states. Numeric Input defaults use step="any" only for number controls; existing controls are preserved.

3. **Component gallery and manifests.** A searchable static gallery and per-component manifests expose component contracts, sources and design references. Agents can discover reusable UI without reading every implementation file.

4. **Mobile-first design guidance.** Foundations and generated guidance address responsive layouts, safe areas, touch controls and mobile input behavior. Exact, unambiguous design-token reuse suggestions remain author-reviewed warnings rather than automatic cascade rewrites.

5. **Guarded upgrades and design adoption.** Upgrade previews inventory existing files, preserve unknown configuration/design fields and prepare revision-checked package-pin edits. Design adoption is separate; installation and behavior migrations remain explicit decisions.

6. **Scoped CSS deduplication.** Component styles retain encapsulation while equivalent declarations can share compiler-owned output under compatible selector/context/order rules. Small semantic differences and public contracts keep distinct output.

7. **Production cleanup with agent feedback.** Proven unused bindings, imports and declarations are removed during production compilation, with structured source-cleanup feedback. Observable effects, throwing reads, live bindings and module evaluation are preserved; this is not a claim of complete ESLint/Stylelint coverage.

8. **Deterministic compilation and bounded caches.** Runtime pruning converges in stable bounded passes rather than hash-map iteration order. Content/configuration/compiler-aware caches reject unsafe or incomplete entries, and source reads have explicit file, byte and time bounds.

9. **Zero executable JavaScript for eligible static pages.** Successful static prerenders can emit HTML/CSS without hydration or router startup, including in mixed sites. Interactive, guarded, request-dependent and server-rendered routes retain the execution they require.

10. **Final-asset byte accounting.** Build ledgers attribute emitted raw/gzip/Brotli JS and CSS assets and distinguish initial, deferred and conservative route references, including .mjs modules. Asset accounting is separate from observed network transfer, browser execution time and per-rule compressed-byte provenance.

11. **Predictive navigation workers.** Page-local next-route/asset plans support bounded speculative fetching from link visibility and user intent. Time/byte/concurrency limits, session-write invalidation and cancellation keep speculation within its declared lifecycle.

12. **Immutable assets and precompression.** Content-hashed assets retain immutable caching. Build-time Brotli quality 11 and gzip results are cached by content/settings; native Rust and Node delivery negotiate encoding weights and explicit exclusions without applying static-asset caching to private HTML or APIs.

13. **Fragment hot reload.** Proven-compatible template text and attribute edits preserve live nodes, focus and state. Changed wiring and unproven fragment updates use an explicit fallback; development proof metadata is excluded from production output.

14. **Native types and browser-target diagnostics.** Conservative inference connects declarations, signals, schemas, API bodies and form bindings with mechanical warnings B2001-B2005. Pinned Baseline evidence adds B2101-B2103 for app code; unsupported browser globals remain visible warnings, and unknown flows are not presented as proven types.

15. **Experimental Rust/Postgres backend.** Literal resource schemas and policies drive validation, CRUD, search and OpenAPI. PostgreSQL migrations are explicit and checked; the HTTP/database stack uses maintained Rust infrastructure while the compiler keeps its own parser, bundler and minifier.

16. **Persistent native sessions and JWT.** Argon2id authentication, revocable persisted sessions and optional access/rotating-refresh JWT sessions provide native identity. The backend chooses the configured mode; failed JWT authentication never falls back to another provider.

17. **One native sign-in authority for SSR, RPC and forms.** ctx.backend.login relays validated native HttpOnly cookies and returns only the canonical sanitized user from a same-origin POST server function. New sign-in pages share this flow with JavaScript and progressive no-JavaScript submission; deployed runtimes cannot activate the development demo implicitly.

18. **Google sign-in, recovery, verification and roles.** Google nonce/JWKS verification, password recovery, email verification and configured roles have project-owned recipes. Optional Resend delivery requires private operator configuration, and first-admin promotion is an explicit command rather than a signup privilege.

19. **Request-scoped server services and repositories.** ctx.backend.resource exposes list/get/create/update/remove through the same native schemas, policies and cookie identity used by the API. Operations share bounded transport/cancellation; writes require same-origin POST and are not retried automatically.

20. **Native resource support generation.** brain add resource creates schema/policy/hooks/search contracts, typed collection/detail clients, server services/repositories and a discoverable schema-revision snapshot. It preserves routes, guards, controls and budgets; administrative UI is marked planned and its unfinished renderer is excluded from the public generator.

21. **Enforced hooks and persistent queues.** Configured before-write hooks run through bounded private Node executors on Unix, revalidate returned data and reject same-record conflicts. After-write events commit with the mutation in a PostgreSQL outbox; leased workers, retries and terminal states provide at-least-once delivery, so handlers must be idempotent.

22. **Signed webhooks.** Inbound HMAC verification checks event identity, timestamp and replay state. Durable encrypted payloads, signed outbound deliveries, bounded retries and validated HTTPS destinations keep provider integration explicit; configuring or building a project does not send external messages.

23. **Search, slugs, short links and HTTP helpers.** Explicit public search fields use PostgreSQL full-text/trigram indexes. Native utilities also cover slugs, authenticated short-link creation, validated redirects, status/response helpers and safe default error pages with real HTTP statuses.

24. **Private image library and optional cloud storage.** Content-validated originals, bounded renditions/crops, versioned ownership metadata and leased cleanup support native media. Optional Google Cloud Storage uses an official-SDK adapter and explicit credentials/configuration; it does not create buckets or upload during scaffolding.

25. **Concurrent agent changes.** Per-file leases, SHA-256 revisions, checked plans, individual atomic replacement and conditional recovery protect cooperating agent edits. Multi-file changes are recoverable logical transactions, not an instantaneous filesystem transaction; external-editor races remain explicit.

26. **Native CLI and MCP discovery.** New projects include Claude Code/Cursor MCP registrations, CLAUDE.md importing AGENTS.md, optional edit-time checks and compact tool profiles. Component search, contracts, graph/context/impact/why tools provide structured navigation; registration is distinguished from a verified live client connection.

27. **Actionable error feedback.** Structured diagnostics preserve known source/generated locations, bounded frames, causes, mapping precision, truncation and request references. Durable compiler feedback reaches local agents before browser overlay publication; production public messages exclude raw traces, internal paths and arbitrary exception messages.

28. **Quality panel and project history.** Private JSON/Markdown/panel output exposes code-quality observations, proposed fixes and acceptance evidence. Project-local tasks, history and lesson proposals distinguish reported claims from verified results; they do not imply model retraining or automatic external issue publication.

29. **Native form validation and runtime safeguards.** Required/blank/type checks, field errors and duplicate-submit locking retain client/server consistency. Proven flat literal forms can use smaller capability-specific runtime paths, while dynamic/async validation remains available; owned reactive/worker budgets do not claim to predict every leak or stop all foreign code.

30. **Shareable public navigation state.** Selections and native dialogs can synchronize declared public state with URL parameters and restore direct links/back/forward while preserving unrelated parameters. Credentials, private form values and transient state are excluded from this navigation contract.

31. **Browser data and worker capabilities.** IndexedDB records, small public preferences/cookies and an optional official SQLite Wasm/OPFS adapter support bounded local storage. Dedicated/shared worker RPC and explicit worklet, transfer, shared-memory and canvas capability checks preserve browser prerequisites and fallbacks.

32. **Official integrations and selective dependencies.** Discoverable OpenAI/Claude/Gemini and Google-service adapters favor official SDK compatibility and selective installation/upgrades. Installed ESM imports and explicit integrity-pinned CDN acquisition use the compiler graph with side-effect preservation; native date/hash helpers and library tools are also discoverable.

33. **PM2 and nginx deployment configuration.** Reviewable configurations cover static/Node SSR frontends, optional Rust backends and supervised queue workers with fixed loopback upstreams and graceful lifecycle settings. Four topology configurations passed actual PM2/nginx parsers; syntax validation is not a claim that a public host was activated.

34. **Agent-oriented end-to-end and benchmark tooling.** Detected connected Codex Viewer/Claude Viewer adapters support scenario execution, explicit profile selection and owned-tab cleanup; headless accessibility/optional heap and engine-matrix checks are complementary evidence. Provider-neutral LLM benchmarks record explicit models, frozen spec/binary hashes and real first-pass outcomes, rejecting compiler crashes or malformed diagnostics.

### Changed

- Production public error output uses safe references; raw traces remain in private diagnostic channels.
- Runtime export pruning now converges in stable bounded passes instead of depending on hash-map iteration order;
  removed functions no longer permanently retain another runtime module's unused parser or core exports.
- Server-rendered view styles use immutable compressed stylesheet assets, separate from personalized HTML.
  Proven flat literal form rules use tracked specialized validators; dynamic/bounded/nested schemas retain validation.
- JavaScript ledgers include emitted `.mjs` SDK modules and their static dependencies.
- Safe HMR template text/attribute edits preserve live nodes, focus and state; incompatible wiring has an explicit fallback.
- npm packages foreground the Rust-powered full-stack compiler and carry precise LLM, frontend/backend, CLI,
  Design System and testing discovery keywords, without claiming unrelated IoT, robotics or mathematics capabilities.

### Fixed

- Server module file URLs use platform-native path conversion: Windows verbatim drive/UNC paths and reserved filename characters no longer corrupt module specifiers; modification-time cache busting is retained.
- Native release smoke failures report their phase, allowed diagnostic/Node codes and known exit status before cleaning private scratch output.
- Resources with no configured search fields no longer fail their first write: additive migration 7 handles PostgreSQL's null trigger-argument array without changing prior migration checksums.
- Development HMR proof ranges account for prepended guard imports; location bookkeeping cannot mask the wrong generated string.
- Feedback writers explicitly release their own file lock even when a child retains a duplicate descriptor. Locale generation keeps its existing writer guard while applying translation fixes.
- Runtime npm tarballs retain their actual JavaScript modules alongside notices. Native packages carry verified Cargo/std/toolchain notice inventories, and publication includes native and anonymous-registry smoke checks.
- Decoded static routes cannot expose private server modules, environment files or deployment configuration; private Node traces discard arbitrary exception messages while preserving source frames.
- Native account adapters honor hosting prefixes, reject foreign application bases and never inherit external API origins.
- LLM benchmarks reject compiler crashes/invalid diagnostic output and use valid scaffold names for dotted model identifiers.

### Documentation

- The site hero shows the `npm create` command with a copy button (cbb0917).

### Upgrading

- New type/browser diagnostics are warnings in this minor development cycle. Inspect `brain preflight`; do not
  disable security diagnostics. `brain upgrade <exact-version>` prepares pin edits only; installation and semantic
  migrations remain explicit. Atomic plans require leases and exact revisions, including null for new files.
- Existing app: run `brain agent setup` to merge local MCP registrations; add `--hooks` only to opt into Claude Code
  edit feedback. Restart the client and verify its tools/hooks. Source installations must use the installed brain
  command in their MCP config; npm installations use the pinned local binary.
- New diagnostic B0734 is a doctor warning about missing local MCP configuration, not a failed connection test.
  B0735 rejects invalid opt-in resource/job schemas, policies or hook references. Existing B04/B05 severities do not
  change, and ordinary server folders without backend metadata keep their previous meaning.
- Connect the experimental backend through a fixed loopback BRAIN_BACKEND_URL for native proxy/SSR auth; migrate
  only the selected Postgres environment explicitly. Validate each existing production API slice before replacement. Account email needs explicitly configured delivery; admin provisioning is explicit. Native authenticated socket rooms and production Upgrade tunnels are deferred beyond this checkpoint.
- A fresh frozen LLM generation gate and production topology/latency benchmarks remain acceptance work for this slice.

### Measured migration and known limits

The real npm 0.3.1 compiler generated the migration fixture; its archive integrity was verified before execution.
The same source/data inputs retained `budget.viewKB: 30`. These are verified pre-release build-ledger snapshots,
not measured network traffic or a speed comparison:

| Fixture / compiler snapshot | Conservative `/login` bytes | Original 30 KiB budget |
|---|---:|---|
| Released 0.3.1 starter, compiler 0.3.1 | 26,984 | Pass |
| Same starter, initial 0.4.0 checkout | 32,782 | Fail |
| Same starter, optimized 0.4.0 checkout | 29,836 | Pass |
| New 0.4.0 starter, optimized checkout | 30,223 | Pass |

Totals include gzip JS/CSS plus raw personalized SSR HTML, including conservative deferred-code references.
The real adoption/CAS regression preserved 40 pre-existing non-package files, unknown configuration/design fields,
token/font bytes and opaque local data. It changed the package declaration without installing the destination package.
Two separate cold-cache compiler processes produced identical complete build files and signatures for the new starter.
These checkout snapshots are not release-binary performance benchmarks.

- Native authenticated sockets/rooms and their Node/Rust/nginx Upgrade integration are not part of this checkpoint.
- Source mapping is line-level where proven; source columns remain null when unavailable. Precise arbitrary minified-byte
  provenance and `brain explain-output <offset>` are not implemented. `brain explain <code>` explains diagnostics.
- General cross-module constant propagation, strength reduction, common-subexpression elimination and loop-invariant
  hoisting are not established capabilities. Existing cleanup/mangling is not proof of these mathematical passes.
- Asset ledgers measure emitted payloads and distinguish initial/deferred references; they do not prove browser timings,
  attribute every compressed byte to a CSS rule, or establish a universal advantage over another framework.

## [0.3.1] - 2026-10-29

### Changed

- Releases: every `v*` tag publishes the 11 npm packages through npm Trusted Publishing (OIDC, no stored token, no
  approval step); `docs/NPM-RELEASE.md` describes the new flow (57264a4).

### Documentation

- README, npm package descriptions and the site lead with "built in Rust" and the ~34 ms build benchmark; the npm
  README links the cover and the badges to the website.

## [0.3.0] - 2026-10-29

### Added

- `brain guide [topic] [--json]` and the MCP tool `brain_guide`: task playbooks for agents embedded in the binary
  (`start`, `views`, `data`, `forms`, `server`, `seo`, `a11y`, `webmcp`, `tests`, `i18n`, `performance`), each with
  steps, verify commands, common mistakes with the diagnostic that catches them and a minimal correct example. The
  `seo` guide starts from the target query and covers answer-first pages, FAQ, authors, freshness and the AI crawler
  policy. Tests reject a guide that cites an unknown diagnostic code or command. `brain new`'s AGENTS.md, SPEC.md
  and `brain --help` point to it.
- WebMCP runtime `tool(name, description, params, run, { title, readOnly, consequential, untrusted })` passes
  `title` and `annotations: { readOnlyHint, consequentialHint, untrustedContentHint }` to the registration (WebMCP
  draft 2026-10-27).
- B0411 (error): an `<a href>` with no accessible name (no text, `aria-label` or image with alt).
- B0412 (warning): link text that is only a generic phrase such as "click here" or "read more".
- B0413 (warning): an `alt` that is a file name or a lone generic word such as "image" or "logo".
- B0414 (error): an `<iframe>` without `title` or `aria-label`.
- B0415 (warning): a `<video>` with controls or sound that has no captions or subtitles track.
- `<meta seo keyword="…">` is part of the authoring contract: an indexable view without it is warning B0502, and
  B1816 now also asks for the keyword in the description (the home page `/` is not asked for it in its URL).
  `brain new`, `brain add view|collection`, the examples and the generated AGENTS.md carry a keyword.
- B1836 (info): a page without `keyword` whose title or description does not name its H1 topic ("Home" over an
  "Offline task planner" page). The `brain new` sample posts name their topic in the description.
- Every page gets one JSON-LD `@graph` tying `WebSite`, `Organization`, `WebPage` (name, description, language,
  `dateModified`, primary image, breadcrumb), `BreadcrumbList`, FAQ and `BlogPosting` together by `@id`; before,
  `WebSite` and `Organization` were on the home page only and nothing said which page a node belonged to. A type
  the author declares still replaces its automatic node.
- A page URL requested with `Accept: text/markdown` (preferred over `text/html`) answers its Markdown twin in
  `brain start` and the static server, with `Vary: Accept`; browsers still get HTML. `seo.markdown=false` turns it off
  together with the `.md` copies.

- Every built page names what built it: `<meta name="generator">` is followed by a comment with the BRAINLLM home page
  and repository, llms.txt and llms-full.txt end with a `## Built with` section, and `brain start` answers with
  `X-Powered-By: BRAINLLM (https://chrisifm.github.io/BRAINLLM/)`.
- `rss.xml`, a site-wide feed like WordPress `/feed`, plus `<section>/rss.xml` for every section without a content
  collection, and `sections.json`, one manifest of every section with its title, description, home page, feed and pages
  (URL, Markdown copy, lastmod). Every page links both. On by default; `seo.feed` and `seo.sections` turn them off.
- Predictive prefetch in the client router: internal links that scroll into view are fetched while the browser is
  idle, hover, focus or touch fetch a link first, and the next navigation swaps from memory with the page's chunks
  already preloaded. Any write (a form submit, an `api()` write, a server function call) forgets the fetched pages;
  files, `/api/`, `/auth/`, log-out, unsubscribe and confirm links and URLs with a token are never fetched, and the
  server's `load` sees `ctx.prefetch`. `data-prefetch="false"`, `<meta name="brain-prefetch" content="off">` and
  Save-Data turn it off. Pages no longer carry Speculation Rules: the router intercepts every click, so the browser's
  prerender was never used and only doubled the request.
- `ai`, a primitive for Chrome's built-in AI (Gemini Nano): `ai.ready()`, `ai.prompt`, `ai.stream`, `ai.session`,
  `ai.summarize`, `ai.translate`, `ai.detect`, `ai.write`, `ai.rewrite` and `ai.proofread`, feature-detected and
  `'unavailable'` in other browsers and on the server. `brain guide ai` covers when and how to use it.

### Fixed

- `brain seo audit` reads each view's `keyword`, so it reports the same B1816 findings as `brain build`.
- An app whose name is a runtime helper (`brain new t`) builds again: a word `t` in server text or `logout(` no
  longer imports brain-i18n.js on the server.
- `<meta seo schema="Article">` on an SSR page becomes one JSON-LD node built from that request's title,
  description, canonical, `og:image` and `lastmod`; `brain start` used to write a `<meta name="schema">` tag and keep
  the build's stale node. The build's node now carries `headline` for article types, `image` and `dateModified`, and a
  leftover `<meta name="schema">` (a view's own `<head>` block) is turned into JSON-LD by the SEO step.
- B1831 (questions without FAQ) no longer sells FAQPage as a Google rich result: Google stopped showing FAQ rich
  results on 2026-05-07; the markup stays because answer engines and Bing read it.
- `brain check` on a view outside any app no longer walks up to `/` looking for `_guard.jsbrain`; only the view's
  own folder is searched.

### Documentation

- The served spec (`brain spec`) points to `brain guide forms|server|seo` for form messages, sessions and live
  sitemaps instead of spelling them out, and says that a module the app does not have yet is declared in the file or
  created with `brain add`: Haiku 5.5 had dropped to 17/20 first-try passes on the 0.2.0 spec (bench/llm/README.md).
- Site and README: the title, hero, metadata and footer say up front that BRAINLLM is a compiler written in Rust.

## [0.2.0] - 2026-10-29

### Added

- `export async function sitemap({ env })` in a dynamic view without `paths()`: the server adds those live pages to
  `/sitemap.xml` (cached 5 minutes, capped at 50 000 URLs, `/sitemap-dynamic.xml` when the build wrote an index).
  `sitemap()` outside a dynamic view is B0117 (#8).
- `notFound()` from `load` (thrown or returned) answers 404 with `views/404.brain` in `brain start`, `brain run` and
  prerender; routes whose `load` can return it are not streamed (#7).
- B0133: replacing a form's `f.v` is rejected, because the inputs keep reading the original field signals (#6).
- Validation messages are localized through the app dictionary (`"form.required"` and friends in `i18n/<locale>.json`), falling back to English (#6).
- `brain schema` lists `seo.ai`, `seo.og`, `seo.search` and `seo.news` (#9).

### Fixed

- `<select value>` and `<option value>` keep their value in SSR, prerender and the browser (#7).
- `canonical` and `og:url` on dynamic pages use the request path instead of the route pattern; a view's head tags
  replace the layout's tags with the same name, property or canonical (#7).
- A client-side `_guard.jsbrain` that reads `ctx.user` falls back to a full page load so the server decides (#7).
- Logout no longer stalls in Chrome: session clearing sends `Clear-Site-Data: "storage"` only, and session-bearing
  SSR HTML is `private, no-store` (#7).
- llms.txt section headings use the title of the section's own page (#8).
- The `*` route and special `!` routes never appear in sitemap.xml or llms.txt (#8).
- The starter login button uses `busy={f.busy}`, so it disables while signing in (#9).

### Documentation

- SPEC: `ctx.session` and `ctx.env`, signed session cookies, server function error handling, `load` only in views,
  and that `brain run` does not render view markup on the server (#9).
- Site: a real-world testing section with the fixes above.

## [0.1.0] - 2026-10-28

### Added

- First public npm release for seven native platforms (macOS arm64/x64, Linux x64/arm64/musl, Windows x64/arm64),
  the runtime and the `npm create @christianirack/brainllm` and `npm create @christianirack/brainll` initializers (#5).

[Unreleased]: https://github.com/chrisifm/BRAINLLM/compare/v0.4.0...HEAD
[0.4.0]: https://github.com/chrisifm/BRAINLLM/compare/v0.3.1...v0.4.0
[0.3.1]: https://github.com/chrisifm/BRAINLLM/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/chrisifm/BRAINLLM/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/chrisifm/BRAINLLM/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/chrisifm/BRAINLLM/releases/tag/v0.1.0

<!-- Maintainer acceptance is tracked locally; this section describes implemented source, not a release verdict. -->
