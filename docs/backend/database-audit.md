# Read-only database audit

`brain db audit [--env development] [--limit 50] [--json]` observes the current app's configured
`DATABASE_URL`. It never searches another app, accepts an arbitrary connection override or applies
SQL changes. Production connections use the backend's existing verified-TLS policy.

The report contains current-database/current-role connection counts, readable table and index
statistics, selected PostgreSQL settings and review proposals. It excludes row values, query text,
connection secrets and other databases' or roles' statement statistics. An optional
`pg_stat_statements` section reports bounded numeric statistics and query IDs when installed,
preloaded and readable. Its absence or permission refusal is explicit, not an empty success claim.

One connection runs read-only session settings and a read-only transaction, rolled back at the end.
The audit has a 20-second total deadline, three-second connect deadline, two-second statements,
250-millisecond lock timeout and at most 200 rows per section (`--limit 1..200`). Settings temporarily
overridden by the audit are identified separately from their observed defaults. No statistics are reset.

Exit `0` means mandatory metadata was observed; optional sections may still be unavailable. Exit `1`
means a required section was incomplete. Exit `2` means invalid arguments or unavailable configuration.
Inspect each section's status and safe SQLSTATE. A slow query or large connection count alone does
not establish the cause of an outage.

Recommendations are review proposals, including quoted `ALTER ROLE ... IN DATABASE` statements when
appropriate. They are never executed. Shared-cluster settings are observations rather than permission
to tune the server globally. Review the workload and available privileges before applying a proposal
through your normal database change process.
