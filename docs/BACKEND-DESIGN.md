# Backend design note — 2026-10-29

Status: experimental vertical slice; no production topology or performance winner has been selected.

## Study, not an API clone

Reviewed the current [Fastify releases](https://github.com/fastify/fastify/releases) (stable 5.12.7; 6 alpha remains
prerelease), [lifecycle](https://fastify.dev/docs/latest/Reference/Lifecycle/),
[validation/serialization](https://fastify.dev/docs/latest/Reference/Validation-and-Serialization/) and
[encapsulation](https://fastify.dev/docs/latest/Reference/Encapsulation/) before implementation.

The applicable architecture lessons are schema-driven input/output, a fixed request sequence, prebuilt routing,
resource-local context and explicit dependencies. BRAINLLM does not copy Fastify's code, decorators, method names,
plugin registration tree or hook vocabulary. The initial implementation does not claim compiled output serialization.

LLM-oriented deviations:

- Literal `schema.jsbrain` and `policy.jsbrain` objects replace registration callbacks: an agent edits one small
  contract and can inspect the resolved result without running application code.
- Four explicit grants replace middleware ordering as the source of authorization. Unknown properties and public
  writes fail early. There is no implicit privileged role bypass.
- Fixed CRUD routes replace route-by-route boilerplate. Application-specific service/repository functions are future
  work, not empty generated files that imply a capability exists.
- Postgres JSONB per resource permits runtime-generated contracts without inventing an ORM/query DSL. Bind
  parameters are mandatory; incompatible schema changes require review. SQLx runtime queries are used, not its
  compile-time query macros; database-free builds must remain possible.
- Durable after-write hooks replace fire-and-forget callbacks. They share the resource transaction, and retries,
  leases and failures are inspectable by id. Handlers are explicitly at-least-once, not described as exactly-once.
- Compact MCP discovery returns names first and one contract on request, keeping a large app out of a small model's
  context. Full MCP remains the default; a six-tool minimal profile is also available.

## Rust choices

Latest stable direct versions were checked against crates.io on 2026-10-29; Cargo.lock records the resolved graph.

| Need | Library | Reason and limits |
|---|---|---|
| HTTP/router/multipart | axum 0.8.9 + Tokio 1.54.1 | maintained Tower/Hyper ecosystem; own code handles only Brain contracts |
| CORS/compression/deadline | tower-http 0.7.1 | shared HTTP infrastructure; no custom compressor |
| Rate limits | tower_governor 0.8.0 | peer-IP quota with request-driven pruning (no detached cleanup task); proxy identity policy still pending |
| Postgres/pool/transactions | SQLx 0.9.0 | Postgres only, 8 connections, parameterized queries, transactional migrations |
| Passwords | RustCrypto argon2 0.6.0 | Argon2id, random salts, bounded blocking work |
| Tokens/identifiers | uuid 1.29.0 + sha2 0.11.1 | opaque random session tokens, stored as hashes, revocable in Postgres |
| Images | image 0.25.10 | PNG/JPEG/WebP content decoding and re-encoding; optional formats disabled |
| Static precompression | brotli 9.1.1 | build-time quality 11, cached by bytes and settings; gzip fallback |
| File watching / platform calls | notify 8.2.1 / libc 0.2.191 | existing infrastructure updated to latest stable direct releases |
| Contract interchange | serde 1.0.229 + serde_json 1.0.154 | JSON data only; BrainScript still uses the framework's parser |

Selection is based on current primary documentation and maintained releases. No security audit of those libraries or
comparative throughput benchmark has been completed for this slice. Those are acceptance work; this note does not
infer safety or speed from a crate's popularity. No third-party frontend parser, bundler or minifier is introduced.

Queue-engine follow-up: current [apalis-postgres 1.0.0](https://docs.rs/apalis-postgres/1.0.0/apalis_postgres/)
and [sqlxmq 0.6.0](https://docs.rs/sqlxmq/0.6.0/sqlxmq/) were also checked as maintained Postgres queue candidates.
This slice keeps the small typed queue/outbox contract on the existing SQLx transaction; it does not claim an owned
queue engine is preferable to those libraries. Compare adapters for transaction ownership, lease recovery,
idempotency, cancellation and deployment cost before selecting the production engine. The app-facing schema/handler
contract should remain stable if the storage engine changes.

## HTTP ownership remains a measured decision

The Rust resource API currently runs separately on loopback through `brain backend serve`. Existing Node SSR stays
unchanged. This enables contract, storage, policy and queue verification without silently replacing deployments.

Before selecting the managed topology, measure both:

1. Rust owns the public socket, auth, resources, queues and files; an owned Node renderer handles only SSR.
2. Node owns the public socket and SSR; it forwards authenticated resource operations to an owned Rust worker.

Use the same Postgres data and connection budget, same hardware and warmup, and publish raw requests/s and p50/p99
for login, list/search and upload plus end-to-end page navigation. Include process startup, IPC, memory, cancellation,
shutdown and failure recovery. Compare with the authorized reference Fastify API, without copying its implementation.
No architecture is declared faster before those measurements.

## Navigation graph

The build emits each page's actual outgoing routes and immutable assets as `brain-next`. The router validates those
candidates against live links and link opt-outs. Visibility and intent remain runtime inputs. A dedicated hashed worker
fetches likely pages and extracts chunk references; it does not execute route code or persist personal HTML. Limits:
two speculative requests, twelve automatic targets, twenty-four retained pages, 512 KB/page, 4 MB/tab, six seconds per
fetch. Actual navigation waits at most 150 ms for unfinished speculation. Writes cancel requests and advance a tab-local
epoch; an older epoch cannot populate a later session's navigation. Private/no-store HTML and dynamic no-cache HTML
are never reused for navigation; static servers mark immutable-by-build HTML with X-Brain-Static, and public cache
lifetimes are bounded by their declared max-age. Worker failure falls back to browser fetching.

This maps views and assets, not arbitrary backend effects. Compiler-derived data contracts and safe read-only data
warming need explicit purity/caching contracts before the framework can speculate on them. Worker usage alone is not
proof of a latency improvement; runtime regressions and a real-browser comparison are required.


Dependency freshness rechecked on 2026-10-30 against the official crates.io API: all 16 direct infrastructure/dev
crate versions selected across brain-backend/brain-cli/brain-pack/brain-serve equal their latest stable release.
This includes Argon2, Axum, Brotli, http-body-util, image, libc, notify, serde, serde_json, sha2, SQLx, Tokio, tower,
tower-http, tower_governor and UUID. Cargo.lock remains reproducible; no prerelease or unverified version was selected.
Registry source: https://crates.io/api/v1/crates/<crate-name> (`crate.max_stable_version`). Recheck when adding a crate
or preparing a release; this dated observation is not an evergreen freshness guarantee.

### Direct dependency freshness audit — 2026-10-30

The crates.io API max_stable_version was checked for every direct third-party Rust dependency (including dev dependencies): argon2 0.6.0, axum 0.8.9, brotli 9.1.1, http-body-util 0.1.5, image 0.25.10, jsonwebtoken 11.1.0, libc 0.2.191, notify 8.2.1, reqwest 0.13.5, serde 1.0.229, serde_json 1.0.154, sha2 0.11.1, slug 0.1.6, sqlx 0.9.0, tokio 1.54.1, tower 0.5.3, tower-http 0.7.1, tower_governor 0.8.0, url 2.5.8 and uuid 1.29.0. All configured direct versions matched the latest stable registry version at this audit. Keep Cargo.lock for reproducibility; future registry releases require a fresh compatibility audit rather than a timeless claim that these pins remain latest.

### Exact resource types — 2026-10-30

Exact types (`int`, `decimal(p,s)`, `date`, `datetime`, `enum`, `list<str>`), defaults and declarative constraints
added no dependency: parsing, calendar math and canonical decimal handling live in `brain-backend/src/types.rs`.
Decimals travel as strings because both JavaScript and `serde_json` without `arbitrary_precision` read JSON numbers
as binary floats; enabling `arbitrary_precision` would change number handling across the whole backend. Records stay
in the JSONB `data` column; exactness is guaranteed by validated canonical strings plus CHECK constraints that cast
losslessly to `numeric(p,s)`. Constraints are recorded under a reserved `$constraints` key of the existing schema
row, so legacy contracts keep a byte-identical recorded value and no migration of `_brain_resource_schemas` is needed.
