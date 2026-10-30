# PostgreSQL foundation review

DBA review of the experimental Rust backend, 2026-10-30. This document separates
observed implementation from the proposed foundation. Source inspection is not a
production security audit or measured query-plan evidence. No existing developer
database was accessed during this review.

## Observed foundation and prioritized findings

The backend already installs users, hashed opaque sessions, one-use auth-token
storage, auth events, upload metadata, resource contracts, persistent login limits
and leased jobs. Resource writes and after-write jobs share a transaction. Values
are bound; generated identifiers are validated. Serving verifies resource contracts
and does not create tables. Preserve these properties.

| Priority | Evidence at review | Finding and concrete action |
|---|---|---|
| P1 | `crates/brain-backend/src/auth.rs:138` | Login expiry depends on deleting one of the first 100 expired rows. The upsert increments an expired surviving key without resetting its window. Reset attempts and expiry inside the upsert when the existing expiry has elapsed. |
| P1 | `crates/brain-backend/src/db.rs:28` | Repeated `CREATE IF NOT EXISTS` is not a migration history or schema-drift check. Add an ordered migration ledger and checksums, under the existing advisory transaction lock. Adopt the existing baseline without rewriting its SQL. |
| P1 | `crates/brain-backend/src/db.rs:53` | Full-text indexing `data::text` includes property names and every stored value. Index only declared searchable text fields, with explicit weights. Never offer a global cross-owner spelling dictionary. |
| P1 | `crates/brain-backend/src/auth.rs:188` | Concurrent successful logins can exceed the session cap because pruning and insertion have no per-user serialization. Lock the user row before pruning/inserting, including JWT sessions. |
| P1 | `crates/brain-backend/migrations/0001_backend.sql:29` | Deleting a user cascades upload metadata but cannot delete filesystem objects. Enqueue owned object deletion before removing metadata or retain a deletion tombstone/outbox. Never infer that a cascade removes storage. |
| P2 | `crates/brain-backend/src/db.rs:8` | Pool acquisition has a deadline, but SQL execution has no database-side statement or lock deadline. HTTP cancellation alone is not a PostgreSQL resource budget. Configure bounded runtime statements and idle transactions separately from migration timeouts. |
| P2 | `crates/brain-backend/src/db.rs:93` | Owner/search optional predicates and chronological ordering share one query. Split blank-search, scoped-search and broad-policy search so plans are measurable; add matching chronological indexes and keyset pagination. Offset remains compatible. |
| P2 | `crates/brain-backend/migrations/0001_backend.sql:14` | Session pruning needs `(user_id,expires_at DESC,token_hash)`, and expired sessions/tokens/limits need expiry indexes and explicit bounded cleanup. |
| P2 | `crates/brain-backend/migrations/0001_backend.sql:21` | Auth events lack correlation/outcome and retention indexes. Add bounded allowlisted metadata and request IDs; keep secrets and arbitrary error messages out. |
| P2 | `crates/brain-backend/src/jobs.rs:41` | Final expired-lease cleanup can update every matching job at once. Claim a bounded batch with `SKIP LOCKED`, and index terminal retention by `(finished_at,id)` with a terminal-state predicate. |
| P2 | `crates/brain-backend/src/upload.rs:31` | Images are rewritten to PNG; original bytes, dimensions and rendition identity are absent. Preserve a private validated original and model versioned derivatives explicitly. Do not claim original preservation or compression savings today. |
| P2 | `crates/brain-backend/src/db.rs:184` | Startup verifies resource fields only. Verify the foundation migration version and required extensions before accepting requests; return an actionable migration/configuration diagnostic. |

Line numbers are the reviewed snapshot; parallel work can move them. Missing JWT,
Google identity and cloud-storage flows are capability gaps, not evidence that the
existing opaque-cookie flow implements those protocols.

## Incremental migration contract

Keep `0001_backend.sql` and `0002_jobs.sql` unchanged. Add new ordered migrations;
never drop user/resource tables or overwrite resource contracts to make migration
pass. A new empty project gets all core tables when `brain backend migrate` runs,
even with zero declared resources. Creating an app does not silently create a
database or use a developer's existing cluster.

The ledger stores version, name, SHA-256 of the embedded SQL and application time.
The advisory transaction lock covers ledger lookup, pending DDL and version write.
Recognize the existing two migrations through reviewed table/column checks before
recording their baseline. Reject changed checksums and unknown newer versions.
`IF NOT EXISTS` alone must not mark an incompatible pre-existing object compatible.
Return a schema drift code and object name without connection strings or SQL data.

