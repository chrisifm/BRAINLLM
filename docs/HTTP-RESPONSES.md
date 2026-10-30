# HTTP responses and safe error defaults

The Rust resource backend and the Node production server supply error responses when no application page handles the failure. Default pages use the registered HTTP status title, an accessible `main`/`h1`, mobile viewport and a request reference. They never reflect the request path, query, exception message or stack. Errors carry `X-Request-Id` and `Cache-Control: no-store`; private errors also use `private`. Server diagnostics retain the corresponding reference.

## Negotiation and custom responses

API namespaces (`/api`, `/_brain`, `/_r`) remain JSON, including ambiguous `Accept: */*`. Elsewhere, HTML and JSON preferences respect media quality and explicit exclusions. A client requesting JSON receives a machine-readable error rather than a document. Negotiated errors vary on `Accept`.

The Node server preserves the configured 404 page, the existing forbidden/error component contracts, and status-specific static pages such as `410.html`. Optional manifest `errors` entries can name a status page. These files pass the same dist containment and symlink checks as ordinary static assets. Explicit application JSON is preserved; this feature does not overwrite RPC validation payloads. It does not create dozens of physical error-page files.

HEAD returns the status and headers with no body. Informational responses, 204, 205 and 304 never receive an error document or entity body. Early Hints remain an independent informational response. A failure after streaming headers have been sent keeps the established inline error fallback; HTTP cannot replace the already-sent status at that point.

## Typed Rust helpers

`brain_backend::response` exposes ordinary Axum-compatible values:

```rust
response::json(value)                 // 200 JSON; T: Serialize
response::created("/api/items/1", value)? // 201 JSON + Location
response::no_content()                // 204
response::status(404)?                // empty status; middleware supplies error default
response::redirect("/login", 303)?    // empty redirect + Location + no-store
```

`status` accepts registered final codes from 200 through 599. Redirects accept only 301, 302, 303, 307 and 308. Invalid values return `ResponseError`, which itself implements `IntoResponse` with a safe 500 response. The response middleware invokes `finalize(method, path, accept, rid, response)` after handlers. It preserves semantic headers such as `Allow`, `Retry-After` and `WWW-Authenticate`. Custom JSON/HTML responses are preserved; only framework-marked errors or generic rejections are converted to negotiated defaults.

These are Rust library APIs. No additional BrainScript response primitive or arbitrary custom HTTP endpoint syntax is implied.

## Redirect policy

Application redirects require a same-site absolute path, starting with one `/`. Schemes, authority-relative URLs, backslashes, whitespace/control characters, encoded controls and encoded leading slash tricks are rejected. Location length is bounded. Both returned/thrown guard/load redirect shapes and form redirects share validation. Redirect status codes are validated before sending.

Explicit external HTTP(S) redirects already declared in the build's redirect manifest remain supported. They cannot carry credentials, raw control characters or backslashes. This configured mapping is separate from application-provided redirect values; arbitrary function results cannot opt into external redirects.

The Node production server's `redirect(path, status)` uses the same redirect code set. `HttpError` load failures preserve registered 4xx/5xx statuses with safe status pages; unexpected errors stay generic 500 responses, with details restricted to private logs.

## Evidence and standards

Focused Node tests exercise every registered 4xx/5xx status recognized by the installed Node HTTP implementation, custom 404/410 pages, HTML/JSON negotiation, HEAD, method headers, trace references and redirect defenses. Rust unit tests cover helper validation, custom JSON/challenge preservation, negotiation and empty bodies; execute them before release.

The implementation follows [RFC 9110 HTTP semantics](https://www.rfc-editor.org/rfc/rfc9110.html), including HEAD and status-specific body rules, and [Axum's response contract](https://docs.rs/axum/latest/axum/response/index.html). A document can be customized without exposing a private exception or weakening cache/security defaults.
