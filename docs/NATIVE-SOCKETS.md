# Native authenticated invalidation sockets

This is an opt-in Rust backend transport. The frontend client, development proxy, production Node proxy and nginx
integration are separate integration work; no automatic browser subscription or cross-process delivery is claimed.

## Activation and lifecycle

Set `brain_backend::Config.sockets` to `Some(brain_backend::http::SocketConfig::new(origins)?)` explicitly.
`None` omits the Upgrade route and preserves ordinary HTTP resource behavior. The list contains 1..8 distinct,
canonical HTTP(S) frontend origins, each at most 2048 bytes and also present in `Config.origins`. Credentials,
wildcards, paths, query strings and fragments are rejected. Do not derive allowed origins from request headers.

`brain_backend::serve` closes its upgraded connections during graceful shutdown. An embedding Rust host can use
`http::router_with_socket_control(pool, config)` to obtain `(Router, SocketControl)`. Call `shutdown()` before draining
that host's owned HTTP listener, then await `closed()`. This returns an error after eight seconds rather than waiting
indefinitely; an embedding host may add a shorter outer deadline. Shutdown serializes with connection
admission, wakes active connections and rejects subsequent upgrades. The compatibility `http::router` still returns
only a Router; hosts needing coordinated socket shutdown should use the control-returning API.

All limits and delivery are **per backend process**, not distributed across replicas. A single backend process behind
nginx/Node is supported by this transport; multiple independent backends require an additional reviewed event system.

## Handshake and commands

The endpoint is `GET /_brain/socket`. Send exactly one configured `Origin` and exactly one
`Sec-WebSocket-Protocol: brain.v1`. The backend selects that protocol. Every query string is rejected, including
query tokens. Authentication uses the existing native opaque session cookie, native access cookie, or standard Bearer
header; a browser normally uses its HttpOnly native cookie. Invalid Bearer credentials never fall back to a valid
legacy cookie. Duplicate credential headers and combined credential values over 16 KiB are rejected.
Credentials bind at the handshake: after native login, logout or token refresh, close/reconnect the transport so the
browser sends its current HttpOnly cookies. The socket never refreshes tokens or exposes them to JavaScript.

The first message is:

```json
{"type":"ready","protocol":1,"durable":false,"resyncRequired":true}
```

Join or leave an interest using one of these shapes:

```json
{"type":"join","resource":"posts"}
{"type":"join","resource":"posts","id":"00000000-0000-4000-8000-000000000001"}
{"type":"join","room":"user:self"}
{"type":"leave","resource":"posts"}
{"type":"ping"}
```

A successful join returns `{"type":"resync","reason":"joined"}`. Repeated joins are idempotent. Leaving has no
acknowledgement. An application ping returns `{"type":"pong"}`. Unknown fields, arbitrary room names, binary messages
and client publication commands are rejected. A denied resource interest returns
`{"type":"error","code":"ROOM_FORBIDDEN"}`. Room membership never grants access to a record.

A committed native HTTP resource create/update/delete may emit:

```json
{"type":"invalidate","resource":"posts","id":"00000000-0000-4000-8000-000000000001","operation":"update","eventId":"00000000-0000-4000-8000-000000000002"}
```

Only those five fields are emitted. No row data, owner identifiers, emails, tokens or application errors are published.
`eventId` is an ephemeral random event identifier, not a row revision, sequence, history cursor, ETag or CAS token.
Clients must fetch authorized canonical HTTP state after readiness, joining, reconnection or overload.

## Authority and transaction boundary

Native HTTP handlers publish only after their repository transaction commits. Validation failures, failed transactions
and before-hook conflicts do not publish. Update/delete keep the existing checked-write semantics. A deletion carries
only the id and owner from its authoritative row captured before the callback and successful commit; it does not keep
or expose a deleted payload.

Each join checks current authentication and read policy. Each interested delivery reauthenticates the original user,
then checks that specific row's read policy against authoritative ownership. Current policies depend only on role
and ownership, so socket checks select only the owner UUID, never the row JSON. Deletions use the captured authoritative
owner and the current user's role. `user:self` also applies the resource read policy; it cannot bypass a private resource.
Database role changes, disabled accounts and session/JWT revocation are rechecked. Idle sessions reauthenticate on
heartbeat. A revocation racing after a completed authority check is not claimed to be atomic with network delivery.

