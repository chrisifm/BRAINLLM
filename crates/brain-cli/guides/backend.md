# guide: backend — resources, policies, accounts and jobs on the native Rust/Postgres backend

## When
The app stores records, has accounts or runs background work. A page that only reads a third-party API needs `load` or
a server function instead (`brain guide data`). The backend is experimental; it runs as its own process.

## Steps
1. Fastest start for a new app: `brain new <app> --backend`. In an existing app: `brain add auth`. Both write the account
   views, the auth configuration and the environment keys; `brain add auth` never overwrites a file.
2. Add a resource: `brain add resource posts --fields "title:str! body:str status:enum(draft|published)"`.
   `brain backend capabilities` lists every field type, filter and policy grant with examples.
3. Review `server/resources/posts/policy.jsbrain` before serving. A new resource starts admin-only, so a signed-in account
   gets `403 POLICY_DENIED`; `--policy owner` (each account its own rows) or `--policy public-read` starts open.
   Grants: `public`, `owner`, `owner.<ref>`, a role name.
   A role grant admits only that role. New accounts get role `user`; `brain add roles` writes your own names.
4. Set `DATABASE_URL` and a fixed loopback `BRAIN_BACKEND_URL` in `.brain_env`.
5. `brain backend migrate` applies the generated migrations; it never runs on its own. `--create-db` creates a
   local database that does not exist yet; a remote one is never created.
6. Run both processes: `brain run --backend` in development. In production run `brain backend serve --env production` and
   `brain start` separately, usually with `--with-workers` for the job queues.
7. Read data from pages through the generated client under `api/`, or from `load` and server functions with
   `ctx.backend.resource('posts').list({ filter, sort, limit })`. Writes need a same-origin POST server function.
8. Inspect instead of guessing: `brain card resource:posts` prints the request and response shapes, and
   `brain backend schema --json` every resolved contract. An error response has a `rid`: `brain backend logs <rid>` gives
   its cause.
9. Grow the contract with the file that names the feature: `query.jsbrain` (filters, sort, include, search, aggregates),
   `actions.jsbrain` (guarded atomic writes), `transitions.jsbrain` (state machines), `batch.jsbrain`.
   A field is optional unless it ends in `!`, `enum(draft|published)` included: a record without it stores no value.
10. The first administrator is explicit: `brain backend bootstrap-admin --email <address>`. Signup never grants it.

## Reference
Each feature has a page served by the binary: `brain guide docs/backend/filters.md` and so on; `brain guide docs` lists
them. Start with `docs/backend/ownership.md`, `docs/backend/visibility.md` and `docs/backend/server-queries.md`.

## Verify
For captcha, keep public test keys in development/testing; production needs the real provider-issued secret.
Check/doctor report B0790 when production selects a public fixture. Dummy hostname responses belong to the provider;
omit hostname pinning for test keys instead of assuming the sample hostname. See `docs/backend/captcha.md`.

```bash
brain check --json                 # [] (contract errors are B0735)
brain backend migrate              # applies pending migrations
brain card resource:posts          # the shapes your client should expect
brain doctor                       # exit 0
```

## Common mistakes
| Mistake | Code | Fix |
|---|---|---|
| invalid field, policy or hook in a contract | B0735 | the message names the file and key; `brain backend capabilities` lists what is valid |
| `read: ['public']` on a resource with a state field | B0749 | a row condition `{ public: { where: { status: 'published' } } }`, or `expose_all_rows: true` |
| a bucket without `types` | B0770 | `types: ['application/pdf']`, or `types: '*'` when any file is meant |
| a backend command on a remote database with no `--env` | B0785 | pass `--env production` (or the real environment) |
| a variable read in code but absent from `.brain_env` | B0721 | declare it; `KEY?=` when the app works without it |
| a policy that grants a role nobody can get | — | signup gives role `user`; grant `user`, or assign the role explicitly |
| expecting `owner` on a public list | — | `owner` is hidden from other readers unless `expose_owner: true` |
| writing from `load` | — | a same-origin POST server function; reads only in `load` |

## Minimal correct example
```bash
brain new notes --backend && cd notes
brain add resource notes --fields "title:str! body:str" --policy owner
brain backend migrate --create-db
brain run --backend
```
