# Compiled native live-resource browser gate

This opt-in acceptance runner owns a disposable PostgreSQL 18 container, a private
Docker network, a compiled Brain CLI dev gateway/backend and headless Chromium.
It never reads an application database URL from the host environment. There are
no published host ports, external email/provider calls, personal browser profiles,
lasting services or runtime dependencies added to the framework.

Run it with an explicitly verified Linux binary and its actual source revision:

```sh
node bench/native-live/run.mjs \
  --binary /private/artifacts/brain-linux \
  --source-sha ACTUAL_BINARY_SOURCE_SHA \
  --expect-sha ACTUAL_BINARY_SHA256 \
  --scratch /private/disposable-directory \
  --out /private/evidence/native-live.json
```

The owned compiler/browser image defaults to
`brainllm-next-verify:rust-1.99.1-owned`. The caller provisions that image and the
official `postgres:18` image before running the gate. The runner records their
actual image identities, binary digest, PostgreSQL version and browser version.
The image's Rust version does not establish which compiler built a supplied binary.
The source pin comes from the caller's verified artifact provenance.

The fixture schema migration affects only a generated `brain_live_*` database
inside that owned container. It is **not** a migration of Vacantes or any other
application, and it does not read, authorize or perform such a migration.

The bounded CDP metadata oracle can be checked without Docker or services:

```sh
node --test bench/native-live/frame-metadata.test.mjs
```

## Acceptance cases

The fixture imports real compiled `api/*.jsbrain` clients. Its resource handle is
passed to `liveResource` through the compiler, not replaced by a mock or direct
handwritten browser fetch. Real JWT login supplies HttpOnly native cookies to
three owned Chrome contexts; test accounts and signing/database secrets exist
only for the disposable run.

The gate fails fast and counts only completed cases:

1. The compiled HTTP resource joins through the CLI's fixed dev gateway and
   displays only its authenticated owner's rows.
2. Anonymous HTTP and WebSocket Upgrade disclose no private row body.
3. Committed create/update/delete refresh the actual DOM. Another owner receives
   neither the row body nor its invalidation identifier.
4. A fresh join resynchronizes a mutation missed while the scope was disabled;
   explicit reconnect creates a fresh socket generation.
5. A principal switch clears private view state and closes the previous generation.
6. Revoked JWT sessions close on delivery. Navigating a healthy owner away closes
   its socket and prevents reconnection after disposal.

Protocol frames are retained in a bounded allowlisted form; row fields and secret
values are prohibited. Reports contain status, known error categories and source
coordinates, not passwords, cookies, JWTs, row values, arbitrary exception messages
or environment values. Temporary credential files are private and removed. Final
cleanup verifies child/context disposal, zero remaining app database connections
and removal of the owned containers/network.

Viewer is preferred when an explicit isolated assignment is available. The initial
acceptance environment had multiple connected Viewer profiles without an assigned
profile or tab; none was selected or commandeered. This runner records the explicit
owned-headless fallback and preserves all user browser sessions.

## Current evidence boundary

The first pinned binary (`3987d0b`, SHA-256
`c57c36e234c736dbcf2a2f13b4d3b5d993da4f7f7cf0d46ad63765906e3e56b0`)
completed synthetic schema migration, HTTP signup/login and fixture-row creation;
three actual Chromium contexts loaded the compiled view successfully. No acceptance
case is claimed complete yet: B1102 incorrectly rejected the documented endpoint
`cache: 'no'`, and dev module fallback hid that diagnostic behind a component export.
The fixture deliberately retains `cache: 'no'`. Checker correction `f2287f6`
is available; the explicit module-error behavior has a separate owner. Rerun the complete gate with a fresh
verified binary after those corrections before claiming full-chain acceptance.
