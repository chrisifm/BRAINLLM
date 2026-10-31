# Proxies and CORS

The native backend binds to loopback and is meant to sit behind a reverse proxy such as nginx. Two settings in
`server/platform.jsbrain` make that setup correct: which proxies are trusted to report the client address, and which
extra browser origins may call the API.

```js
<meta server>
export default {
  // Proxies whose X-Forwarded-For hops are trusted (IP or CIDR). Empty or missing: the TCP peer is the client.
  trustProxy: ['127.0.0.1', '::1'],
  cors: {
    // Origins allowed in addition to the site origin (PUBLIC_SITE_URL / --origin).
    origins: ['https://admin.shop.example'],
    // Access-Control-Max-Age for preflight responses, in seconds (0..86400).
    maxAge: 600,
  },
}
```

`brain backend schema` shows the parsed values under `platform.edge`. Unknown keys, malformed ranges and catch-all
ranges (`0.0.0.0/0`, `::/0`) fail at startup with the file name in the message.

## Client address

Behind nginx every request reaches the backend from `127.0.0.1`. Without `trustProxy` the rate limiter therefore puts
every visitor in the same bucket. With `trustProxy` the backend resolves the real client:

1. If the TCP peer is not in `trustProxy`, the peer is the client and `X-Forwarded-For` is ignored.
2. Otherwise the `X-Forwarded-For` hops are read from right to left, skipping trusted proxies. The first untrusted
   address is the client.
3. Hops to the left of that address were written by the client and are never used, so a client cannot choose its own
   rate-limit key by sending `X-Forwarded-For: 1.2.3.4`.
4. An unreadable hop stops the walk at the last trusted address. If every hop is trusted, the left-most one is used.

IPv4-mapped IPv6 addresses (`::ffff:127.0.0.1`) are compared as IPv4. A hop list has no hop-count setting on purpose:
counting hops trusts whatever sits at that position, while a range list trusts only machines you control.

The resolved address is available to native code as the `brain_backend::http::ClientIp` request extension.

### nginx

```nginx
location / {
  proxy_pass http://127.0.0.1:5182;
  proxy_set_header Host $host;
  # Appends the connecting address to whatever the client sent.
  proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
}
```

With this configuration and `trustProxy: ['127.0.0.1']`, a request from `203.0.113.9` that sends a forged
`X-Forwarded-For: 1.1.1.1` arrives as `1.1.1.1, 203.0.113.9` and is keyed as `203.0.113.9`.

## CORS

The site origin is always allowed. `cors.origins` adds more origins, each an explicit `http(s)` origin without a path,
credentials or wildcard. Every allowed origin may:

- read responses with credentials (`Access-Control-Allow-Credentials: true`),
- send mutations; other origins get `403 ORIGIN_DENIED` with `brainCode: "B0740"`.

`cors.maxAge` lets browsers cache preflight results, which removes one round trip from every cross-origin mutation:

```sh
curl -si -X OPTIONS http://127.0.0.1:5182/api/orders \
  -H 'Origin: https://admin.shop.example' -H 'Access-Control-Request-Method: POST'
# access-control-allow-origin: https://admin.shop.example
# access-control-allow-credentials: true
# access-control-max-age: 600
```

## Troubleshooting by code

`brain explain <code>` prints the rule. `brain check` and `brain doctor` catch B0744, B0745 and B0746 before anything runs.

| Code | You see | Where the detail is | Fix |
|---|---|---|---|
| B0740 | `403 ORIGIN_DENIED`, or a browser CORS error on a read or preflight | Server log `origin-refused`: seen origin, allowed origins, key. Development answers add `error.origin`. Production answers carry only the codes and a generic fix | Add the exact origin to `cors.origins` in `server/platform.jsbrain` or `backend.origins` in `brain.json`, or call from the site itself |
| B0741 | `403 Cross-origin request`, or the cross-origin form page | Server log `origin-refused`: seen origin, host, forwarded host. Production adds only the code (`brainCode`, or the `x-brain-code` header on the form page) | Call from the page this server serves. Behind a proxy, forward the host (`proxy_set_header Host $host;`). There is no allowlist to widen |
| B0742 | `ctx.backend` refuses the call | Server log `origin-refused`; the message starts with the code | Run the login, logout or write from a same-origin POST |
| B0743 | The browser reports only that the socket closed | Backend log and the dev proxy diagnostics: seen origin, allowed origins. The dev console prints the likely fix. The handshake answer stays opaque | Add the exact origin to `backend.sockets.allowedOrigins` and `backend.origins` |
| B0744 | `brain check`, or the backend does not start | The message names the value, the key and the exact replacement | Write `scheme://host[:port]` as a browser sends it: no trailing slash, path, credentials or wildcard |
| B0745 | `brain check`, the build or the backend does not start | The message names the origin and both lists | Make the socket list a subset of the HTTP origins, and list the frontend being built |
| B0746 | `brain check` warning (`http:`, `ws:`) or info (`https:`, `wss:`) | `file:line` of the literal call | Use a relative path through your own server, or `https`, or declare the origin |
| B0747 | The dev console, for an opaque `TypeError` in `fetch` | Target and page origins, and whether the server answered | Add the page origin to `cors.origins` on that backend, or call it from a server function |
| B0748 | `brain backend serve` does not start | The message names the URL, what it resolves to and the `--origin` values | Pass `--origin` for that frontend, add it to `backend.origins`, or set `PUBLIC_SITE_URL` to an allowed origin |

No path ever adds a wildcard, widens the list on its own or reflects an unlisted origin. Messages drop any credentials
in a value and any query string. Refused requests log at most 16 lines per minute. Trailing slashes, uppercase hosts and
default ports in `cors.origins` still start, because they only never match; the backend logs
`origin-declaration-ignored` and `brain check` warns.

## Tested example

`crates/brain-backend/tests/edge.rs` runs this configuration against Postgres: one forwarded client exhausts its own
rate-limit bucket, another client behind the same proxy keeps its own, a forged left-most hop does not create a new
bucket, and the extra origin passes preflight with `max-age` and may send mutations. It also proves that a refused origin gets
full detail in development and no allowlist in production.

## HTTPS pinning

In production every answer carries `Strict-Transport-Security: max-age=31536000; includeSubDomains`, so
browsers keep using HTTPS for the API host and its subdomains for a year. Serve the backend only behind
TLS in production; development answers do not send the header.
