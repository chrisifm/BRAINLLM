# Sign in with Apple and Sign In with LinkedIn

Both providers end in the same native JWT session as [Google Sign-In](JWT-GOOGLE.md): the backend verifies the
provider's ID token, finds or creates the account through `_brain_identities` (keyed by provider and `sub`, never by
email) and sets the same access and refresh cookies. Both require `BRAIN_JWT_SECRET`, issuer and audience; the backend
refuses to start a configured provider without them. Run migrations first: migration 0026 admits the `apple` and
`linkedin` providers, their one-use challenges and the display name a provider shares.

All values below are placeholders. Keep real Services IDs, App IDs, client IDs and secrets in the private server
environment; never commit them.

## Settings

| Variable | Provider | Meaning |
|---|---|---|
| `APPLE_CLIENT_ID` | Apple | The Services ID registered for the website (`com.example.web`) |
| `APPLE_APP_ID` | Apple | Optional primary App ID of that Services ID (`com.example.app`); also accepted as the audience of account notifications |
| `APPLE_REDIRECT_URI` | Apple | The https Return URL registered for the Services ID; defaults to the site origin followed by `/` |
| `BRAIN_AUTH_LINKEDIN_CLIENT_ID` | LinkedIn | The app's Client ID |
| `BRAIN_AUTH_LINKEDIN_CLIENT_SECRET` | LinkedIn | The app's Primary Client Secret, server-only; never sent to a browser or logged |
| `BRAIN_AUTH_LINKEDIN_REDIRECT_URI` | LinkedIn | Defaults to the site origin followed by `/_brain/auth/linkedin/callback`; https, or http on `localhost`/`127.0.0.1` for development, without query or fragment. It may be a page of the app instead: that page forwards `code` and `state` to the callback with the `__Host-brain-linkedin` cookie, and the exchange still uses this exact URL |

LinkedIn needs both the client ID and the secret; one without the other stops startup. `brain check` and `brain doctor`
report the same problems early from the development section of `.brain_env` (B0751).

## Endpoints

| Method and path | Request | Result |
|---|---|---|
| POST `/_brain/auth/apple/challenge` | No body | `{client_id, redirect_uri, nonce, expires_in}` and the HttpOnly `__Host-brain-apple` nonce cookie |
| POST `/_brain/auth/apple` | `{id_token, name?}` plus nonce cookie | Verified Apple identity becomes the native JWT session |
| POST `/_brain/auth/apple/notifications` | `{payload}` from Apple | Applies a verified account notification once; `401` for a payload Apple did not sign |
| GET `/_brain/auth/linkedin/start` | No body | `303` to LinkedIn with a one-use state bound to the `__Host-brain-linkedin` cookie |
| GET `/_brain/auth/linkedin/callback` | `code`, `state` from LinkedIn | Sets the session and redirects `303` to `/`; any refusal or cancelled consent redirects to `/?signin=failed` |

The two challenge and login endpoints are same-origin mutations like Google's. Apple's notification endpoint is the
only one that accepts a POST without the site `Origin`: Apple calls it server to server, and the handler accepts only a
payload signed with Apple's keys. Register `https://<your-site>/_brain/auth/apple/notifications` as the Services ID's
server-to-server notification endpoint.

## Apple

The browser uses Sign in with Apple JS in popup mode: `brain recipe apple-signin` prepares an application-owned adapter
that asks the backend for the challenge, initializes Apple's script with the returned client ID, Return URL and nonce,
and posts the resulting `id_token` once. Closing the popup is a choice, not an error. The form-post flow is not used.

The generated `brain-apple-signin` document-head marker selects `Cross-Origin-Opener-Policy:
same-origin-allow-popups`, as `brain-google-signin` does for Google. The router performs a full document navigation
when entering or leaving this policy: swapping a view cannot change HTTP document headers. Moving between two
popup-enabled pages keeps client navigation. Conditional GET and HEAD responses preserve the same opener policy
on 304 without issuing a new CSP nonce. Application reload shims for this policy transition are unnecessary after
rebuilding with this fix. Real provider sign-in still requires registered credentials and return URLs.

Verification accepts only RS256 tokens signed with a key from `https://appleid.apple.com/auth/keys`, issuer
`https://appleid.apple.com`, the configured Services ID as audience, an unexpired lifetime and the exact raw nonce. The
key, cache and concurrency limits are the same as Google's.

