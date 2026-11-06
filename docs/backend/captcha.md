# Captcha on public writes

Requires a verified token before a protected public write runs. Any provider with the common `siteverify` contract works:
a form POST of `secret`, `response` and `remoteip`, answered with JSON holding `success` and `hostname`.

```js
// server/auth/captcha.jsbrain
<meta server>
export default {
  verify_url: 'https://challenges.cloudflare.com/turnstile/v0/siteverify',
  secret_env: 'CAPTCHA_SECRET',
  hostname: 'app.example.com',
  protect: ['signup', 'login', 'password_request', 'route:contact']
}
```

| Provider | `verify_url` (docs read 2026-10-31) |
|---|---|
| Cloudflare Turnstile | `https://challenges.cloudflare.com/turnstile/v0/siteverify` |
| hCaptcha | `https://api.hcaptcha.com/siteverify` |
| reCAPTCHA (classic siteverify; Google marks that page deprecated) | `https://www.google.com/recaptcha/api/siteverify` |

When this file exists the build opens the Content-Security-Policy to the provider named by `verify_url` (script, frame and
what its widget needs), and to nothing else. Dev does the same.

`brain recipe captcha` prepares a project-owned form and adapter with loading, ready, verified, expired and error states,
retry and a reset after every submit, for Turnstile, hCaptcha and reCAPTCHA. Replace the test site key it starts with.

The page sends the widget's token in the `X-Brain-Captcha` header. `protect` takes `signup`, `login`, `password_request` and
`route:<name>` for a declared public, non-GET route.

| Situation | Answer |
|---|---|
| No token, rejected token, or a hostname other than `hostname` | 403 `CAPTCHA_FAILED` |
| Provider unreachable, an HTTP error, unreadable JSON, or a fault with our secret | 503 `CAPTCHA_UNAVAILABLE`, and a `captcha-unavailable` log line whose `reason` names which (`secret-not-set`, `provider-unreachable`, `provider-status`, `provider-answer-unreadable`, `provider-missing-secret`, `provider-invalid-secret`, `provider-sitekey-secret-mismatch`, `provider-internal-error`) |

It fails closed: nothing runs without an explicit `success: true`. Tokens are single use, so verification never retries.
Production refuses to start without the secret or with a non-https `verify_url`; `http` is accepted on loopback only, for a local fake.
Known public provider test secrets remain local-only; pass, fail and already-spent Turnstile fixtures are refused in
production. `brain check`/`brain doctor` warn B0790 if the production environment resolves to a public fixture, without
printing its value. Production also refuses a known public test site key proven in a reachable widget binding,
even with a non-test secret, before opening a database connection. Literal values, direct public environment reads
and unshadowed immutable module constants can be resolved. SDK options with uncertain overwrites, unresolved
conditions, external JavaScript and cross-module aliases remain unknown. Discovery is bounded; this is not a browser
or provider verification. A provider-named public test site-key setting without a proven binding is advisory and does
not refuse startup. Library hosts supplied only the server contract cannot infer browser site keys. Put dummy keys
under development/testing and provider-issued site and secret keys under production.
Provider-specific checks (action names, scores) are not read.

`hostname` is optional. Set it when one site uses the key; omit it for a widget shared by several hosts (production and a
preview) and for test keys, whose verification hostname is controlled by the provider and may differ from the local
host. Do not copy a sample response's hostname into the test configuration. See the provider's
[official Turnstile testing guidance](https://developers.cloudflare.com/turnstile/troubleshooting/testing/).

## Signup, then login

A token is good once, so a login right after a signup would need a second challenge. When both `signup` and `login` are
protected, a signup that passed its challenge also answers `login_grant`. Send it as `X-Brain-Login-Grant` on that account's
login, in place of a token. It lasts five minutes, works once, only for that account, and does not replace the password: a
wrong password spends it, and a grant that does not fit answers 403 `CAPTCHA_FAILED`. Without it the login needs its own token.

## From a server function or a BFF

The token comes from the browser, so pass it along. `ctx.backend.login(values, { captcha })` and
`ctx.backend.signup(values, { captcha })` send it; `ctx.backend.signup` keeps the grant for a `ctx.backend.login` of the same
address in the same request, and never shows it to the page. Native-proxied calls forward both headers. A BFF forwards
`X-Brain-Captcha` and `X-Brain-Login-Grant` itself, and the provider sees the client address only if the BFF is a trusted proxy.

## Batch signups

A script has no widget. Declare `operator_key_env: 'CAPTCHA_OPERATOR_KEY'` and send that value as `X-Brain-Operator-Key` on
`POST /_brain/auth/signup`. It opens signup only, earns no login grant, leaves an `operator-signup` audit event, and is
never forwarded by the site's own proxies or allowed by CORS, so keep it on the server side. There is no bypass unless you
declare it: a wrong key answers 403 without asking the provider, and a declared key that is unset admits nobody. Production
refuses to start with a key under 32 bytes, or with a provider's always-pass test secret.

## Test keys

Docs read 2026-11-01. They work on `localhost` and `127.0.0.1`; never ship them.

| Provider | Site key | Secret | Result |
|---|---|---|---|
| Turnstile | `1x00000000000000000000AA` | `1x0000000000000000000000000000000AA` | passes |
| Turnstile | `2x00000000000000000000AB` | `2x0000000000000000000000000000000AA` | fails |
| Turnstile | `3x00000000000000000000FF` | `3x0000000000000000000000000000000AA` | interactive; the secret says already spent |
| hCaptcha | `10000000-ffff-ffff-ffff-000000000001` | `0x0000000000000000000000000000000000000000` | passes |
| reCAPTCHA v2 | `6LeIxAcTAAAAAJcZVRqyHh71UMIEGNQ_MXjiZKhI` | `6LeIxAcTAAAAAGG-vFI1TnRWxMZNFuojJ4WifJWe` | passes |
