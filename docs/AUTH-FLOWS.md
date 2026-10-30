# Native account recovery, verification and roles

The Rust backend owns account state. Recovery changes an existing account password; it does not create an account,
link a Google identity, issue a session or silently sign the user in. Email verification applies to the currently
authenticated user and its current email. A GET never consumes a token. Browser mutations use the backend's existing
origin checks, request/body limits and rate limiting.

| POST route | JSON input | Result |
|---|---|---|
| `/_brain/auth/password/request` | `{email}` | Generic accepted response when delivery is configured |
| `/_brain/auth/password/reset` | `{token,password}` | Changes the password once, or rejects an invalid/expired link |
| `/_brain/auth/email/request` | `{}` | Generic acceptance for the signed-in account |
| `/_brain/auth/email/verify` | `{token}` | Verifies the current signed-in account once |
| `/_brain/auth/users/{id}/role` | `{role}` | Administrator-only assignment from the server's configured roles |

Request bodies reject unknown fields. Signup and Google sign-in use only the server's configured default role;
signup credentials never accept a client-supplied role. Passwords retain Argon2id and the existing bounded hashing
workers and 12..1024-byte password limits. Recovery links do not appear in HTTP responses, private diagnostics,
database plaintext columns or generic job payloads.

## Default sign-in page

New scaffolds use the server-owned `ctx.backend.login(credentials)` facade from their existing server form/RPC.
It forwards a fixed native login request, relays only native HttpOnly cookies and returns the sanitized canonical
user. The Rust `/_brain/auth/login` endpoint selects JWT cookies when the backend has JWT configured, or a native
opaque session otherwise. Authentication or JWT errors never trigger another mode. The explicit token endpoint
remains available to clients that require JWT. The frontend does not need the backend's signing key to choose a mode.
The page navigates to a validated local `next` path or `/account`. No access/refresh token is rendered or copied
into the channel store. That store contains only an ephemeral display hint; backend policies and guards always use
the verified server user.

A native endpoint failure never invokes the demo provider, including when a client changes page props. The same
server form supports JavaScript and progressive no-JS submission through the fixed native authority. The legacy
development demo remains available only when the framework marks the Node runtime as development and native
authority is absent. A production runtime cannot enable that demo merely by receiving `DEMO_PASSWORD`.

Existing applications keep their authored provider and pages until a reviewed migration changes them. Updating the
compiler alone does not rewrite an old demo login into a native provider. Inspect `server/auth.jsbrain` and
`views/login.brain`, or prepare the separate `auth-ui` recipe before choosing a route migration. Preserve custom
provider behavior and verify both the native authority boundary and any required no-JS behavior explicitly.

## Delivery configuration

