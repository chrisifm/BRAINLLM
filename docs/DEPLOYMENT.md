# Optional deployment previews

`deployment::preview(root, target, options)` prepares revision-checked public configuration source for pm2 or nginx. It does not install software, acquire certificates, migrate a database, publish a service, run a command or restart anything. The native production path remains brain build followed by brain start; PM2 is an optional host integration.

Options include topology (static/node/rust-node/rust-static), name, frontend port, backendPort, memoryMb, brainBinary, domain, certificate, certificateKey and expectedRevision. Ports remain loopback upstreams. Names, DNS names and certificate paths are validated; PM2 values are JSON-serialized literal values, never interpolated shell commands. Configuration operations target deploy/ecosystem.brain.config.cjs or deploy/nginx.brain.conf. An existing file is preserved and returned as a separate proposed comparison, with no replacement operation.

## Explicit topology

- Static uses the native Rust static handler through brain start. Inspect the build artifact: the topology label does not disable or remove a server runtime from an existing dist directory.
- Node uses brain start in the foreground, with its existing Node SSR supervisor when dist/server.mjs exists.
- Rust-node adds a separate loopback resource backend with PostgreSQL and an exact HTTPS origin. Schema migration remains separate. The frontend server receives a fixed loopback BRAIN_BACKEND_URL in its host environment; this is not a browser PUBLIC_* setting. Its bounded native proxy forwards API and short-link requests, and SSR resolves ctx.user through the native session verifier. Credentials remain outside generated configuration.

- Rust-static pairs the native Rust static frontend with the same resource backend. Use a static release without dist/server.mjs. The fixed loopback proxy supports PM2 directly; nginx can route native endpoints directly to the backend.

Each PM2 service uses one fork instance. Node SSR frontends use one supervised Node child; static frontends do not require that child. Queue workers can own handler children up to their explicit concurrency cap. Adding replicas requires explicit shared-state, rate-limit, session and database concurrency review. The [PM2 ecosystem configuration](https://pm2.keymetrics.io/docs/usage/application-declaration/) sets bounded restart backoff, restart limits and a memory restart threshold. The native supervisor retains its child RSS checks and graceful reload behavior. Memory restart thresholds are polling policies, not hard whole-tree memory quotas. PM2 wait_ready is false: the native supervisor's stdout readiness protocol is not a PM2 process-message handshake.

The [PM2 shutdown guide](https://pm2.keymetrics.io/docs/usage/signals-clean-restart/) explains signal delivery and kill timeouts. Generated configuration allows twenty seconds for the foreground manager to stop and its Node children to drain. Validate shutdown and resource behavior on the actual host; the preview is not runtime measurement evidence. Production environment key names are listed without reading or returning values. Supply credentials through the host's private environment and existing secret configuration.

## HTTPS reverse proxy

The nginx preview requires explicit existing certificate and private-key paths. It emits HTTP-to-HTTPS redirect and TLS server blocks for inclusion in nginx's http context. The [official proxy directives](https://nginx.org/en/docs/http/ngx_http_proxy_module.html) configure exact loopback upstreams, bounded connect/read/send timeouts, no automatic request replay and no proxy cache. The proxy overwrites host, scheme and client-IP headers rather than appending untrusted incoming identity. Do not add an unreviewed intermediate proxy hop or expose the upstream listeners publicly.

Native backend topologies route /api, /_brain, /s/ and /health to the backend; other requests reach the frontend. No development WebSocket/HMR forwarding is enabled. App statuses, CSP, content negotiation, Vary and versioned asset Cache-Control headers stay upstream-owned. No blanket immutable policy applies to HTML, API responses or session data. The upstream already serves precompressed assets where supported. Optional nginx Brotli directives are omitted until its installed module and actual precompressed assets are verified; [gzip is also an explicit nginx module](https://nginx.org/en/docs/http/ngx_http_gzip_module.html), not an assumed server capability. The preview does not globally enable dynamic compression of authenticated responses.

Review the source, apply its CAS operations, build and inspect the release, install reviewed production dependencies, configure private keys, and run nginx -t or parse the PM2 configuration before explicit activation through the verified host manager. Then measure HTTPS health, graceful drain, resource limits and cache behavior. This module does not choose a permanent local alias, expose a preview publicly or replace existing custom service configuration.

## Durable queue processes

The optional workers array contains explicit `{ "queue": "thumbnails", "concurrency": 2 }` contracts. PM2 receives one separately supervised foreground `brain jobs work` process per queue, with the production environment and the same graceful timeout/restart policy. At most eight queue names, eight concurrent handlers per queue and sixteen in total are allowed. Queue names are native identifiers; duplicates or shell syntax are rejected. Create/implement the queue contract and explicitly migrate the selected database before activation. Credentials remain private host environment values. nginx supervises no process; pair its routing configuration with the corresponding PM2 worker configuration.

Native image ingestion has a thirty-second processing budget. Generated nginx read timeouts allow thirty-five seconds. The Node and Rust frontend proxies use thirty-five-second deadlines specifically for `/_brain/uploads` and its children; ordinary native API/media metadata routes retain ten-second deadlines. This does not introduce automatic retries or an unbounded pending queue.


## Checkpoint limits and verification

Native authenticated socket rooms and production WebSocket Upgrade tunnels are deferred beyond this checkpoint.
The generated nginx configuration deliberately clears Connection and contains no Upgrade forwarding rule. Existing
browser transport helpers do not provision an authenticated native socket server. PM2 supervision does not add one.

Queue leases/retries require idempotent handlers; interrupted work may be delivered again. The preview does not
promise exactly-once external effects, distributed live-event delivery, whole-tree memory quotas or zero-downtime
behavior on an untested host. Keep one backend instance until shared concurrency and failure behavior are reviewed.

Local acceptance checked generated PM2 configurations for all four topologies with an installed PM2 config parser,
without launching its daemon. Generated nginx configurations for those topologies passed `nginx -t` in an owned
container using temporary test certificates. These checks establish parsing and syntax, not live HTTPS forwarding,
certificate issuance, host restart behavior or authenticated browser journeys. Repeat them for the final release
configuration and the actual host before activation.
