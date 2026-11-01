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

The page sends the widget's token in the `X-Brain-Captcha` header. `protect` takes `signup`, `login`, `password_request` and
`route:<name>` for a declared public, non-GET route.

| Situation | Answer |
|---|---|
| No token, rejected token, or a hostname other than `hostname` | 403 `CAPTCHA_FAILED` |
| Provider unreachable, an HTTP error, unreadable JSON, or a fault with our secret | 503 `CAPTCHA_UNAVAILABLE` |

It fails closed: nothing runs without an explicit `success: true`. Tokens are single use, so verification never retries.
Production refuses to start without the secret or with a non-https `verify_url`; `http` is accepted on loopback only, for a local fake.
Provider-specific checks (action names, scores) are not read.
