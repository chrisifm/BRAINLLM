# Computed data backfills

`brain data backfill` fills an absent field in existing rows using a bounded deterministic expression. It is an
operator workflow over the current app's selected database. It never runs automatically during build, startup or
schema migration, and does not execute application JavaScript. Literal declared defaults remain the separate
`brain data defaults` workflow.

## Declare and review

Start with an optional target field, review its resource contract and apply the framework/resource migrations
explicitly. Framework migration 0029 creates the database identity and durable checkpoint tables. Preview requires
these tables already installed; it does not create schema. Do not make the field required until the backfill and
the resulting data have been reviewed. A unique target is checked by its existing database constraint; duplicates
roll back the current batch.

Create `server/backfills/codes.jsbrain`:

```javascript
<meta server>
export default {
  resource: 'records',
  field: 'code',
  compute: row => row.data.label.trim().toLowerCase() + '-' + row.id
}
```

The resource and target must exist. Inputs may read `row.id` and declared unrestricted `row.data.<field>` values.
Expressions accept scalar JSON literals, `+` on two strings or two numbers, numeric `-`, `*`, `/`, nullish `??`,
`trim()` and `toLowerCase()`. There are no imports, globals, SQL, network calls, asynchronous functions, blocks or
mutation. Division by zero and nonfinite results fail validation. Arithmetic uses finite JavaScript Number
semantics; native exact field contracts still validate the result. Use string-based declared decimal fields for
exact decimal data, rather than expecting floating-point arithmetic to become exact.

The target cannot be a slug or a restricted field. Resources with application mutation hooks are refused, as are
unknown database triggers, rewrite rules and database row-level policies. The command does not apply application
user/owner grants: it requires operator database authority. Native constraints and known search, reference, slug,
per-owner and audit triggers remain active. Their installed functions are trusted, not cryptographically attested.

## Preview, then apply one batch

```sh
brain data backfill codes --env development --limit 100 --json > backfill-plan.json
# Review ok, invalidRows, eligibleRows, sampledRows and the exact target before approving.
brain data backfill codes --env development --limit 100 --apply --request backfill-plan.json --json > backfill-progress-1.json
brain data backfill codes --env development --limit 100 --apply --request backfill-progress-1.json --json > backfill-progress-2.json
```

`--env` is required. Applying requires explicit operator approval of the preview and its unchanged request; the
framework does not grant that approval. Each invocation processes one batch. Continue using the returned request
until `complete:true`; a repeated older request is a no-op and returns current progress. Never redirect output over
the request file being read: shell redirection would truncate it first. Keep these local plan/progress files out
of version control. Requests must be regular, nonhidden app-relative JSON files, without symlinks, at most 16 KiB.

Existing values, including explicit nulls, are preserved. Add `--include-null` to both preview and apply only when
nulls should also be filled. Preview validates only its selected batch in a repeatable-read, read-only transaction.
Invalid rows produce `ok:false` and no approvable request. Apply revalidates locked rows against the current resource
contract. Row updates, native audit events and checkpoint progress commit together; a failure rolls back this batch,
while earlier committed batches remain. `updated_at` is preserved; PostgreSQL row versions still change.

## Bounds, restart and identity

`--limit` defaults to 100 and accepts 1..1000. A metadata preflight checks the aggregate one-MiB document transfer
budget before fetching eligible documents; this is not a one-MiB process-heap guarantee. Source is at most 16 KiB,
with 128 expression nodes, depth 16, twenty source fields, 4096-byte string literals and a 16-KiB computed JSON value.
The transaction has a thirty-second total deadline, five-second statement timeout and 500-ms lock timeout.
Locked rows are not skipped; a lock failure does not advance progress.

The request binds the expression, complete resource contract, selected environment/normalized connection, stable
database UUID, time cutoff, cohort and checkpoint revision. Passwords, row IDs, row contents and SQL are not emitted.
Progress survives both CLI and PostgreSQL restarts. Definition/schema/target changes require review; they cannot
silently resume the old job. Use a new definition name for a separately reviewed migration.

The cohort uses native database-managed `created_at` and UUID ordering. Direct SQL can backdate inserted rows and
database copies can preserve the identity UUID. Connection binding catches a different configured target; it is not
server attestation or an anti-cloning guarantee. Back up the database before applying migrations or backfills. Older
binaries can refuse a newer migration ledger; this workflow is not an automatic database downgrade or rollback plan.