The transport is ephemeral and best effort. Rapid changes may suppress an older event if its row is already gone.
Direct SQL, direct repository calls and other backend processes do not automatically publish. There is no arbitrary
client publish API, durable history, event replay or exactly-once guarantee. Invalidations ask the client to refetch;
they do not replace policy checks on the subsequent HTTP read.

## Bounds and failure behavior

- 128 connections/process, at most four per user, and 32 interests/connection.
- A shared 128-event ring. Each event has a resource name bounded to 48 ASCII bytes and fixed-size UUID/operation
  fields; its published JSON is at most 256 bytes. Record payloads never enter the ring.
- At most eight concurrent socket authentication/policy database checks. Unadmitted handshakes fail immediately
  at capacity. Each already admitted connection can wait for one check, so the known pending-work bound is 128;
  no task or waiter is created per queued event. One five-second deadline includes admission wait, reauthentication
  and the authoritative owner query. Shutdown cancels a pending or running socket check.
- Frames and assembled messages are at most 4096 bytes. Read buffer target is 4096 bytes; the maximum write buffer is
  8192 bytes. These are protocol budgets, not a universal prediction of allocator or process heap usage.
- Fixed one-second windows allow 16 incoming messages and 32 outgoing JSON messages per connection. Protocol heartbeat
  control frames are separately finite. The send deadline is two seconds; close frames have a one-second deadline.
- Heartbeats run every 25 seconds with an initial phase jitter of 0..5 seconds per user; missing Pongs expire after 55 seconds. Connections have a hard 15-minute lifetime.

Authentication failure closes with 1008. Shutdown closes with 1001. Unavailable authority checks, subscriber lag or
outbound overload close with 1013 and require resynchronization. Oversized frames are rejected by Axum's WebSocket
parser. No security budget can be disabled through a client command.

## Dependencies and verification

The implementation uses existing Axum 0.8.9 with its `ws` feature and Tokio 1.54.1. The native integration client uses
tokio-tungstenite 0.30.0 and futures-util 0.3.34 as development dependencies. These direct versions were verified against
the crates.io registry on 2026-10-30. Axum currently pins tokio-tungstenite 0.29 internally; the test client intentionally
uses the latest stable direct version without overriding Axum's compatible dependency.

Primary API references: [Axum WebSocketUpgrade](https://docs.rs/axum/0.8.9/axum/extract/struct.WebSocketUpgrade.html),
[Tokio watch](https://docs.rs/tokio/1.54.1/tokio/sync/watch/index.html),
[Tokio Semaphore](https://docs.rs/tokio/latest/tokio/sync/struct.Semaphore.html) and
[tokio-tungstenite](https://docs.rs/tokio-tungstenite/0.30.0/tokio_tungstenite/).

Unit and disposable PostgreSQL/loopback TCP regression fixtures are in `src/sockets.rs` and `tests/native_sockets.rs`.
The integration cases are intentionally ignored outside the owned database harness. The repository selector is
`node scripts/backend-check.mjs --only-tests native_sockets`; that harness also retains its existing separate CLI smoke.
For backend-only verification against an explicitly owned disposable database, set `BRAIN_BACKEND_TEST_URL` privately
and run:

```sh
cargo test --locked -p brain-backend --lib --test integration --jobs 2 -- \
  --include-ignored --nocapture --test-threads=2 http::sockets::tests:: native_sockets::
```

On 2026-10-30, all seven unit cases and four real PostgreSQL/TCP cases passed in Linux using the official PostgreSQL
18.6 image on an owned internal Docker network without published ports. The diagnostic compiler was Rust 1.96.1;
this is not a substitute for the release's separate Rust 1.99 gate. Real cases cover opaque/JWT revocation and heartbeat,
owner and role changes, a 2 MiB legacy row and delete authority, rollback, default-disabled HTTP compatibility,
protocol limits, sixteen simultaneous subscribers without reconnection and coordinated shutdown. The database
container and private network were removed. An ignored/skipped case is never evidence of a passing test.