| Address Apple shares | Account created verified | Links an existing account |
|---|---|---|
| Verified address hosted by Apple (`icloud.com`, `me.com`, `mac.com`) | Yes | Yes, under Google's [linking rules](JWT-GOOGLE.md#linking-an-existing-account) |
| Private relay address | Yes | No: the relay reaches the person only through this team |
| Verified address on another domain | No | No |
| No address or unverified | Signs in only a subject that is already linked | No |

Apple sends the user's name only on the first authorization, unsigned, to the browser. The adapter forwards it as `name`;
it is stored as a display hint on the new identity and never used to identify anyone.

Account notifications are verified like ID tokens (audience is the Services ID or App ID), must be issued within seven
days and are applied once by event id. `consent-revoked` ends the account's sessions and keeps the link, so the person
can sign in again; `account-deleted` disables the Apple identity and ends the sessions; email events are recorded only.

Not handled: exchanging Apple's authorization code, revoking Apple tokens on account deletion, and `transfer_sub` team
transfers. An Apple-only account always needs an address, because native accounts require one.

## Profile hints returned to the app

After `brain backend migrate` applies migration 0027, native session users include optional `name` and `picture`
when the verified Google or LinkedIn ID token shares them. Apple supplies only its first-authorization name through
the adapter; it supplies no profile photo. The initial sign-in JSON, `/_brain/auth/me`, refreshed sessions and native
SSR `ctx.user` keep these hints. LinkedIn's redirect callback sets the session and redirects; read `me` or `ctx.user`
after that redirect rather than expecting a JSON callback body.

Names are plain text, trimmed and capped at 200 characters. Picture URLs must be credential-free HTTPS and at most
2048 bytes. Missing or invalid hints do not deny sign-in or erase previously shared hints; a later valid value updates
them. Migration 0027 preserves names already stored with identities. These hints never establish identity, verify an
email, choose a role or enter access-token claims. The framework does not fetch or copy the image, retain provider
access tokens or call `userinfo`. Treat the URL as provider-managed and handle image load failure; no lifetime is
promised. Apps that need a durable avatar can independently store an authorized copy under their own media policy.

## LinkedIn

The Node gateway relays the native start's 303 only for GET/HEAD on `/_brain/auth/linkedin/start` and the exact
HTTPS destination `https://www.linkedin.com/oauth/v2/authorization`, without credentials or a fragment. Other
paths, methods, statuses and destinations retain same-site redirect validation. Google and Apple use token/challenge
endpoints, not this redirect exception; arbitrary OIDC authorization redirects are not enabled by it.

LinkedIn uses the OpenID Connect authorization-code flow of a confidential web client: `/start` redirects the browser
to `https://www.linkedin.com/oauth/v2/authorization` with scope `openid profile email`, a one-use state and a nonce
derived from it. The callback compares the state with its cookie in constant time, exchanges the code at
`https://www.linkedin.com/oauth/v2/accessToken` with the client secret and the exact redirect URL, keeps only the ID
token (the access token is discarded) and verifies it against `https://www.linkedin.com/oauth/openid/jwks`, the
configured client ID, LinkedIn's issuer and the nonce when LinkedIn echoes it. Register the callback URL exactly in the
LinkedIn app, one entry per environment (for example `http://localhost:5190/_brain/auth/linkedin/callback` and
`https://example.com/_brain/auth/linkedin/callback`).

LinkedIn does not host any mailbox, so its address never links an existing account and never marks a new one verified.
A person who already has a password account signs in with it; a new address creates an unverified account. The callback
GET relies on the SameSite=Lax state cookie, which browsers send on LinkedIn's top-level redirect back to the site.

## Verification

Unit tests build disposable synthetic RSA keys in memory and check audience, issuer, nonce, expiry, subject and the
email rules for both providers, Apple notifications as an object or a JSON string, and that the LinkedIn authorization
URL never carries the client secret. Real sign-in needs provider accounts and registered URLs; synthetic tests do not
claim an external login happened.

Sources: [Sign in with Apple JS](https://developer.apple.com/documentation/signinwithapple/configuring-your-webpage-for-sign-in-with-apple),
[Apple ID token verification](https://developer.apple.com/documentation/signinwithapple/verifying-a-user),
[Apple server-to-server notifications](https://developer.apple.com/documentation/signinwithapple/processing-changes-for-sign-in-with-apple-accounts),
[Sign In with LinkedIn using OpenID Connect](https://learn.microsoft.com/en-us/linkedin/consumer/integrations/self-serve/sign-in-with-linkedin-v2).
