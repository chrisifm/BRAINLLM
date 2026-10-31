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
actual image identities, binary digest, PostgreSQL version and browser version. The binary digest uses a bounded
stream rather than retaining an entire debug executable. `--browser` selects an explicit absolute executable
inside the owned container; the default remains `/usr/bin/chromium`.
The image's Rust version does not establish which compiler built a supplied binary.
The source pin comes from the caller's verified artifact provenance. The report separately fingerprints the
fixture tree and each gate script, so an updated test does not masquerade as the supplied binary source.
Fixture discovery rejects symlinks and bounds file count, individual size and aggregate bytes.

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
6. Revoked JWT sessions close on delivery. Unmounting a healthy child component closes its socket without navigating or
   replacing the document, and prevents reconnection after disposal.

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

## Verified acceptance and limits

All six cases passed with the actual compiled fixture, Chromium
`Chrome/156.0.8078.73` and disposable PostgreSQL 18.6. The verified binary came
from source `d0c7daffd7470a896897d0fbce440f9e5fa326d4`, built with Rust 1.99.1;
its SHA-256 is
`c08686d7a2ba3f8bd96bb152fdd5acee738f90dc5f3095f9525ffd60a649d978`.
Compiler input provenance is recorded separately from fixture and gate-script
hashes. The passing nine-file fixture hash is
`cdcaebe0b8fe9bb5bb53edc93beda59c52fd443ca7358487af46dac5b22ea169`.

The run observed 15 allowlisted metadata frames. All three browser contexts and
owned children were disposed, no app database connections remained, and the
owned containers/network were confirmed removed. Viewer fallback was explicit;
no user profile or existing tab was selected.

The acceptance container combines the backend, frontend and three Chrome
contexts. Its finite PID limit is 512: the observed peak was 262, with zero PID
limit hits and zero OOM kills. An earlier 256-PID run reached exactly 256, recorded
two limit hits and an OS thread-spawn error (errno 11). CPU, memory budgets,
authorization and all six assertions remained unchanged. Recovery from OS thread-admission failure remains a separate reliability
acceptance requirement; this successful run with headroom does not establish
resource-exhaustion recovery. The database retains its separate 96-process limit.

The first binary rejected the documented endpoint `cache: 'no'` as B1102, and
the dev module fallback concealed the diagnostic behind a component export.
Those defects were corrected before this acceptance run. The fixture retains the
explicit no-cache endpoint contract and uses real compiled SDK methods throughout.
This gate is a single owned-process deployment proof, not distributed durability,
an application migration or a replacement for the framework's broader release
gates and actual deployment verification.
