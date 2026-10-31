# Native live resources (development 0.4.1)

`liveResource` adds native invalidation metadata to an existing HTTP resource. It does not replace the resource,
publish client messages, stream row bodies or provide a durable event log. Resource reads remain authoritative.
Applications that do not use it receive no live runtime asset or implicit connection.

## One explicit declaration

```json
{
  "backend": {
    "origins": ["https://app.example.test", "http://localhost:4000"],
    "sockets": {
      "allowedOrigins": ["https://app.example.test", "http://localhost:4000"]
    }
  }
}
```

Both lists contain 1–8 distinct canonical HTTP(S) origins: no path, trailing slash, credentials, query, fragment
or wildcard. Socket origins must be a subset of the resolved HTTP list. Without `backend.origins`, HTTP defaults
to the origin of the selected `PUBLIC_SITE_URL`; a URL path does not become part of the origin. An explicit
`brain backend serve --origin ...` **replaces** the HTTP list; the socket subset must still validate afterwards.
The command never silently unions origins. Omit `backend.sockets`, or set it to `false`, to disable the feature.

Configure the exact development port and each intended deployment deliberately. An automatically selected port
outside the list must fail rather than become another allowed origin. The fixed private `BRAIN_BACKEND_URL` remains
the proxy target; it is never a browser socket URL or a CSP source. This declaration does not create a database,
grant roles, migrate schemas or start another service.

Pack stores a validated boolean `nativeSockets` and the selected canonical `publicOrigin` in the private version-1
server manifest before computing the build signature. A static app receives that metadata without a new `server.mjs`;
it remains eligible for the Rust static server. Absent configuration preserves existing output. Explicit low-level
server overrides may disable the bridge; importing a helper is not server opt-in.

## Attach a resource in its component

```brain
<script>
  import posts from '../api/posts.jsbrain'
  import session from '../channels/session.jsbrain'
  const rows = resource(posts.get)
  const updates = liveResource(rows, {
    rooms: [{ resource: 'posts' }],
    scope: () => session.user ? session.user.id : null
  })
</script>
<p role="status">Live connection: {updates.state}</p>
{#await rows}<p>Loading records.</p>
{:then items}
  {#if items.length}<ul>{#each items as item (item.id)}<li>{item.id}</li>{/each}</ul>
  {:else}<p>No records yet.</p>{/if}
{:catch error}<ErrorState error={error} />{/await}
```

Pass the original resource binding. The compiler passes its handle rather than its awaited `.v` value, just as
`visible(ref)` receives a ref handle. Arbitrary function aliases are not inferred as native calls. `updates.state`
and `updates.connected` are reactive getters, including when read through `drv`; no extra signal kind is introduced.
The new primitive runs inside a component owner. It creates no connection while rendering on the server, in a route
render probe, when WebSocket is unavailable, or when its scope is `null`.

Rooms are an array or a reactive getter returning an array. Each room is exactly `{resource: 'posts'}`, optionally
with a UUID `id`, or exactly `{room: 'user:self'}`. Names and UUIDs are validated. A user-self room never selects an
arbitrary user identifier. Native policy, ownership and authentication remain server decisions.

The scope getter returns `null` or a bounded public user/revision string (160 characters, no whitespace/control
characters). It is a local lifecycle key, never authorization and never transmitted in the URL. Do not use passwords,
cookies or tokens. A principal change discards the previous socket generation. If mounted owners disagree about
their scope, the single shared connection closes; it resumes automatically once their reactive updates converge.

HttpOnly cookie changes are not observable by this helper. After an explicit login/session rotation that keeps the
same public scope, call `updates.reconnect()`; after logout set the public scope to `null`. A public authentication
revision may also be included in the scope. The application must clear its private HTTP/view state when switching
principals; a socket metadata helper cannot erase arbitrary application caches or cancel ignored server-side work.

`updates.close()` stops that subscription. Unmount closes it automatically. The last owner releases the shared socket,
timers, room registrations and callbacks. `reconnect()` affects the shared connection and forces fresh joins.

## Join before resynchronizing

The only endpoint is the current application origin plus `<base>/_brain/socket`, using subprotocol `brain.v1`.
There is no configurable foreign URL, query-token option, publish method or row-state synchronization protocol.
Legacy `store(..., {sync: 'ws'})` and SSE `stream()` retain their separate contracts.

