# Native sockets through the Rust frontend

The Rust development server and static production gateway can forward the native
`/_brain/socket` endpoint to a fixed loopback backend. The gateway transports opaque
bytes after validating both HTTP upgrade handshakes; it does not parse application
WebSocket frames or authorize subscriptions. The backend remains responsible for
origin, session, room, resource and row authorization, including revocation checks.

## Development embedding

`start_dev_with_native_proxy(options, compile, lint, diagnose, Some(proxy))` accepts
`NativeProxy::new("http://127.0.0.1:8000")?.sockets(true)`. Existing `start_dev` and
`NativeProxy` without `.sockets(true)` keep sockets disabled. The optional
`.socket_origins(vec![public_origin])?` adds an early frontend-origin check; it never
expands the backend allowlist. CLI configuration must validate the actual local
frontend origin against the explicitly configured origins.

This explicit backend URL fixes the same authority for HTTP, SSR and sockets.
Other environment values and environment reload behavior remain intact. Recreate
the server to change backend authority. A later environment change cannot move
HTTP to another backend while keeping established sockets on the old backend.

## Static production, including PM2 without nginx

`prod::serve_static` reads `BRAIN_BACKEND_URL` once and uses a managed
`prod::StaticGateway`. Embedders can call
`StaticGateway::load(dist, base, backend_url)` directly and share it with `Arc`.
The existing `Static::load`, `handle` and `handle_with_backend` APIs remain available;
these legacy unmanaged handlers do not enable sockets.

The compiled private `_server/manifest.json` must explicitly contain:

```json
{"v":1,"nativeSockets":true,"publicOrigin":"https://app.example.test","routes":[],"rpc":{}}
```

The manifest is bounded to 64 KiB and must be a regular file below the output
root. Invalid JSON, duplicate activation fields, wrong activation types, unsupported
active versions, missing backend URL and invalid origins fail startup. An absent
manifest or activation flag leaves sockets disabled. The public origin is an exact
canonical HTTP(S) origin, separate from the application's base path. No Node
process is needed for a static frontend with this gateway. SSR deployments retain
their Node bridge and its native-cookie handling.

## Protocol and limits

The endpoint is under the configured base, for example `/app/_brain/socket`.
Requests require GET, HTTP/1.1, version 13, exact `brain.v1`, a valid 16-byte key,
canonical Origin and no query string, including an empty `?`. Only native access
and opaque-session cookies or a standard Bearer credential are forwarded;
refresh and unrelated cookies, forwarded headers and supplied request IDs are not.
The backend URL must be a literal loopback HTTP address with no path or credentials.

Limits apply to each gateway instance, not a distributed deployment:

- 32 admitted tunnels, including pending handshakes; excess receives 503.
- One owned Tokio current-thread worker; bounded admission queue and task set.
- 32 KiB handshake response, 8 KiB read-ahead per side, two 16 KiB transport buffers.
- Five seconds total handshake time, two seconds per blocked write and 15-minute
  maximum tunnel lifetime. Half-close preserves the remaining response direction.
- At most 64 retained structured diagnostics; omitted count is explicit. Diagnostics
  contain phase, code, request ID, source location and fix, never credentials,
  upstream bodies or raw error messages.

`Dev.stop`, `StaticGateway::stop` and the last gateway owner cancel active tunnels
and reap the owned worker. Ordinary HTTP and HMR use separate handlers. Reconnect
is the client's responsibility; this proxy supplies no durable history, arbitrary
publish interface or automatic authentication refresh.

## Evidence

The focused suite uses actual loopback TCP sockets, real development sidecars and
static gateways. It covers binary/read-ahead forwarding, rejected handshakes,
upstream authentication failure, half-close, handshake and blocked-write deadlines,
32-connection admission, base isolation, ordinary HTTP/HMR, fixed authority,
environment preservation and stop/last-owner disposal. This is transport evidence;
backend authorization has its own PostgreSQL and WebSocket integration suite.

The complete `brain-serve --lib` suite passed 64/64 tests with zero ignored tests
on the owned Linux Rust 1.96.1 preflight. The final toolchain and wider framework
gates remain separate integration checks.
