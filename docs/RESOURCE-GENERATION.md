# Native resource generation

`brain add resource posts --fields 'title:str! body:str' --json` creates native
backend contracts and executable client/server helpers offline. It does not run
migrations, create accounts, start services or generate administrative views.

| Files | Responsibility |
|---|---|
| `server/resources/posts/schema.jsbrain`, policy, hooks and search | Canonical native contract, explicit policy and transactional hooks |
| `server/resources/posts/repository.jsbrain` | Request-scoped `ctx.backend.resource('posts')`; no second database client |
| `server/services/posts.jsbrain` | Administrator check, canonical schema validation and native list/get/save/remove orchestration |
| `api/posts.jsbrain`, `api/posts-record.jsbrain` | Collection GET/POST and record GET/PUT/DELETE clients |
| `server/resources/posts/client-contract.json` | Existing helper paths, canonical schema revision, field snapshot and `adminUi.status: "planned"` |

The resource card (`brain card resource:posts`, also exposed through backend
discovery) includes `generatedContract`. The JSON snapshot is metadata and does
not replace the canonical schema or policy. It does not point to nonexistent
views or guards. Review consumer code deliberately after changing a contract.
Native migration drift checks continue to protect existing stored data.

## Identity and request lifecycle

Configure fixed loopback `BRAIN_BACKEND_URL` for the production or development
server. Sign in with an existing administrator; signup never grants this role.
First-admin promotion remains the explicit operator command
`brain backend bootstrap-admin --user <existing-user-uuid>`.

Server functions pass their authoritative `ctx` to the service and repository.
Native policy remains the final authority. Repository writes require a
same-origin POST server function and retain bounded request/response sizes,
deadlines and concurrency. No mutation retries are added. Native responses
carry no-store for private data; static assets keep their immutable caching.

All generated files are create-only. A conflicting client or backend path aborts
before writing. Existing routes, guards and project controls remain untouched.
The shared atomic creator publishes new files while the generator owns the
project writer lock. The command does not change `brain.json` or its budgets.
Build application views deliberately with the project design system.

## Administrative UI remains planned

An experimental renderer was verified against an owned loopback backend with
compiled RPC, SSR, natural boolean/numeric controls, safe JSON field errors and
No-JavaScript form POST, including confirmation before deletion. It is retained
only in isolated tests and a private continuation proposal, not exposed as a
released CLI/MCP generation API.

The normal two-string form still exceeded the unchanged 30 KiB route budget:
create 31449 bytes (30.712 KiB), edit 32338 bytes (31.580 KiB). A mixed four-field
form measured create 32268 bytes and edit 32492 bytes. These measurements include
all deferred code and private HTML counted raw; they do not represent measured
network transfers. The experimental functional fixture uses an explicit 48 KiB
scratch envelope, independently of the release generator's default-30 build
regression. No production budget was raised to advertise this UI as complete.

The release generator is checked by building the default scaffold after adding
both the documented two-string resource and a mixed boolean/number/object
resource. Complete framework, browser and fresh LLM release gates still apply.
