# Access and budgets (`platform.access`, B0752)

By default nothing is limited: visitors, crawlers, scripts and headless browsers all reach every page and the native
API (`/api`, `/_brain`, `/s`). Two opt-in controls sit in `platform.access`, both checked before any body is read or the
backend is asked, so a turned-away request costs one map lookup:

| Setting | Effect | Answer past it |
|---|---|---|
| `perMinute` | dynamic requests (page renders, form posts, `/api`, `/_brain`, `/s`) per client address and minute | `429` + `Retry-After` |
| `concurrency` | dynamic requests in flight for the whole process | `503` + `Retry-After: 2`, no queue |
| `mode: "own-host"` | the native API answers the site's own pages only (below) | `403 {"code":"B0752"}` |

Static files, prerendered pages, `/_b/` assets, `sitemap.xml`, `robots.txt`, `llms.txt` and anything a CDN caches carry
no budget. Turned-away requests are logged as `request limited` (four lines a minute, then one `requests limited` line
with the count per minute).

## Own-host mode

With `mode: "own-host"` the native API is not a public API: a page of the site reads it, and so do callers you name.
Everything else gets `403 {"code":"B0752"}`.

## What passes

| Caller | Evidence | Passes |
|---|---|---|
| A page of the site | `Origin` of this host, `Sec-Fetch-Site: same-origin`, or a `Referer` of this host | yes |
| A person who opens an API URL in the address bar | `Sec-Fetch-Site: none` on a navigation | yes |
| A sign-in provider returning to `/_brain/auth/*` | a top-level navigation | yes |
| Server code (`load()`, server functions, `ctx.backend`) | never leaves the machine | yes |
| A service, script or partner with an API key | `X-Api-Key` or `Authorization` (the backend verifies it, `docs/backend/api-keys.md`) | yes |
| A server whose address you list | client address in `access.ips` | yes |
| Loopback clients while `access.local` is on (the default) | resolved client `127.0.0.1` / `::1` | yes |
| Another site's page, a mirror on another host name, a script without browser evidence | — | **403 B0752** |

Short links (`/s/…`), the analytics script, signed inbound webhooks and Apple's account notifications are reached by
third parties on purpose and keep their own checks. `/_brain/files` keeps its own bucket access rules, so public images
still load in previews and mail.

## Search engines

Pages, `sitemap.xml`, `robots.txt`, `llms.txt`, files and short links are not behind this check, so Google and every
other crawler index the site as before; what they index is the server-rendered HTML. When a crawler renders the page
with JavaScript (Googlebot runs a current Chrome), its API calls carry the page's own `Sec-Fetch-Site` and pass like a
visitor's. With `perMinute` set, keep it above what a crawler
needs from one address; a 429 makes Googlebot slow down, not leave. A crawler is never let through by its `User-Agent`: anyone can claim to be Googlebot. In a CDN, exempt
verified bots by the CDN's own verification (Cloudflare: `cf.client.bot`), never by the user agent.

## Configure

```js
// server/platform.jsbrain
<meta server>
export default {
  trustProxy: ['127.0.0.1', '::1'],
  access: {
    perMinute: 300,                           // optional: dynamic requests per client and minute
    concurrency: 128,                         // optional: dynamic requests in flight for the process
    mode: 'own-host',                         // optional: refuse the native API to other sites and plain scripts
    origins: ['https://admin.example.com'],   // other browser origins of yours (exact scheme://host[:port])
    hosts: ['www.example.com'],               // extra host names the site answers on; PUBLIC_SITE_URL's host is implied
    ips: ['203.0.113.10', '198.51.100.0/28'], // servers that call without a browser and without an API key
    local: true,                              // loopback clients pass; set false to try the refusals on localhost
  },
}
```

- Behind a proxy or a CDN, `trustProxy` (and `trustPublicProxy` for a CDN) must be set, or every visitor resolves to
  the proxy's loopback address: it passes as local and is never budgeted, or, with `local: false`, every visitor
  shares one budget. `brain doctor` reports it (B0784, B1892).
- A request on a host name that is not the site (`hosts` plus the host of `PUBLIC_SITE_URL`) is refused even with
  browser evidence: someone pointing their own domain at your server does not get your API.

## See who is refused

Every refusal is logged as `access refused` (at most 16 lines a minute) with the reason, client address, path, origin,
referer, host, user agent and `cf-ipcountry`. On the server itself, the running process lists the refused sources,
most frequent first (at most 512 addresses):

```bash
curl -s http://127.0.0.1:$PORT/_access/sources | jq '.sources[:10]'
```

The endpoint answers only a loopback socket without a forwarded hop, so it is not reachable through the proxy.

| Reason | Meaning |
|---|---|
| `no-browser-evidence` | no Origin, no Sec-Fetch-Site and no Referer of the site: a script or a scraper |
| `origin` | the browser said another origin |
| `cross-site` | a browser on another site, or a script imitating one |
| `host` | the request named a host that is not the site |

## What it does not do

Both controls run in the Node server that `brain start` runs for an app with `load()` or server functions. A fully static
build served by the built-in gateway, and `brain run`, do not apply it yet.

Headers can be forged. This check keeps other sites' pages, mirrors on foreign hosts and plain scripts out; a scraper
that copies a browser's headers still reads at the per-client rate (`perMinute`, `platform.limits`). Put a CDN rule in front for
volume (rate limiting per path, bot challenges) and keep `trustPublicProxy` set so the limits key on the real visitor.