Add nullable columns and additive tables first; validate constraints/backfills in
bounded explicit steps. Use `NOT VALID` followed by `VALIDATE CONSTRAINT` where
supported and useful. Existing incompatible data gets a diagnostic and a migration
plan, never deletion. Index creation for large deployed tables needs a separate
nontransactional concurrent-index step; it cannot be hidden inside the current
all-in-one transaction. Fresh empty projects can build indexes transactionally.

## Core relational model

| Entity | Incremental shape and invariants |
|---|---|
| Users | Retain UUID, normalized unique email, password hash and existing roles. Add `updated_at`, optional disable time and auth-version counter. Check normalization before validating a new normalized-email constraint; report collisions without merging accounts. Provider-only accounts use a separate identity flow, never a fake usable password. |
| Identities | `(provider,subject)` unique, user FK, issuer, verified-email snapshot and timestamps. Google `sub` is identity. Matching email never links an existing account automatically; linking requires a current authenticated account and provider proof. |
| Persistent sessions | Stable session UUID, user FK, creation/last-use/absolute-expiry/revocation timestamps. Keep existing opaque sessions valid during a bounded transition. Do not place email, IP, credentials or arbitrary request headers in JWTs. |
| Refresh generations | Random token digest PK, family/session FK, parent/replacement relation, issued/expiry/used timestamps. One live successor per parent. Retain used digests until family expiry to detect reuse; deletion on rotation destroys evidence. |
| Auth challenges | Digest, purpose/provider, expiry, consumed time and optional session/user FK. Consume by conditional update in the same transaction as identity/session creation. Separate nonce/PKCE/state purposes; never store raw verifier or token in logs. |
| Audit events | Append-only ID, optional actor FK `ON DELETE SET NULL`, event/outcome code, request ID, resource identifier and bounded sanitized metadata. No passwords, JWTs, cookies, SQL values, upload content or raw arbitrary exceptions. |
| Media | Retain upload IDs/ownership. Add private original object identity, byte size, detected MIME, dimensions, digest, alt text, caption, status and version. Separate rendition rows with unique `(upload_id,version,preset)`, immutable object key, format/dimensions/bytes/digest and provider generation. |
| Jobs | Preserve at-least-once delivery and transactional hooks. Add check constraints for nonnegative attempts and lease/state coherence after checking legacy rows. Idempotency keys currently expire when jobs are pruned; document that horizon. |

Lock a session/family row during refresh. An unused, unexpired token is consumed
once and replaced transactionally. A second use revokes the family and associated
access session, with an audit event. Concurrent client refreshes need single-flight
coordination; do not add a broad reuse grace period that masks theft. Validate JWT
signature, allowed algorithm, `iss`, `aud`, `exp`, `sub`, `sid` and token purpose.
An active session lookup preserves immediate logout, disablement and role changes.
The signing-key ID is not permission to fetch an arbitrary remote key URL.

## Search model and bounds

