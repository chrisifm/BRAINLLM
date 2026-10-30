# Mandatory native before-hooks

Declare validation/transformation in a resource's existing literal hooks contract:

```js
export default {
  before_create: 'validate_order',
  before_update: 'validate_order',
  before_delete: 'validate_order',
  after_create: 'order_created'
}
```

Create the private server module with `brain add hook validate_order`. Generated placeholders throw until their validation is implemented; they never silently authorize a write. The CLI and `brain_add` use the same source generator and existing writer/lease guard. Source lives at `server/hooks/validate_order.jsbrain`:

```js
<meta server>
export default async function validateOrder(ctx) {
  if (!ctx.payload.title) throw { errors: { title: 'Required' } }
  return ctx.payload
}
```

The hook receives `{resource,operation,payload,existing,user,rid}`. `user` comes from native authentication; `existing` is the server's real `{id,owner,data}` snapshot for update/delete and null for create. Client JSON cannot supply authoritative user or existing state. Do not put tokens, passwords or arbitrary private values in error messages.

Every native HTTP create/update/delete checks policy before the callback. Raw objects are bounded before application execution. The callback's returned object is then validated against the canonical resource schema. Unknown fields and invalid/missing values cannot enter storage through a transformation. A hook may reject with `{errors:{field:'message'}}`; the HTTP boundary returns only known schema field names with static messages, discarding application error values.

Delete receives the existing record's data as its payload. Returning a valid object approves deletion; its transformed values are checked but do not rewrite the record. Throw to reject deletion. There is no delete request body.

When a before-hook is declared, serving fails if its executor/module is missing, unimportable or lacks a callable default export. Compiled private graphs remain owned by that backend process. Hooks run for direct native CRUD requests and requests originating from `ctx.backend`, including static-front/Rust-backend and Node/PM2/nginx deployments. A frontend service cannot bypass these callbacks by invoking native CRUD directly.

## Execution and conflicts

The CLI provisions two warm Node sidecars and never queues excess hook requests. A busy pool returns 503. Requests and responses are bounded to 64 KiB; raw/returned payload objects are limited to 48 KiB, depth 32 and 4,096 nodes, leaving room for context and protocol metadata. Context exceeding the total frame budget fails before a mutation. Sidecars have a two-second I/O deadline, bounded stdout, silent untrusted stderr and a 128 MiB V8 old-space limit. That V8 setting is not a universal process-memory cap.

Production hooks force `NODE_ENV=production`. The Node watchdog runs in a separate worker thread so a main-thread loop cannot prevent detecting a lost parent. Deadline/frame failures kill and reap the owned process; subsequent calls provision a replacement. On Unix, disposal kills each owned process group, reaps its Node child and joins the owned I/O threads. This also closes pipes inherited by ordinary child processes. A hook must not deliberately detach its own descendants from supervision. Application errors are never copied to HTTP or private logs; diagnostics identify the hook source with explicit location precision.

Update/delete read the target row and its internal PostgreSQL `xmin` before calling application code. The final mutation compares that revision and owner again in SQL. If another transaction updated or removed the record during validation, the write returns 409; it does not overwrite the newer row. Only a successful mutation and its existing after-hook outbox commit together. `xmin` is an internal, short-lived conflict token, not a durable public version or an ETag.

This protects one target record. It does not make arbitrary cross-record reads in a hook atomic with its eventual write. Use database constraints or an explicitly transactional domain operation for uniqueness, balances or multi-record business invariants. Application callbacks can have side effects; prefer pure validation/transformation because rejected/conflicting/cancelled writes do not roll back external effects.

## Verification

Real loopback HTTP tests cover missing-executor startup failure, transformation, schema revalidation, unauthorized-owner exclusion, bounded rejection fields, callback deadlines, concurrent updates returning 409, delete rejection and transactional after-hook counts. Private executor tests compile the generated placeholder and a real project module, exercise production mode, two-slot capacity, timeout recovery and secret-free failure metadata. Sidecar tests verify large-frame rejection and hard-loop termination/reaping separately from compiler instrumentation.

A related PostgreSQL regression was exposed by the HTTP hook fixture's default `search: []`: PostgreSQL supplies a null trigger-argument array for zero arguments. Additive migration 7 coalesces that array to empty so opt-out resources accept writes without indexing their data; previous migrations remain unchanged. The real HTTP test checks both successful writes and empty search text/vector.

Unexpected native errors emit their request reference and caller immediately. Stack symbolization runs separately with one diagnostic permit, no queue and a 32 KiB output budget. Concurrent failures explicitly report a caller-only diagnostic rather than waiting for symbolization. The underlying error value is never formatted.

The CLI's native before-hook executor currently requires Unix process-group supervision. Configuring before-hooks on Windows fails at backend startup with an explicit unsupported-platform diagnostic; it does not fall back to unguarded execution. Ordinary frontend/dev commands and resources without before-hooks remain available. No Windows process-tree quota or universal process-memory cap is claimed.

Checkpoint verification used PostgreSQL 18 owned by the disposable harness. Backend library tests passed 22/22; integration cases passed across the full run and the corrected focused foundation run (15 distinct cases). The hook executor's compiled-project scenario passed, including production mode, capacity and recovery; the separate Node watchdog test passed with a blocking main thread. The inherited-stdout process-group regression passed.
