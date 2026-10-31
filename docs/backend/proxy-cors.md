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
- send mutations; other origins get `403 ORIGIN_DENIED`.

`cors.maxAge` lets browsers cache preflight results, which removes one round trip from every cross-origin mutation:

```sh
curl -si -X OPTIONS http://127.0.0.1:5182/api/orders \
  -H 'Origin: https://admin.shop.example' -H 'Access-Control-Request-Method: POST'
# access-control-allow-origin: https://admin.shop.example
# access-control-allow-credentials: true
# access-control-max-age: 600
```

## Tested example

`crates/brain-backend/tests/edge.rs` runs this configuration against Postgres: one forwarded client exhausts its own
rate-limit bucket, another client behind the same proxy keeps its own, a forged left-most hop does not create a new
bucket, and the extra origin passes preflight with `max-age` and may send mutations.

## HTTPS pinning

In production every answer carries `Strict-Transport-Security: max-age=31536000; includeSubDomains`, so
browsers keep using HTTPS for the API host and its subdomains for a year. Serve the backend only behind
TLS in production; development answers do not send the header.
