# Explicit startup data warmup

Production already imports its server modules before readiness. An optional `server.warmup` additionally performs anonymous cached SSR reads before the public listener becomes ready:

```json
{
  "server": {
    "warmup": {
      "paths": ["/catalog"],
      "timeoutMs": 8000,
      "onError": "continue"
    }
  }
}
```

The example requires an existing exact public `/catalog` SSR route with positive cache seconds and a configured real site origin. Without `server.warmup`, no data requests or warmup worker are created. Static output rejects the option; it never gains a server automatically.

## Eligible reads

Declare one to eight unique canonical paths, at most 256 UTF-8 bytes each, including the app base and locale prefix when present. Paths must name exact public cached SSR routes: no dynamic parameters, catchalls, guards, middleware or forms. Reserved API/internal/health routes are excluded. Query strings, fragments, credentials, traversal, encoded separators, repeated slashes and external URLs are rejected. Use the canonical trailing slash for the configured policy.

Set `site` or `PUBLIC_SITE_URL` to the public HTTP(S) origin. Placeholder domains are rejected for this operational opt-in. Warmup opens a temporary loopback listener and makes at most two concurrent GETs with that public Host header and identity encoding. It sends no cookies or Authorization. Only its tracked loopback connections can invoke the warmup dispatcher; it closes the listener before readiness.

The warmup context rejects framework write operations and privileged requests. Application code must still implement genuinely read-only public loads: arbitrary authored side effects cannot be made safe by a GET method. Responses must be HTTP 200 without Set-Cookie, private/no-store/no-cache or content encoding. Each body is capped at 256 KiB and all bodies at 2 MiB. Warmup is not an external crawl or an authentication mechanism.

## Deadline, failure and shutdown

`timeoutMs` is an absolute monotonic budget of 100–10000 ms, default 8000, including module imports, data reads, cancellation and cleanup. It is independent of the request/RPC timeout. A watchdog in an owned worker can terminate this instance if synchronous authored code blocks the event loop or cleanup ignores cancellation. Such an instance must not advertise readiness.

`onError: "continue"` is the default: clean partial failures may finish startup with a bounded private diagnostic. `"abort"` rejects startup/reload on failure. Overdue execution or incomplete cleanup is fatal for either mode. No task is allowed to continue behind a ready process. Failure logs contain fixed codes, counts and bytes rather than response bodies, credentials or application values.

When Sentry is also enabled, private configuration validation and trusted helper initialization precede the warmup watchdog and application imports. Invalid Sentry setup does not run application warmup. The existing successful module/data readiness barrier precedes the public listener; reload and shutdown retain their normal process ownership.

Use this option to prepare a small declared public working set. It does not prove a cold-cache latency target, fill distributed caches atomically or replace real readiness and deployment tests.