The native delivery adapter uses the official [Resend send-email API](https://resend.com/docs/api-reference/emails/send-email)
through the existing maintained `reqwest` dependency, without adding a new HTTP stack or handwritten SMTP transport.
It sends plain text, with a unique idempotency reference and a fixed HTTPS endpoint. Provide a privately stored API
key and a verified plain sender address. Use a key restricted to the required sending domain when provisioning the
provider. Configure credentials according to the provider's [API-key guidance](https://resend.com/docs/dashboard/api-keys/introduction).
The framework does not create a provider account, edit DNS, send a real test email or claim deliverability automatically.

For `brain backend serve`, keep `BRAIN_MAIL_RESEND_KEY` and `BRAIN_MAIL_FROM` in the private environment and set
`PUBLIC_SITE_URL` to the frontend's public base URL, including its deployment prefix. This URL is independent of
the backend's fixed loopback listen address. The URL's origin must also be in the explicit frontend origin allowlist.
No credential value is needed in source or command arguments. An unset mail key disables email delivery instead of
selecting a development token-return mode.

The public base URL includes any frontend deployment prefix (for example `https://app.example.test/project/app`).
HTTPS is required except for explicit HTTP loopback development. User information, query/fragment, controls,
backslashes, empty internal segments and literal/encoded dot segments are rejected before URL normalization.
Links append the recovery/verification route to this prefix; they never discard it through an absolute URL join.
CORS origins remain the separate scheme/host/port allowlist.

The server API is `MailConfig::resend(key, from, public_base_url)`. `delivery()` returns the bounded adapter;
`Flows::new(pool, auth, roles, public_base_url, delivery)` composes it with native account state. Secrets are never
serialized by these types. The mail target is fixed and does not use proxy environment variables or follow redirects.
Each response is bounded to 16 KiB and must acknowledge a message identifier. A provider acknowledgement means
accepted by the provider, not read by the recipient. Provider-side message retention remains an operator policy.

Without delivery configuration, all recovery requests receive the same service-unavailable classification, and a
private structured diagnostic explains the missing configuration. The server never simulates an email or exposes
the token as a development shortcut. With delivery configured, known, unknown, disabled, already-verified and throttled
accounts receive the same generic acceptance result. Delivery errors are recorded as static private diagnostics;
the HTTP response never reports which account exists or whether its email was sent.

Lookup, challenge preparation, provider call and activation share one absolute two-second work deadline. All accepted
requests hold one of four slots through a uniform 2100 ms completion floor; unknown accounts do not return early.
There is no pending work queue. Cancellation drops owned asynchronous work. This removes the explicit lookup/send
timing difference; it is not a claim that a shared operating system or network has mathematically constant timing.
The global HTTP and persistent per-address limits still apply. A digest permits at most three sends per 15 minutes,
with bounded cleanup and a 10,000-row retained challenge/limit budget.

## Challenge state and revocation

Migration `0005_auth_flows.sql` adds email/version bindings and delivery activation to the original hashed token
table. Existing unbound tokens remain unusable. Reset links expire after 30 minutes, verification links after 24
hours. Both use 64-character opaque random values; only SHA-256 digests are stored. Only the latest requested token
per account and purpose remains eligible. Expired, consumed, unbound and unconfirmed hashes cannot authorize a mutation.

Sending occurs before activation. A timeout or lost provider response is not retried: its pending hash remains
inactive even if the provider eventually delivered an email. Users can request a new link. No token plaintext is
queued durably, so no extra encryption key or plaintext outbox is introduced. A later worker-backed delivery design
would require its own reviewed authenticated-encryption and key-rotation contract.

Successful reset locks the user and rechecks token purpose, expiry, email, authentication version and single-use
state inside one transaction. It changes the password, increments `auth_version`, revokes all opaque/JWT sessions
and refresh families, consumes all outstanding account tokens and records a value-free audit event. Existing
access tokens and refresh tokens cannot survive the reset. Login checks the authentication version again after
password verification, so an in-flight old-password login cannot race a reset into a new session.

Verification requires the same account identity as the token binding and the same current email/version. It marks
that account verified and consumes the token transactionally; a token from another account or another purpose cannot
be substituted. Opening or previewing an email cannot verify an account. Templates place tokens in URL fragments,
remove the fragment on mount and retain the value only in that page's memory. They send the token only in an explicit
same-origin mutation body. Never put challenge links in analytics, error messages or source fixtures with real users.

## Server-owned roles and first administrator

`RolesConfig` includes `admin`, `company`, `candidate` and `user`, with at most 32 distinct role names total.
Custom names use lowercase ASCII identifiers of 1..48 characters. `public` and `owner` remain policy keywords,
not assignable account roles. The default signup role remains `candidate` for compatibility; a server operator can
select another configured non-admin role. `RolesConfig::register` registers only explicit server configuration;
it never removes historical roles or silently reassigns existing users.

Declare a literal server-owned configuration in `server/auth/roles.jsbrain`, for example:

```js
export default {
  roles: ['admin', 'company', 'candidate', 'user', 'editor'],
  signup_role: 'candidate'
}
```

Run the explicit `brain backend migrate` for that environment to register configured roles. Serving checks that
they were provisioned; it does not add roles or run migrations during startup. Resource policies may reference
only configured names plus the reserved policy keywords. Removing a role from configuration does not delete its
database users or grant it to newly registered accounts.

The versioned migration replaces the old three-role CHECK with a registered-role foreign key. Baseline adoption
accepts that replacement only after validating the complete immutable migration ledger and checksum, required
columns, keys and built-in roles. An unknown role or missing migration does not degrade to unrestricted signup.

Role assignment reloads the actor's current database role, serializes concurrent changes, validates the target role
against server configuration and revokes the target's existing sessions and challenge tokens after changing its
authentication version. A stale client role cannot authorize an assignment. The last enabled administrator cannot
demote itself through this operation. Direct operator database access remains outside the HTTP policy boundary.

The private operator API `bootstrap_first_admin(pool, existing_user_id)` promotes an explicitly selected existing
account only while no enabled administrator exists, then revokes its old sessions and records the bootstrap event.
It is never a public HTTP route or an automatic first-signup rule. Operator tooling must obtain the selected account
through private input; passwords and tokens do not belong in command arguments, shell history or public examples.
The corresponding operator command is `brain backend bootstrap-admin --user <existing-user-uuid>` with the explicit
environment selection. Use an account you control and selected deliberately, then sign in again after its old
sessions were revoked. No first public signup is promoted automatically.

## Verification

`tests/auth_flows.rs` uses a fresh database inside the explicitly owned disposable Postgres harness and an in-memory
synthetic delivery adapter. It exercises concurrent single-use reset, opaque/JWT/refresh revocation, email/version
binding, expiry, user-bound verification, registration-role injection rejection, current-database administrator
checks, custom roles, first-admin protection, unconfirmed delivery and the equal response floor. No real user
database or email provider is contacted by these tests. Real deployment delivery requires the operator's verified
domain and credentials; synthetic tests do not prove inbox delivery.

`tests/auth_http.rs` also runs the real Axum router on an owned loopback TCP listener. Its synthetic activated
hashes isolate handler behavior from email delivery: origin and account checks, single-use responses, cleared
cookies, opaque/JWT/refresh revocation, custom-role authorization and the live OpenAPI enum. The CLI harness
creates two accounts through its actual server, explicitly bootstraps one administrator, rejects a second bootstrap,
checks old-session revocation and assigns a custom role through HTTP. Test credentials stay in request bodies;
no password or token enters command arguments. The compiled default-login regression executes both native login
modes against a fetch fixture, verifies safe navigation and no demo fallback, and rejects persistent auth storage.
