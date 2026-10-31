# Loading performance defaults

What `brain build`, the production server (`dist/server.mjs`) and `brain start` do for load speed without any app
code, how to opt out, and the measured effect on a scaffolded app (`brain new shop`, debug binary, 2026-10-31).
The app-author guide is `brain guide performance`; this file is the contract for deployers and framework changes.

## Response compression

| Response | Encoding | Where |
|---|---|---|
| `/_b/*` chunks, CSS, `public/` text files | precompressed at build: `.br` (Brotli q11), `.zst` (zstd level 19), `.gz` (own optimal gzip); a sibling is written only when smaller than the file | brain-pack |
| static pages, SSR pages, streamed SSR, JSON, error pages | compressed per response (the page gets a fresh CSP nonce, so a precompressed page cannot be sent as is) | `dist/server.mjs` |

Negotiation follows `Accept-Encoding` weights (`br;q=0` forbids Brotli). For static files the server sends the
smallest precompressed sibling the client accepts. For per-response bodies it prefers zstd on ties (level 3 is the
fastest at a Brotli-like ratio), then Brotli quality 5, then gzip. zstd is offered only when `node:zlib` has it
(Node 22.15 and later); older Node answers with Brotli or gzip. Bodies under 1 KiB, `HEAD`, `204`/`304`, a response
that already has a `Content-Encoding` and `Cache-Control: no-transform` are sent as is. Every compressible response
carries `Vary: Accept-Encoding`, including its identity answer, so a shared cache never serves one encoding to a
client that did not ask for it. A streamed SSR page flushes the encoder after each chunk: the head and the loading view
still arrive before `load()` finishes.

Browser support: gzip everywhere; Brotli in every current browser over HTTPS; zstd in Chrome/Edge 123+ and Firefox
126+ (Safari does not advertise it and gets Brotli). Hosts: Caddy serves the siblings with
`precompressed zstd br gzip` (the emitted `_brain/Caddyfile` block); nginx needs `gzip_static on` and the third-party
`brotli_static`/`zstd_static` modules; static hosts that read `_headers` compress on their own.

Opt out (a reverse proxy that already compresses): `createServer({ compress: false })` or `BRAIN_COMPRESS=off`.
Static precompressed siblings are still served.

BREACH note: a page that reflects attacker-controlled input next to a secret in the same compressed body can leak
the secret through its compressed length. BRAINLLM pages carry a per-response random nonce and keep session tokens in
`HttpOnly` cookies, not in the body; forms post with same-origin checks rather than body tokens.

Measured (scaffold, `node dist/server.mjs`, transfer bytes of the HTML document):

| Page | identity (before) | gzip | br | zstd |
|---|---:|---:|---:|---:|
| `/` | 10,492 | 4,098 | 3,741 | 4,208 |
| `/about` | 6,659 | 2,984 | 2,733 | 3,054 |
| `/login` | 5,160 | 2,600 | 2,289 | 2,584 |

Before this change every HTML document left `server.mjs` uncompressed (the identity column): the home document is
now 64 % smaller with Brotli.

Verify:

```bash
brain build && BRAIN_SECRET=change-me-32-bytes-minimum-secret node dist/server.mjs &
curl -s -o /dev/null -H 'accept-encoding: br' -w '%{size_download}\n' http://127.0.0.1:3000/
curl -sI -H 'accept-encoding: gzip, br, zstd' http://127.0.0.1:3000/ | grep -i -E 'content-encoding|vary'
```
