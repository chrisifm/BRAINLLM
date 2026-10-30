# Native webhooks

The Rust backend accepts signed inbound work and explicitly enqueues outbound deliveries. Run the explicit backend migration before serving; migration 6 adds encrypted events and bounded delivery records. Declaring a webhook does not subscribe it to resource mutations. Existing resource after-hooks still enqueue their declared jobs in the mutation transaction.

## Private configuration

Declare a literal `server/webhooks/config.jsbrain`:

```js
export default {
  encryptionKeys: ['BRAIN_WEBHOOK_ENCRYPTION_KEY'],
  inbound: {
    orders: { queue: 'order_received', signingKeys: ['BRAIN_WEBHOOK_ORDERS_KEY'] }
  },
  outbound: {
    partner: {
      urlEnv: 'BRAIN_WEBHOOK_PARTNER_URL',
      signingKey: 'BRAIN_WEBHOOK_PARTNER_KEY',
      allowedOrigins: ['https://partner.example.test']
    }
  }
}
```

`order_received` must already declare a native job schema and handler. Resolve the named variables through the private server environment. Keys are independent cryptographically random 32-byte values encoded as 64 hex characters. Encryption and signing keys must differ; inbound endpoints must use distinct signing keys. URLs belong in the private environment because their paths or queries can contain credentials. Metadata includes approved origins, never complete URLs or keys.

Each key list accepts the current key followed by one previous key for rotation. Keep the previous encryption key until all retained ciphertext using it has expired. Removing it early makes those events unreadable. New events use the first encryption key. Outbound signing has one current key; coordinate receiver rotation before replacing it.

## Signed inbound requests

Send `POST /_brain/webhooks/in/orders`, `Content-Type: application/json`, with an object of at most 16 KiB and these headers:

- `X-Brain-Webhook-ID`: canonical lowercase UUID, stable across retries.
- `X-Brain-Webhook-Timestamp`: canonical decimal Unix seconds, within five minutes of the receiver clock.
- `X-Brain-Webhook-Signature`: `v1=` followed by the hexadecimal HMAC-SHA256.

Sign the exact raw body bytes after this UTF-8 prefix (each `\n` is one newline):

```text
brain-webhook-v1\n{event-id}\n{timestamp}\n
```

The signing frame includes no trailing separator after the body. Sign a fresh timestamp for delayed retries while preserving the event UUID and semantic JSON payload. JSON is parsed and checked against the declared job schema only after signature verification. This single provider route does not require a browser Origin or a cookie; other mutations retain their normal Origin and authorization checks. Duplicate signature headers fail closed.

A successful request returns HTTP 202 with `{id,eventId,duplicate}`. An existing endpoint/event UUID with the same canonical payload returns its original receipt without creating a second job. A changed payload returns 409. Invalid signatures/timestamps return 403. No response returns a secret or the stored payload. The job contains an opaque reference; only the live leased worker can decrypt its associated event before invoking the handler.

## Explicit outbound delivery

An authenticated administrator with an allowed browser Origin may send:

```http
POST /_brain/webhooks/out/partner
Content-Type: application/json

{"eventId":"550e8400-e29b-41d4-a716-446655440000","payload":{"kind":"order_received"}}
```

The backend returns a 202 receipt after committing the encrypted event and durable job together. `GET /_brain/webhooks/out/partner/{receipt-id}` requires an administrator and returns state, attempt counts and bounded status/code/timing records. It does not return the body, URL, headers or provider response. Payload objects must fit the same 16 KiB/depth-32 budget.

The backend owns two outbound workers when outbound configuration exists. Each event has at most five attempts, using the queue's exponential retry delay, a five-second delivery deadline and a stable `Idempotency-Key`. Receivers must implement idempotency: delivery is at least once, including the case where a receiver commits but its response is lost. HTTP 2xx succeeds; redirects and other statuses do not. Worker shutdown is bounded and cancels owned requests.

Destinations require exact approved HTTPS origins. Every delivery resolves at most 16 addresses, rejects the entire answer if any address is private/reserved, and pins the validated addresses into that connection. Redirects, environment proxies and automatic transport retries are disabled. Response bodies are drained only within 16 KiB and the deadline. No external delivery is attempted by the synthetic test suite.

## Retention, limits and hooks

Event bodies use authenticated AES-256-GCM with random nonces and job/endpoint/purpose-bound associated data. A database uniqueness constraint rejects nonce reuse for the same key. Job rows contain only opaque event references; attempt rows contain static outcome codes, HTTP status and duration. Keep database access and encryption keys private: encryption does not replace database authorization or key rotation.

The shared event store caps retained events at 10,000. Queue retention removes only completed/dead work older than 30 days in bounded batches, cascading its event and attempt records. Run the explicit existing queue-prune operation as part of maintenance. Replay protection lasts while the original event is retained; after retention expires, a reused event ID can be accepted again. Application handlers must remain idempotent independently of this window.

Native mandatory before-hooks are described in [BEFORE-HOOKS.md](BEFORE-HOOKS.md). They run for direct native CRUD as well as service calls, with single-record conflict detection. They do not make cross-record business validation atomic.

Cryptography uses the existing Rust `ring` dependency, version 0.17.14, verified against its current [HMAC documentation](https://docs.rs/ring/latest/ring/hmac/index.html) and [AEAD documentation](https://docs.rs/ring/latest/ring/aead/index.html).

## Verification

Use `node scripts/backend-check.mjs` for the disposable PostgreSQL suite, fresh CLI, queue and authentication smoke tests. The harness never reads a real `DATABASE_URL` or sends mail/webhooks externally. The library contains cryptographic, address and bounded-input tests plus an ignored disposable-database test proving replay handling, encrypted storage, live-lease decryption and five finite delivery attempts through a private synthetic sender. Real loopback HTTP coverage exercises signed provider requests and administrator publish/status handlers.

Backend integration cases live in their existing source modules and run through the single `integration` Cargo target. For a focused run without a database, use `cargo test --locked -p brain-backend --test integration native_openapi:: -- --test-threads=2`. `--only-tests auth_http,native_openapi` on the disposable harness filters module names explicitly; it does not imply the omitted modules passed. Default harness execution includes all modules and the library. Combining the modules preserves the original 13 integration cases; the webhook OpenAPI and mandatory-hook HTTP regressions add two cases, for 15 total. No standalone integration target also runs them by default.
