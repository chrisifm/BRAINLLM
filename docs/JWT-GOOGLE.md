# Native JWT sessions and Google Sign-In

The experimental Rust backend supports short access JWTs and rotating opaque refresh cookies alongside its existing opaque session endpoint. Google Identity Services (GIS) establishes the same Rust session. The Node SSR signed-cookie demo remains independent by default; configure the optional [native SSR authority bridge](NATIVE-AUTH-SSR.md) to share the real Rust identity. Do not infer authorization from a frontend button or a decoded browser JWT. Resource policies run against the backend's authoritative user on every request.

## Private configuration

Set the following in the selected private `.brain_env` environment or the process environment:

```text
BRAIN_JWT_SECRET=<at least 32 cryptographically random bytes; private>
BRAIN_JWT_ISSUER=https://your-site.example
BRAIN_JWT_AUDIENCE=https://your-site.example
GOOGLE_CLIENT_ID=<configured web OAuth client id>.apps.googleusercontent.com
```

The signing secret is server-only. The Google web client ID is public and is returned by the nonce endpoint; no Google client secret is used for this ID-token flow. Configure the exact browser origin in Google Cloud's authorized JavaScript origins and the backend origin allowlist. Google sign-in refuses to start without JWT configuration. Existing unconfigured applications keep their opaque session behavior.

Run reviewed database migrations first. Migration 0003 adds revocation/version fields, Google identities, refresh families/tokens and one-use challenges. It does not link existing accounts by email or silently migrate their authentication method.

## Endpoints

All mutation endpoints require the configured site `Origin`, including native clients. Browser requests use same-origin credentials behind the site's reverse proxy. Cookies are HttpOnly, Secure, SameSite=Lax and use the `__Host-` prefix with Path=/; use HTTPS. All responses are no-store and failures expose generic codes/fixes and a request ID, never credentials, JWTs or private exception traces.

| Method and path | Request | Result |
|---|---|---|
| POST `/_brain/auth/token` | `{email,password}` | User, access token, Bearer type and 300-second lifetime; access and refresh HttpOnly cookies |
| POST `/_brain/auth/refresh` | Refresh cookie | Rotated refresh cookie and new short access token |
| POST `/_brain/auth/logout` | Current auth cookies or Bearer access | Revokes the current native session/family, clears auth cookies and sends existing Clear-Site-Data storage cleanup |
| GET `/_brain/auth/me` | Access cookie/Bearer or existing opaque cookie | Authoritative current user |
| POST `/_brain/auth/google/challenge` | No body | Public configured client ID, a browser-bound nonce and five-minute lifetime |
| POST `/_brain/auth/google` | `{credential}` plus nonce cookie | Verified Google identity becomes the same native JWT session |

JWT access uses jsonwebtoken's HS256 implementation with a fixed algorithm, explicit issuer/audience, required exp/nbf/sub, a session ID, auth_version, issued-at and unique token ID. Access lifetime is five minutes. Every access request checks the live session, expiration, revocation, disabled state, auth_version and current role in Postgres. Incrementing auth_version invalidates access and refresh credentials. JWT payloads are signed, not encrypted; no password or secret is included.

Refresh tokens have 244 random bits from two UUIDv4 values. Only SHA-256 digests are stored. Rotation consumes the old token and inserts one child in a transaction serialized by user. Reusing an old refresh token revokes the entire family and its session, invalidating its access tokens immediately. Refresh across tabs must be coordinated: concurrent reuse intentionally fails closed. Family expiration is absolute, at most seven days, and rotation does not extend it. Limits are ten live sessions per user, 1,000 retained refresh families per user and 4,096 token records per family. Expired evidence is cleaned in bounded batches; hitting a token-chain limit requires fresh sign-in.

## Google verification and identity policy

Google tokens must use RS256, one configured audience, an accepted Google issuer, an unexpired lifetime, an acceptable issued-at, the exact browser nonce and a verified email. Multi-audience tokens require the configured authorized party. The stable identity key is Google's `sub`, never email. An existing local email account cannot be linked by this flow; sign in using its existing method. New identities receive only the candidate role. The local verified flag is enabled automatically only when Google is authoritative for the address: Gmail or a verified Google Workspace hosted-domain claim. A third-party address can authenticate its stable Google subject without being marked locally verified.

Only `https://www.googleapis.com/oauth2/v3/certs` supplies verification keys. Token-provided key URLs are rejected. TLS remains verified; redirects and environment proxies are disabled. Public JWKS caching respects bounded max-age; one set contains at most sixteen keys and 64 KiB. Unknown kids cannot cause more than one refresh per thirty seconds. Verification has two concurrent slots, a three-second HTTP deadline, two-second connect timeout and bounded cache-lock waiting. Stale keys are not accepted after their cache lifetime.

The GIS JavaScript callback flow uses the application's explicit Origin check plus a HttpOnly browser-bound nonce, which is signed into Google's ID token and consumed once in Postgres. It is not the GIS automatic form-post integration; that separate integration would additionally require Google's g_csrf_token double-submit validation. It is not silently enabled here.

## Frontend recipe

`brain recipe google-signin --json` prepares a reviewable three-file plan: a sized client-only view, a small `.jsbrain` adapter import and an application-owned GIS DOM adapter. The normal atomic change protocol applies the plan. The recipe refuses to overwrite any of its files. It provides checking, anonymous, authenticated, busy, generic error, retry and logout states.

The adapter fetches the configured public client ID and nonce from the backend, loads only Google's official GIS script, and renders Google's native button. It posts the credential once, stores no tokens in localStorage/IndexedDB, logs no credentials, cancels owned requests on unmount and drops late callbacks. An expired access session attempts one refresh POST before preparing a new Google challenge. Sign-in/logout perform an explicit full page reload at the authentication boundary to forget prior user-specific in-memory state; immutable static assets retain their browser cache. Only one adapter instance is active at a time.

Expose native API paths on the frontend origin using the production Node server with `BRAIN_BACKEND_URL` configured, or an explicit reviewed reverse proxy. A separate backend port alone does not establish this topology. The example uses origin-root API paths and an optional `/account-access` email-sign-in view from `auth-ui`.

The recipe emits an explicit document-head `brain-google-signin` opt-in. Builds add the exact official GIS CSP sources and page-scoped opener policy for static/Node documents; other pages retain their opener isolation. The CSP remains one site policy to support client navigation. Do not disable CSP wholesale. The adapter never generates server secrets or provisions Google Cloud credentials.

## Verification

Unit tests create disposable synthetic RSA material in memory using Node's native crypto, validate positive RS256/JWKS cases and reject wrong signature/algorithm, issuer, audience, authorized party, nonce, expiry and unknown key IDs without contacting Google. Disposable Postgres tests cover refresh rotation/reuse revocation, auth_version/disabled state, one-use Google nonces and absence of implicit email linking. Frontend tests use an SDK stub and verify credential single-submit, cleanup, session reload and safe public status messages.

Actual Google sign-in requires an operator-configured OAuth web client and HTTPS deployment; synthetic tests do not claim an external account login happened. Keep app-author SPEC claims planned until the compiled recipe and backend gates pass.

Sources checked against current official documentation: [jsonwebtoken 11.1.0](https://docs.rs/jsonwebtoken/latest/jsonwebtoken/), [reqwest 0.13.5](https://docs.rs/reqwest/latest/reqwest/), [Google ID-token verification](https://developers.google.com/identity/gsi/web/guides/verify-google-id-token), and [GIS setup/CSP](https://developers.google.com/identity/gsi/web/guides/client-library).
