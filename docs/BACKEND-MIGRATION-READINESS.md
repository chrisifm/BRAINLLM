# Backend migration readiness

This is an acceptance ledger for replacing an existing application backend, not a promise that the published
framework already covers every requirement. Published baseline: **0.4.0**. Current development code checkpoint:
**3987d0b**. No application database, credentials, identifiers or files were modified during this assessment.

The requesting project's `docs/MIGRACION-BACKEND-BRAINLLM.md` must be read before approving its migration.
It was not present at the supplied application path during the initial audit. Its detailed requirements remain
an explicit pending input; this ledger must not be presented as a completed review of that document.

## Capability assessment

| Requirement | Published 0.4.0 | Development and remaining acceptance |
|---|---|---|
| Email/password and Google login | Native accounts, persisted sessions, optional JWT and Google recipes exist. | Real project identity, account linking, role and migration parity still require application tests. |
| Verification and recovery email | Native flows and optional private Resend delivery exist. | Configure the provider and verify delivery/replay/account enumeration behavior with synthetic accounts. |
| Persistent background work | PostgreSQL leases, retries and at-least-once processing exist. | Application jobs must be idempotent; verify real retries and side effects before removing the existing worker. |
| CRUD with permissions | Schema/policy and request-scoped `ctx.backend.resource()` exist. | A role is not an organization membership. Company access to a candidate-owned application needs relational authorization. |
| Amounts, dates, enums and relationships | App fields accept `str`, `num`, `bool`, `email`, `obj`; resource IDs are UUID. | Exact typed values, references/FKs, organizational scope and legacy bigint identity compatibility are open blockers. |
| PDF resume uploads | Uploads are private, content-validated PNG/JPEG/WebP images. The upload route allows 5 MiB plus multipart overhead; the general 64 KiB body limit is not universal. | General document/PDF storage, authorized attachment download, quotas, retention and transport limits must be implemented and tested end to end. |
| Stripe and PayPal webhooks | Native webhooks implement the BRAIN HMAC envelope, not those providers' protocols. | Provider-specific verification and header ingress, raw-body preservation, replay/idempotency and durable receipt are required. |
| Typo-tolerant search | `pg_trgm`, `unaccent`, full-text/trigram GIN indexes and declared search fields exist. | Declarative index control, weights/configuration, reindex CLI, migration/drift and representative query-plan tests remain incomplete. |
| Administrative panel | The published resource generator does not ship admin views. | Opt-in `--admin-ui` is integrated in development. Combined functional tests pass; the fixed edit fixture totals 30,720 B with zero budget headroom. Real native/browser acceptance is still required. |

## Required slices and acceptance

### Data model and migration safety

- [ ] Validate exact money semantics without assuming every JSON number is converted to floating point at storage.
  Define scale, currency and range, plus a lossless API representation. Preserve existing `num` contracts.
- [ ] Separate calendar dates from timezone-aware instants. Define accepted inputs and normalization; do not
  silently turn calendar dates into another local day.
- [ ] Add bounded enum contracts shared by validation, OpenAPI, generated clients and administrative controls.
- [ ] Model company to vacancy to application references with database-enforced existence, explicit delete rules,
  uniqueness and organizational access checks. A string field or the `company` role alone does not enforce a FK
  or tenant boundary. Check cross-company reads/writes and candidate ownership together.
- [ ] Preserve legacy bigint identity and all foreign references. Define external-ID mapping or an explicitly
  supported ID mode; never silently replace existing IDs or send unsafe integers through JavaScript numbers.
- [ ] Preview migrations with reviewed revisions/checksums, incompatible-row diagnostics and bounded backfills.
  Require an explicit apply step and a tested recovery plan. Startup must not silently alter a production schema.

### Behavioral compatibility beyond schema names

- [ ] Support bounded typed arrays where an existing contract requires them. A generic object field does not
  validate an array's element type, length, normalization or duplicate policy.
- [ ] Preserve partial-update semantics explicitly. Sending omitted fields to a full-replacement endpoint may
  erase data; a migration facade must distinguish omission, null, empty and unchanged values.
- [ ] Preserve authorized state transitions and side effects, not only enum membership. Transition preconditions,
  expiration and related timestamp changes must remain consistent under concurrent requests.
