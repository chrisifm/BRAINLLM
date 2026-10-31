# HTTP caching and conditional requests

Every successful `GET /api/...` response carries an `ETag`. A client that sends it back in `If-None-Match` gets
`304 Not Modified` with no body. Resources that anyone may read can also declare a public cache lifetime, so a CDN or
the browser serves repeat reads without reaching the backend. Writes accept `If-Match` to refuse a lost update.

## Validators

| Response | `ETag` | Meaning |
|---|---|---|
| `GET /api/<resource>/<uuid>` (no `include`) | strong, `"r…"` | this record revision as this reader sees it (id, Postgres row revision, visible fields) |
| Every other successful `GET /api/...` (lists, includes, searches) | weak, `W/"b…"` | digest of the exact response body (bodies up to 2 MiB) |

Responses also send `Vary: Cookie, Authorization, X-API-Key`, and CORS exposes `ETag` to browser code.

```sh
curl -si https://shop.example/api/products/4c2e…
# HTTP/1.1 200 OK
# etag: "r5b1f0c9a2d7e4b6c8a0f1e2d"
curl -si https://shop.example/api/products/4c2e… -H 'If-None-Match: "r5b1f0c9a2d7e4b6c8a0f1e2d"'
# HTTP/1.1 304 Not Modified
```

## Public lifetimes for anonymous reads

```js
// server/cache.jsbrain
<meta server>
export default {
  products: { maxAge: '1m', staleWhileRevalidate: '10m', staleIfError: '1d' },
}
```

Durations are `'30s'`, `'5m'`, `'2h'`, `'1d'` or whole seconds; `maxAge` is required and at most `1d`, the two stale
windows at most `7d`. Each named resource must exist and its `policy.jsbrain` read list must contain `'public'`;
otherwise the backend refuses to start with the file name in the message.

The lifetime applies only to `GET /api/products` and `GET /api/products/<uuid>` requests that carry no cookie, no
`Authorization` and no `X-API-Key`:

```
cache-control: public, max-age=60, stale-while-revalidate=600, stale-if-error=86400
```

Every other response, including the same URL with a session, keeps `cache-control: no-store`. A 304 answer repeats the
declared lifetime. `brain backend schema --json` shows the parsed values under `platform.cache`.

Pick `maxAge` as the longest time a visitor may see an old version after an edit: a shared cache does not learn about
writes. `staleWhileRevalidate` lets the cache answer instantly while it refreshes in the background.

## Conditional writes

`PUT`, `PATCH` and `DELETE /api/<resource>/<uuid>` accept `If-Match` with the strong `ETag` from a record read:

```sh
curl -si -X PATCH https://shop.example/api/products/4c2e… \
  -H 'Origin: https://shop.example' -H 'Cookie: __Host-brain=…' -H 'Content-Type: application/json' \
  -H 'If-Match: "r5b1f0c9a2d7e4b6c8a0f1e2d"' -d '{"price":"14.00"}'
```

If someone else changed the record first:

```json
{ "error": { "code": "PRECONDITION_FAILED", "fix": "The record changed since you read it; GET it again and retry with the new ETag in If-Match" } }
```

with status 412 and nothing written. A weak tag (`W/"…"`) never satisfies `If-Match`; `*` matches any existing
record. The check runs against the same row revision the write's compare-and-swap uses, so two writers holding the
same tag cannot both succeed. Without `If-Match` writes behave as before. Domain actions, transitions and batches do
not read `If-Match`.
