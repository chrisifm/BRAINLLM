# Project API keys

A project can issue its own API keys so other services, scripts and partners call the resource API without a browser
session. Keys are opt-in, scoped, quota-limited and fully traced. They are stored as a SHA-256 of a 244-bit random
secret (never the plaintext) and compared in constant time.

## Enable

Create `server/api-keys.jsbrain` (a literal configuration; every field except `enabled` is optional):

```js
export default {
  enabled: true,
  dailyQuota: 1000,        // default requests per UTC day for keys without their own quota (omit = unlimited)
  monthlyQuota: 20000,     // default requests per UTC month (omit = unlimited)
  retentionDays: 90,       // request trace kept this many days (1..3650, default 90)
  perKeyPerMinute: 600     // burst limit per key, on top of the per-IP limiter (default 600)
}
```

Then `brain backend migrate` (creates `_brain_api_keys`, `_brain_api_key_usage`, `_brain_api_key_events`) and
`brain backend serve`. Without the file (or with `enabled: false`) any presented key is rejected with
`401 API_KEYS_DISABLED` and the `/_brain/keys` routes answer 404. The generated OpenAPI document gains the
`apiKey` (`X-API-Key` header) and `apiKeyBearer` (`Authorization: Bearer`) security schemes on every `/api` operation,
and `brain backend schema --json` reports the effective settings under `apiKeys` (null when disabled).

## Key format and scopes

```
bk_live_<prefix>_<secret>        prefix = 12 lowercase hex (public, shown in lists and logs)
                                 secret = 64 lowercase hex (shown ONCE at creation or rotation)
```

Scopes are `resource:action`: `resource` is a resource name or `*`; `action` is `read` (GET/HEAD), `write`
(POST/PUT/PATCH/DELETE) or `*`. Examples: `posts:read`, `orders:write`, `*:read`, `posts:*`.

A key always acts as its owner. The owner's current role is loaded on every request and the resource policy still
applies, so a scope can only narrow what the owner may do: a `*:*` key of a `candidate` user still gets
`403 POLICY_DENIED` on an admin-only resource. Disabling or deleting the owner disables or deletes their keys.

## Use

Keys are accepted only on resource routes (`/api/<resource>` and `/api/<resource>/<id>`). They are a non-ambient
credential, so these requests need no `Origin` header and cookies are ignored. Every other route (sign-in, sessions,
key management, uploads, jobs) rejects a presented key with `401 API_KEY_NOT_ACCEPTED`.

```bash
curl -s https://api.example.com/api/posts?limit=2 \
  -H 'X-API-Key: bk_live_3f9a1c0b7d2e_9c1e...64-hex...'
# or: -H 'Authorization: Bearer bk_live_3f9a1c0b7d2e_9c1e...'
```

```http
HTTP/1.1 200 OK
x-quota-limit: 1000
x-quota-remaining: 997
x-quota-reset: 41211

{"items":[{"id":"6a1d...","owner":"0b7e...","data":{"title":"Hello"}}],"limit":2,"offset":0}
```

| Status | `error.code` | Meaning |
|---|---|---|
| 401 | `API_KEY_INVALID` | Malformed key, unknown prefix or wrong secret |
| 401 | `API_KEY_REVOKED` | Revoked, or replaced by a rotation |
| 401 | `API_KEY_EXPIRED` | Past `expiresAt` |
| 401 | `API_KEY_NOT_ACCEPTED` | Key sent to a session-only route |
| 401 | `API_KEYS_DISABLED` | The project did not enable keys |
| 403 | `API_KEY_SCOPE_DENIED` | No `resource:action` scope for this request |
| 403 | `POLICY_DENIED` | Scope allows it but the owner's role does not |
| 429 | `API_KEY_RATE_LIMITED` | Per-key per-minute burst limit; `Retry-After: 1` |
| 429 | `API_KEY_QUOTA_EXHAUSTED` | Daily or monthly quota used; `Retry-After` = seconds to reset |

## Quotas