Configure `pg_trgm` and `unaccent` explicitly. They are trusted PostgreSQL extensions,
but managed-service availability and the migrator's database `CREATE` privilege
still need checking. PostgreSQL 18.6 is the current stable minor in the official
version policy on the review date; 19 is a release candidate. Target the latest
stable minor, not a prerelease. [Version policy](https://www.postgresql.org/support/versioning/),
[pg_trgm](https://www.postgresql.org/docs/18/pgtrgm.html),
[unaccent](https://www.postgresql.org/docs/18/unaccent.html).

Maintain `search_text` and `search_vector` derived only from searchable fields.
Exclude unknown/private/object fields by default. Explicit title/name fields get
weight A, summary B, body D; persist the configuration revision. Use fixed
configuration names and normalize accents consistently for writes and queries.
A trigger-maintained materialized text/vector allows dictionary-based normalization
without pretending configurable `unaccent` is immutable. Check `pg_proc.provolatile`
before selecting any generated-column expression. Dictionary changes require a
reviewed rebuild. [Text search controls](https://www.postgresql.org/docs/18/textsearch-controls.html).

Create a GIN index on `search_vector`, a GIN trigram index on normalized
`search_text`, and owner/chronological B-tree indexes. Trigram similarity over a
long concatenated body is not a reliable substitute for word similarity. Prefer
word similarity for typo fallback, with the indexed column on the correct side of
the index-supported operator. GiST can serve nearest-neighbor distance ordering;
GIN serves selective operator matches. Select from measured plans, not an assumed
universal winner. [Index operators](https://www.postgresql.org/docs/18/pgtrgm.html).

The API retains search <=256 UTF-8 bytes, limit 1..100 and offset 0..10000. Add a
token-count bound and a database-side search deadline (initial proposal 1 second).
Blank queries use chronological lists; one/two-character queries use exact/prefix
behavior rather than broad typo matching. Set similarity thresholds with
transaction-local configuration, never leave request state on pooled sessions.
FTS and typo branches each include the same owner/tenant policy before their
candidate limit. Union, deduplicate by record ID, score with fixed weights, and
break ties deterministically by UUID. Return only authorized fields. Avoid exact
global result counts and cross-tenant suggestion vocabularies.

A candidate limit bounds returned ranking input, not PostgreSQL's entire scan;
statement deadlines and plan measurements remain necessary. Public/broad reads
are available only when the resource's explicit policy grants them. Existing
`owner` means a user, not an organization: do not claim multi-tenant isolation or
infer a company tenant from its role. A future tenant model needs tenant IDs,
membership authorization and tenant-qualified uniqueness/FKs throughout.

## Index and retention proposal

| Table/use | Proposed additive index or bound |
|---|---|
| Sessions per user | `(user_id,expires_at DESC,token_hash)` and `(expires_at,token_hash)` |
| One-use auth tokens | `(expires_at,token_hash)` and `(user_id,purpose)` |
| Auth events | `(created_at,id)` and `(user_id,created_at DESC,id)` |
| Login limits | `(expires_at,key)`; expired-window reset independent of cleanup |
| Upload library | `(user_id,created_at DESC,id)`; FK indexes on rendition/upload and deletion outbox |
| Terminal jobs | `(finished_at,id) WHERE state IN ('succeeded','dead')` |
| Resource lists | `(created_at DESC,id)` and `(owner,created_at DESC,id)` |

Run explicit bounded maintenance, initially 1000 rows per invocation with a finite
iteration budget. Remove expired challenges/tokens/sessions; retain consumed
refresh digests until family expiry. Propose 90 days for sanitized auth/audit events
and preserve existing 30-day terminal-job retention. These are configurable project
defaults, not legal retention guarantees. Report counts and truncation without row
contents. Media deletion first records an idempotent storage deletion task; retries
must target the recorded generation/version, never a newly replaced object.

## Database privileges and request policy

Use a DDL-owning migrator credential separately from runtime. Runtime is neither
table owner, superuser nor `BYPASSRLS`; grant only needed DML, sequence use and schema
usage, with no role/schema creation or arbitrary extension installation. Keep the
migrator credential out of the serving process. Fix trusted `search_path` and
schema-qualify privileged functions. Explicitly inspect grants on existing databases
rather than revoking unrelated users' permissions automatically.

Current isolation is application policy, not RLS. Introducing RLS requires both
`USING` and `WITH CHECK`, transaction-local identity on the same acquired connection
as every statement, `FORCE ROW LEVEL SECURITY` where appropriate, explicit worker
policies and tests using the runtime role. Owners and bypass roles otherwise bypass
policies. A connection pool setting alone cannot safely carry request identity.
RLS based on an application-set custom setting is defense against forgotten filters,
not protection from a compromised trusted runtime that can set that identity.
[Row security](https://www.postgresql.org/docs/18/ddl-rowsecurity.html).

Configure runtime `statement_timeout`, `lock_timeout`, and
`idle_in_transaction_session_timeout`; choose separate migration budgets. Avoid
logging bind parameters. A safe database diagnostic can expose allowlisted SQLSTATE,
operation/resource and request ID, with a known constraint-to-fix mapping. Do not
discard all diagnostic categories or expose PostgreSQL's arbitrary detail/message.
[Connection defaults](https://www.postgresql.org/docs/18/runtime-config-client.html).

## Media and cloud runtime configuration

Use UTC `year/month/upload-id/version/preset.ext` keys generated by the backend;
filenames never become paths. Preserve a private original after content validation,
strip metadata from public derivatives, and record responsive presets/crops rather
than overwriting outputs. Bound original/decoded/output bytes, dimensions, pixel
count, aggregate owner quota, worker concurrency and retained versions. Original
byte preservation does not mean originals may be served as trusted public images.

Filesystem writes and PostgreSQL commits are not one transaction. Record staged
metadata and deterministic operation IDs, persist immutable outputs, then finalize
metadata; reconcile abandoned stages after a bounded grace period. Recovery only
deletes known owned objects and reports failures. A native gallery lists authorized
metadata, dimensions, rendition URLs and accessibility fields without scanning disk.

Optional Google Cloud Storage requires bucket, permitted object prefix, provider
choice and Application Default Credentials configured at runtime. Keep buckets
private, use least-privilege object access and conditional writes tied to generation.
Never store credential JSON or signed URLs in manifests/audit events. Discoverable
config/templates can ship without credentials; real bucket permissions, upload,
signed access and deletion need an explicitly configured disposable cloud target.
Do not claim cloud integration tested from a template alone.

## Acceptance matrix for the owned disposable harness

The following is a proposed acceptance contract, not a claim that these cases ran:

1. Empty database plus zero resources installs all core tables; repeat migrations
   are no-ops. Existing baseline rows survive byte-for-byte; checksum drift and
   incompatible existing objects fail with an actionable diagnostic.
2. Expired login keys beyond the first cleanup batch reset on demand. Concurrent
   login/refresh runs enforce caps, one successor and reuse revocation. Logout and
   user disablement invalidate JWT use immediately. Google email collision cannot
   link or take over an existing account.
3. Synthetic 100000-row resource across two owners, mixed selective/common words,
   accents, misspellings and private fields. Run `ANALYZE`, then capture
   `EXPLAIN (ANALYZE,BUFFERS,FORMAT JSON)` for blank list, owner list, exact FTS,
   typo fallback, very common term and short query, using runtime credentials.
4. Selective FTS/trigram cases demonstrate their intended index paths; chronological
   pages avoid a whole-table sort. Do not disable sequential scans to manufacture
   evidence: tiny or nonselective fixtures may legitimately use them. Record row
   estimates, actual rows, buffers, sort spills, duration and timeout outcome.
5. Verify owner B cannot influence A's matches, score, suggestions or returned
   metadata. Unauthorized direct SQL is separately tested only when RLS is actually
   installed. App policy tests must not be mislabeled RLS tests.
6. Two workers claim disjoint jobs; stale leases cannot acknowledge new claims;
   final-lease cleanup/pruning remain bounded. Hook rollback leaves no orphan job.
7. Media failure at every persist/finalize boundary leaves recoverable metadata and
   owned cleanup records; duplicate callbacks cannot delete a replacement version.
8. Runtime role cannot create/drop tables or extensions; migrator can apply only
   the intended schema setup. Capture engine/extension versions, fixture seed,
   database settings and source revision, never connection strings or private data.

No latency target is declared met without those plans and a repeatable benchmark.
Provider credentials, DNS/HTTPS, JWT signing material and real GCS permissions are
deployment configuration, not values that can safely be invented by a scaffold.

## Implemented foundation increment (2026-10-30)

This increment adds `0003_foundation.sql`; migrations 0001 and 0002 remain byte-for-byte unchanged. `db::migrate` now writes an ordered `_brain_migrations(version,name,checksum,applied_at)` ledger using SHA-256 under the advisory transaction lock. Unknown versions, checksum changes, incompatible column types/nullability, missing baseline keys/referential actions, an unsafe verified default or role constraint fail with `BDB_SCHEMA_DRIFT`. Legacy 0001/0002 tables are checked before recording their baseline. Startup checks the ledger, required extensions, important foundation columns, media deletion triggers and each resource's field/search contracts, shape and trigger arguments. These are targeted structural checks, not a complete proof that every arbitrary DBA alteration has been detected.

Runtime connections set `statement_timeout=5s`, `lock_timeout=2s`, `idle_in_transaction_session_timeout=10s`, and a fixed search path. Explicit migration transactions use 60s/5s/60s instead. Search transactions lower the statement deadline to one second and set their word-similarity threshold transaction-locally. Sanitized SQL errors expose only an allowlisted category, never database error detail or bound values. Migration accepts at most 128 resources. Transactional index building remains appropriate to new/small installations; large deployed tables still need a reviewed concurrent-index rollout outside this transaction.

`Resource.search: Vec<String>` defaults empty and permits at most eight unique declared `str` fields. Example contract JSON: `{"search":["title","summary"]}` alongside the existing resource name, fields and policy. No value or field name enters the search vector unless explicitly listed; object/email/unknown fields are rejected. The CLI/compiler extraction must preserve this field before promising author syntax support. Title/name are weight A, summary B, other explicitly allowed text D. A trigger maintains accent-normalized `search_text` and `search_vector`; GIN FTS/trigram and chronological/owner indexes are installed. An old generated `search` column can remain on an adopted resource table, but its old broad index is removed and application queries do not use it.

Blank lists, one/two-character escaped-prefix queries and longer FTS/word-similarity queries have separate SQL paths. Both search candidate branches carry the caller's owner scope before their 500-row candidate limits; broad reads still require the HTTP policy check. SQL uses fixed operators/configuration and bind values. Search accepts at most 256 bytes, sixteen whitespace tokens, limit 1..100 and offset 0..10000. `search::Cursor {created_at,id}` provides an internal chronological keyset option for blank lists; HTTP cursor extraction/output is not implied by that API. It is rejected for ranked searches. No global suggestions, vocabulary or counts are exposed. Candidate limits do not bound the entire database scan; the deadline and measured plans remain necessary.

Changing an existing resource's search allowlist is a contract change and fails closed until a reviewed explicit migration updates that contract/trigger. No full-table rebuild is hidden in startup or migration. `db::rebuild_search(pool,resource,limit,after)` explicitly processes at most 1000 rows, returns the next UUID and a `possiblyMore` indicator; it does not skip locked rows and silently advance past them. Existing populated resources need that explicit rebuild after a reviewed allowlist migration. No performance target is claimed from source inspection.

`auth::reserve_session(tx,user_id)` takes the user row lock, prunes expired/revoked/excess sessions and retains nine slots before the caller inserts the tenth in the same transaction. All opaque/JWT creators must use it. Login's persistent limit upsert resets its own expired window independently of cleanup batch ordering. The new synthetic acceptance test includes twenty competing session creators; its execution status must be reported separately from the implementation.

Additive identity/JWT/media contracts now available to their flow implementations:

- `_brain_users`: `updated_at`, nullable `disabled_at`, nonnegative `auth_version`.
- `_brain_sessions`: unique `session_id`, `created_at`, `last_used_at`, nullable `revoked_at`; existing opaque digest and expiry stay valid.
- `_brain_identities`: unique `(provider,subject)`, user FK, issuer, verified-email snapshot and timestamps. This table alone does not link identities or verify provider proof.
- `_brain_refresh_families`: UUID ID, nullable session FK, user FK, captured nonnegative `auth_version`, creation/expiry/revocation. Session deletion sets the reference null instead of deleting reuse evidence. `_brain_refresh_tokens` retains digest, family, unique parent, deferred replacement, issuance/expiry/used timestamps. Flow implementations must lock the family and revoke on reuse; the schema alone does not implement rotation. Expired-family cleanup can clear links only after family expiry.
- `_brain_auth_challenges`: digest, allowlisted purpose/provider, optional user/session, expiry and consumption time. Never a raw OAuth state, PKCE verifier, JWT or password.
- `_brain_uploads`: optional original identity/provider/generation, original bytes/MIME/digest, dimensions, alt/caption, status and version. Legacy originals are not invented from rewritten PNG bytes.
- `_brain_media_renditions`: `(upload_id,version,preset)` key with immutable provider/object identity, generation, format/dimensions/bytes/digest.
- `_brain_media_deletions`: tombstones survive user/upload cascades. BEFORE DELETE triggers record legacy owned PNG/thumbnail identities and any explicitly stored original/rendition identity. `claim_media_deletions(limit<=100)` leases a batch for 30 seconds, at most ten attempts; `ack_media_deletion` requires the same unexpired lease. A filesystem/cloud worker must validate its configured root/prefix and recorded generation before deleting. No filesystem operation is claimed from the SQL trigger.

`db::cleanup(pool,limit)` performs one explicit finite pass with a total 1..1000 row budget. It removes expired tokens/challenges/limits, eligible sessions, refresh generations only after family expiry, empty expired families, sanitized auth events older than 90 days and completed media tombstones older than 90 days. It returns counts and `possiblyMore`, never row contents. Queued-job retention and final-lease cleanup are owned by the queue implementation. A completed pass does not claim that all expired rows have been exhausted.

Source verification completed for this increment: PostgreSQL 18 accepted 0001+0002+0003 in one transaction in a newly created, owned disposable cluster, including both required extensions; that cluster was stopped and deleted. Rust integration execution, 100000-row authorization/search acceptance and query-plan results are separate gates and are not asserted by this paragraph. No developer database or real cloud bucket was used.