- [ ] Provide transactional multi-resource business commands where atomicity is required. Cross-row invariants,
  conditional state changes, uniqueness and an outbox must commit together; asynchronous after-hooks cannot
  substitute for those database transactions.
- [ ] Preserve public API paths, stable slugs, pagination totals, projections, statuses and localized error
  envelopes through a tested compatibility boundary. Generic resource response shapes do not prove parity.
- [ ] Plan authentication cutover explicitly: existing password algorithms, active opaque/JWT sessions, cookies,
  account linking and application-space selection. A new hash algorithm does not make old stored hashes compatible.
  Never silently invalidate every account or import old secrets as if they already used the native scheme.
- [ ] Compare search language, related-field vocabulary, synonyms, facets and eligibility/expiration filters.
  Existing trigram support alone does not establish matching results.
- [ ] Treat historical application findings as regression requirements. Reproduce them on the current framework
  before labeling them current defects, and preserve behavior that was already fixed.

### Documents and media

- [ ] Add a separate private document capability without weakening image decoding/re-encoding rules.
- [ ] Apply bounded content validation, authorization, quotas and cancellation before accepting PDF resumes.
  Client filenames and MIME headers cannot establish trust or become storage paths.
- [ ] Use immutable private keys, metadata/ownership, safe attachment responses, nosniff and explicit retention.
  Do not expose resumes as public static assets or claim malware scanning/disarming without actual coverage.
- [ ] Match multipart/body limits across Rust, Node, development proxy and nginx; test an allowed PDF larger than
  64 KiB, an oversized file, forged content, cross-user access, interrupted writes and cleanup.

### Payment provider ingress

- [ ] Preserve the exact signed body through every gateway and allow only the configured provider's headers.
- [ ] Verify Stripe's own signature protocol and timestamp using its endpoint secret; do not relabel the BRAIN
  HMAC envelope as Stripe verification. [Official signature guidance](https://docs.stripe.com/events/manage-webhook-endpoints#signature-errors).
- [ ] Verify PayPal using its documented certificate/signature protocol or official verification endpoint with
  the configured webhook ID. Untrusted certificate URLs require explicit SSRF controls and bounded retrieval;
  do not fetch arbitrary URLs from webhook headers. [Official PayPal guidance](https://developer.paypal.com/api/rest/webhooks/rest/).
- [ ] Isolate provider, environment and endpoint identity in deduplication keys. Acknowledge only after committing
  the durable receipt and queue job. Repeated deliveries must not duplicate payment side effects.
- [ ] Test valid/forged/modified/stale events, duplicate headers and retries offline with synthetic fixtures;
  separately verify sandbox behavior when operator credentials are explicitly configured. Never log payloads,
  credentials, raw signatures or internal traces to public production responses.

### Search and operational parity

- [ ] Expose bounded declarative search/index settings and verify actual index definitions for schema drift.
- [ ] Expose the existing bounded search rebuild operation through a resumable CLI with progress and safe failure
  diagnostics. Representative tests must distinguish returned rows from evidence of an index-backed plan.
- [ ] Preserve filters, sorting, pagination, accents and typo behavior; measure representative datasets rather
  than assuming PostgreSQL always chooses an index.
- [ ] Verify authentication, role/company boundaries, CRUD, documents, webhooks, queues and admin flows before
  deleting the existing backend. Framework component tests do not establish whole-application parity.

## Evidence anchors

- `crates/brain-backend/src/contract.rs`: field kinds, resource declaration and role/owner policy.
- `crates/brain-backend/src/db.rs`: resource UUID/JSONB storage, owner FK, search/index migration and rebuild.
- `crates/brain-backend/src/search.rs`: bounded full-text/trigram queries and authorization conditions.
- `crates/brain-backend/src/http.rs`: general 64 KiB request limit and separate image multipart limit.
- `crates/brain-backend/src/upload.rs`: accepted image formats and content/decode limits.
- `crates/brain-backend/src/webhooks.rs`: BRAIN signature frame, encrypted receipts and durable delivery work.
- `runtime/server/server.mjs`: image-specific proxy limit and current BRAIN webhook-header projection.
- `docs/CONTINUATION-CHECKLIST.md`: exact component, integrated-test and publication evidence.

The audited resource contract, search, database implementation and migrations are unchanged between the published
baseline and checkpoint3987d0b. Native socket/admin work must not be mistaken for completion of relational migration.