Each key may declare `dailyQuota` and `monthlyQuota`; otherwise the project defaults apply. Periods are UTC calendar
days and months. Counters live in Postgres and are consumed with one atomic upsert per period, so they hold across
restarts and multiple processes. Every key-authenticated response carries `X-Quota-Limit`, `X-Quota-Remaining` and
`X-Quota-Reset` (seconds) for the tightest quota; keys with no quota omit the headers. `lastUsedAt` is updated at most
once a minute.

## Provisioning (signed-in session)

These routes require a session cookie (or native JWT) and, for writes, the configured `Origin`. Admins act on any key;
everyone else only on their own. A user may hold at most 50 active keys.

```bash
curl -s -X POST https://api.example.com/_brain/keys -b '__Host-brain=<session>' \
  -H 'Origin: https://app.example.com' -H 'Content-Type: application/json' \
  -d '{"name":"Reporting job","scopes":["posts:read","orders:read"],"expiresInDays":365,"dailyQuota":5000}'
```

```json
{"key":{"id":"7c2f...","userId":"0b7e...","name":"Reporting job","prefix":"3f9a1c0b7d2e",
        "scopes":["orders:read","posts:read"],"dailyQuota":5000,"monthlyQuota":null,
        "createdAt":"2026-10-31T06:00:00+00:00","expiresAt":"2027-10-31T06:00:00+00:00",
        "lastUsedAt":null,"revokedAt":null,"rotatedFrom":null,"usedToday":0,"usedThisMonth":0},
 "secret":"bk_live_3f9a1c0b7d2e_9c1e...64-hex..."}
```

| Route | Purpose |
|---|---|
| `POST /_brain/keys` | Create; body `name`, `scopes`, optional `expiresInDays` (1..3650), `dailyQuota`, `monthlyQuota` |
| `GET /_brain/keys` | Your keys with usage (`usedToday`, `usedThisMonth`, `lastUsedAt`); admins add `?all=true` |
| `POST /_brain/keys/:id/rotate` | New secret with the same name, scopes, quotas and remaining lifetime; the old key is revoked atomically |
| `DELETE /_brain/keys/:id` | Revoke |
| `GET /_brain/keys/:id/events` | Request trace (below) |
| `GET /_brain/keys/:id/summary?days=7` | Requests per UTC day by resource, action and status, plus `droppedEvents` |

## Request trace

Every key-authenticated request (including scope, quota, revoked and expired denials) appends one row to
`_brain_api_key_events`: time, request id, method, route template and concrete path, resource and action, status,
latency in ms, client IP (peer address), user agent (first 200 characters), bytes out and the **names** of query
parameters. Secrets, `Authorization` headers, bodies and query values are never recorded.

Writes leave the request path through a bounded in-memory channel and a batched insert; when the channel is full the
event is counted in `droppedEvents` instead of blocking the request. Rows older than `retentionDays` are purged on
boot and then daily.

```bash
curl -s 'https://api.example.com/_brain/keys/7c2f.../events?status=200&resource=posts&limit=2' -b '__Host-brain=<session>'
```

```json
{"items":[{"id":812,"at":"2026-10-31T06:01:02+00:00","rid":"5b0e...","method":"GET","route":"/api/{name}",
           "path":"/api/posts","resource":"posts","action":"read","status":200,"latencyMs":4,
           "clientIp":"203.0.113.7","userAgent":"report-job/1.0","bytesOut":512,"queryKeys":["limit"]}],
 "next":811}
```

Filters: `from`, `to` (ISO-8601), `status`, `resource`, `limit` (1..200). Pagination is keyset: pass `before=<next>`.

## Operator CLI

```bash
brain backend keys create --user <uuid> --name "Nightly export" --scopes posts:read --daily 2000 [--expires-days 90] [--monthly N]
brain backend keys list [--user <uuid>]
brain backend keys rotate 3f9a1c0b7d2e
brain backend keys revoke 3f9a1c0b7d2e
brain backend keys events 3f9a1c0b7d2e --status 429 --limit 20
brain backend keys summary 3f9a1c0b7d2e --days 30
```

Each command accepts a key id or its public prefix, uses `DATABASE_URL`, and prints JSON. `create` and `rotate`
print the secret once.
