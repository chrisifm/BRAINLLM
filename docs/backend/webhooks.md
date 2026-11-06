# Signed webhook routes

Payment and shipping providers call your backend and sign the exact request bytes. Declare a route with
`verify`, and the backend checks the signature before the handler runs:

```js
// server/routes/payment_events/route.jsbrain
<meta server>
export default {
  method: 'POST',
  path: '/payments/events',
  auth: 'public',
  body: 'raw',
  bodyLimit: '256kb',
  rate: '120/min',
  verify: {
    scheme: 'timestamped',              // header value 't=<unix seconds>,v1=<hex hmac>'
    header: 'x-payment-signature',
    secret: 'PAYMENTS_WEBHOOK_SECRET',  // environment variable NAME, never the value
    tolerance: '5m',                    // default 5m, 1s..1h
  },
}
```

```js
// server/routes/payment_events/handler.jsbrain
<meta server>
export default async function handle(ctx) {
  const event = JSON.parse(ctx.rawBody)
  if (event.type === 'payment.succeeded') {
    // mark the order paid; make this idempotent by event.id, providers retry
  }
  return { received: true }
}
```

## Schemes

| Scheme | Header value | Signed message |
|---|---|---|
| `timestamped` | `t=1767225600,v1=5f2b…` (several `v1` allowed) | `<t>.<raw body>` |
| `hmac` (default) | `sha256=5f2b…` with `prefix: 'sha256='`, or a bare digest | `<raw body>` |
| `custom` | Application-declared provider headers | Exact bytes verified by a private `verify.jsbrain` |

The two built-in schemes use HMAC-SHA256 with a constant-time comparison. `hmac` takes `encoding: 'hex'` (default) or
`'base64'`. `timestamped` rejects a timestamp older or newer than `tolerance`.

## Built-in HMAC secrets

`secret` names an environment variable (`.brain_env`, or the host's secret store). Pass a list of up to four
names to rotate: `secret: ['PAYMENTS_WEBHOOK_SECRET', 'PAYMENTS_WEBHOOK_SECRET_PREVIOUS']`. Each secret
must be at least 16 bytes. An absent secret disables the affected route: it answers `503 ROUTE_DISABLED`
and never runs its handler, while the server can continue serving other routes. Set the secret and restart.
A present but too-short secret, or an invalid literal secret in the contract, refuses startup.

## Certificate and other provider protocols

For certificate signatures or another protocol the built-in HMAC schemes do not implement, use
`body: 'raw', auth: 'public', verify: { scheme: 'custom' }`. Add a private sibling `verify.jsbrain`
whose default export verifies the request and resolves to the literal boolean `true` before the
handler can run. See [provider-specific signature verification](routes.md#provider-specific-signature-verification)
for its context, contract and failure behavior.

Declare the provider headers in the route's `headers` list. Duplicate declared header values are
rejected; the verifier must reject missing headers required by its protocol. It receives the exact
`rawBody` and `rawBodyEncoding`, declared headers and selected environment values, without visitor
identity or an injected backend client. Route/job backend API keys are removed from its environment;
other selected server secrets remain available, so the verifier is trusted application code.
Custom verifiers read their selected secrets from `ctx.env`; the
built-in HMAC `secret` option does not implement a custom protocol.

The application's adapter must validate the actual signature over the exact bytes, timestamp and
trusted certificate/key chain. Pin permitted key endpoints, bound key fetches and persist replay
protection when required. A caller-supplied certificate URL is not trusted merely because it can be
fetched. Neither returning `true` unconditionally nor decoding an unsigned payload authenticates an
event. BRAINLLM supplies the verification boundary, not a built-in certificate-provider adapter.

## Failures

| Status | Code | Meaning |
|---|---|---|
| `401` | `SIGNATURE_INVALID` | Header missing, malformed or not matching the body |
| `401` | `SIGNATURE_EXPIRED` | `timestamped` signature outside `tolerance` |
| `413` | `BODY_TOO_LARGE` | Body above `bodyLimit` |
| `429` | `RATE_LIMITED` | Above `rate` |
| `503` | `ROUTE_DISABLED` | Built-in signing secret absent; handler never runs |
| `503` | `VERIFIER_UNAVAILABLE` | Custom verifier failed |
| `504` | `ROUTE_TIMEOUT` | Shared custom-verification and handler deadline expired |

Successfully verified public routes replace the browser `Origin` requirement with signature verification.
Built-in HMAC routes use POST, PUT or PATCH; custom raw public routes also support DELETE. Ordinary
unsigned public writes still require Origin validation, including when `body: 'raw'` is declared.
Do not add a synthetic Origin or disable that protection to make a provider callback pass.

The handler receives the bytes as signed in `ctx.rawBody` (`ctx.rawBodyEncoding` is `utf8` or `base64`),
the parsed `ctx.body` when the payload is JSON, and `ctx.verified.timestamp` for the timestamped scheme.

The built-in outbound/inbound webhook endpoints under `/_brain/webhooks` keep their own scheme; signed
routes are for providers that define the URL and signature format themselves.