An initial READY frame is not a successful join. Refetch starts after the corresponding ordered JOIN acknowledgement,
and again for matching invalidation metadata. A reconnect rejoins before refetching, covering changes missed while
offline. Metadata is ephemeral, single-backend-process and non-durable; durable work belongs in hooks/jobs.

Callbacks `onInvalidate(metadata)` and `onResync(metadata)` are optional synchronous observers. They receive frozen
allowlisted metadata only. Bursts coalesce; observers are not promised one callback per backend event. No user row,
payload, cookie, credential, arbitrary error property or remote error message is retained in these frames.

Each subscription has one refresh flight and one dirty indication. Refetch calls the existing resource method,
waits one owned post-effect tick, then waits for its current Promise. More metadata produces at most one later refresh.
Callbacks and late Promise completions are guarded by owner/auth/socket generation. A 10-second refresh timeout
does not start another overlapping fetch; a fetcher that ignores cancellation keeps its slot occupied until it
settles. Disposing its owner releases the handle even if an externally retained Promise never settles. Completion
of a resource Promise is not proof that a server SQL query stopped after an HTTP abort.

## Bounds, states and policy

- One connection per loaded application runtime in a tab; at most 128 owners and 32 distinct rooms.
- Frames contain at most 4 KiB of ASCII protocol JSON; unknown fields, binary data and prototype keys fail closed.
- At most 128 event identifiers retained for 60 seconds; metadata flushes coalesce in a 100 ms window.
- Outbound buffered bytes stop at 32 KiB; there is no unbounded application publication queue.
- An 8-second READY deadline, five retries with capped jitter/backoff, a 60-second retry window, and owned heartbeat
  deadlines bound reconnect work. Authentication/policy closes stop retries until an explicit reconnect or scope change.

States include `idle`, `connecting`, `open`, `retrying`, `forbidden`, `auth-required`, `policy-closed`, `scope-conflict`,
`room-limit`, `protocol-error`, `invalid-config`, `exhausted`, `refresh-pending`, `refresh-timeout`, `refresh-error`,
`unsupported` and `closed`. `connected` means an active socket with at least one acknowledged requested room; HTTP
loading/error state still belongs to the resource. A generic policy close is not presented as proof of expired auth.
B0918 explains an invalid handle, ownership, room or scope contract with a static correction; values are not echoed.

Pack adds the exact `ws://` or `wss://` source derived from the selected public origin. It preserves explicit app
connect sources and existing legacy `wss:` policy. Browsers do not universally map CSP `'self'` to WebSocket schemes;
the explicit same-origin expression also covers an enforced development policy. Removing broad legacy sources is
a separate compatibility/security decision, not an implicit effect of enabling live resources.
See [MDN connect-src](https://developer.mozilla.org/en-US/docs/Web/HTTP/Reference/Headers/Content-Security-Policy/connect-src).

Verification covers the runtime with mock transport and forced GC, actual compiled const/let/derived DOM updates,
emitted-asset reachability, configuration bounds, build-signature invalidation and CSP sources. The actual
[compiled native/browser gate](../bench/native-live/README.md) passed six cases with Chromium156, PostgreSQL18.6
and a verified Rust1.99.1 binary: committed CRUD, isolation, reconnect/resync, principal changes, revocation and
owner disposal. It used owned headless contexts because no Viewer profile was assigned. It establishes neither
distributed durability nor a fixed whole-process browser-memory ceiling; broader release gates remain required.

## CLI startup authority

The app dev server resolves `BRAIN_BACKEND_URL` once from the selected environment. Editing `.brain_env` while it
runs may update other keys, but changing or enabling the native authority requires restarting that server. Native
sockets still require the explicit origin declaration above. After binding, the CLI checks the actual numeric
frontend URL against that declaration; if a busy port caused selection of another, unlisted port, it stops the
owned server and reports a configuration fix. Live-enabled dev output reports that validated numeric URL.

`brain test` keeps its ephemeral loopback port when sockets are disabled. With sockets enabled it uses the explicit
development `PUBLIC_SITE_URL` HTTP loopback address and port instead: use `127.0.0.1` or `[::1]`, and declare the same
origin in both lists. An HTTPS production site or a hostname alias is not an implicit local test origin. A busy
port still fails closed unless the actually selected origin was deliberately declared.

For a static production build, `brain start --env <name>` loads the private compiled manifest and the selected
fixed backend authority before writing readiness/control metadata. It serves through the Rust static gateway and
does not create `server.mjs`, copy private variables into browser code, or change global process environment.
