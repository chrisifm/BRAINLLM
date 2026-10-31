# File uploads of any type (native backend)

Declared file buckets accept any content type by default (PDF, Word, spreadsheets, zip, audio, video, images, plain
text), stream the body to private disk without buffering it in memory, record metadata in Postgres and serve
downloads through the bucket's policy with `Range` and `ETag` support. The image pipeline in [MEDIA.md](../MEDIA.md)
is unchanged and remains the place for responsive renditions.

## Declare a bucket

One literal file per bucket: `server/files/<name>.jsbrain`. The file name is the bucket name (lowercase identifier).

```js
// server/files/attachments.jsbrain — a task app's attachments, any type
<meta server>
export default {
  visibility: 'private',          // 'private' (default) or 'public'
  maxBytes: '25mb',               // integer bytes or '512kb' / '25mb' / '1gb'; default 25 MiB, ceiling 2 GiB
  types: [],                      // MIME patterns; empty (default) accepts any type
  create: ['user'],               // roles that may upload (default ['admin'])
  read: ['owner', 'admin'],       // default; also 'related' when a resource is declared
  delete: ['owner', 'admin']      // default
}
```

```js
// server/files/invoices.jsbrain — PDFs that belong to a shop order record
<meta server>
export default {
  resource: 'orders',             // uploads need ?record=<uuid> and update rights on that order
  field: 'invoice',               // documents which field of orders stores the returned file id
  types: ['application/pdf'],
  maxBytes: '10mb',
  create: ['user'],
  read: ['owner', 'related', 'admin']   // related: anyone the orders read policy lets read that order
}
```

| Key | Meaning |
|---|---|
| `visibility` | `public` downloads need no session and are cached `public, max-age=31536000, immutable` (an id never changes content). `private` downloads require a grant and are `no-store`. |
| `maxBytes` | Per-bucket limit enforced while streaming; the request is cut at the first byte over the limit (413). |
| `types` | `application/pdf`, `image/*`, `*/*`. Matching uses the type detected from content (below), never the file name. |
| `resource` / `field` | Link files to records of a declared resource. |
| `read` / `create` / `delete` | Configured role names, `owner` (uploader) and `related` (resource-linked buckets). `create` takes role names only. |

`brain backend schema --json` reports the resolved buckets under `platform.files`. Invalid contracts fail at
`schema`/`serve` with the file name, for example `server/files/invoices.jsbrain: read names an unconfigured role`.
No migration is generated per bucket: one `_brain_files` table (installed by `brain backend migrate`) holds the
metadata of every bucket, with indexes by bucket/owner and by linked record.

## Content type detection

The first 8 KiB are sniffed with magic numbers (`infer` 0.22). The response records three types:

- `sniffedType`: the detected type, or `null` when the content has no signature (CSV, plain text, unknown binary).
- `declaredType`: what the client sent (normalized, informational only).
- `contentType`: the type used for policy matching and downloads. It is the sniffed type when detected; for
  undetected UTF-8 text it is the declared text type (`text/csv`, `application/json`) or `text/plain`; otherwise
  `application/octet-stream`.

A PNG sent as `invoice.pdf` with `Content-Type: application/pdf` is therefore stored as `image/png` and rejected by a
PDF-only bucket with 415 `FILE_TYPE_DENIED`. Unrestricted buckets accept it and record what it really is.

## HTTP

All mutations need the configured browser `Origin` (same rule as every native mutation).

| Method and path | Behavior |
|---|---|
| `POST /_brain/files/<bucket>[?record=<uuid>]` | multipart/form-data with exactly one field named `file`; 201 + `Location` + metadata |
| `GET /_brain/files/<bucket>/<id>[?inline=true]` | download; `Range`, `If-Range`, `If-None-Match`, `HEAD` |
| `GET /_brain/files/<bucket>/<id>/meta` | metadata under the read grant |
| `GET /_brain/files/<bucket>?limit=25&offset=0[&record=<uuid>]` | role grant: all files; `related` with `record`: that record's files; `owner`: own files |
| `DELETE /_brain/files/<bucket>/<id>` | 204; removes the row and the stored bytes |

Example upload and response:

```sh
curl -X POST 'http://127.0.0.1:5182/_brain/files/invoices?record=7d1c2f0e-4c1e-4a43-9b4e-2f9d1c0b7a55' \
  -H 'Origin: http://127.0.0.1:5182' -b '__Host-brain=<session>' \
  -F 'file=@invoice-1042.pdf;type=application/pdf'
```

```json
{
  "id": "0c9b2c5e-1d0f-4e8e-9a51-3c1f1b7f3a10", "bucket": "invoices",
  "owner": "5b8e…", "resource": "orders", "record": "7d1c2f0e-4c1e-4a43-9b4e-2f9d1c0b7a55", "field": "invoice",
  "filename": "invoice-1042.pdf", "contentType": "application/pdf", "sniffedType": "application/pdf",
  "declaredType": "application/pdf", "bytes": 48213, "sha256": "9f2c…", "visibility": "private",
  "createdAt": "2026-10-31T15:04:05.123Z", "url": "/_brain/files/invoices/0c9b2c5e-1d0f-4e8e-9a51-3c1f1b7f3a10"
}
```

Store the returned `id` in the record yourself (`PUT /api/orders/<id>` with `invoice: "<file id>"`); uploading never
rewrites a record.

Partial download (video seeking, resumable downloads):

```http
GET /_brain/files/invoices/0c9b2c5e-1d0f-4e8e-9a51-3c1f1b7f3a10
Range: bytes=0-1023

HTTP/1.1 206 Partial Content
Content-Range: bytes 0-1023/48213
Accept-Ranges: bytes
ETag: "9f2c…"
Content-Disposition: attachment; filename="invoice-1042.pdf"; filename*=UTF-8''invoice-1042.pdf
```

One range per request is served; a multi-range request receives the whole file (200). A syntactically invalid range is
ignored (200); a range outside the file is 416 with `Content-Range: bytes */<size>`. `If-None-Match` with the ETag
answers 304. Downloads are attachments; `?inline=true` renders inline only for images (not SVG), PDF, plain text,
audio and video. The global `nosniff` and `default-src 'none'` CSP headers apply to every file response.

| Status | Code | When |
|---|---|---|
| 400 | `INVALID_MULTIPART` | not multipart, field not named `file`, or more than one field |
| 401 | `SIGN_IN_REQUIRED` | private download or any upload without a session |
| 403 | `POLICY_DENIED` | role/owner/related grants do not allow the operation |
| 404 | `NOT_FOUND` | unknown bucket or file |
| 413 | `FILE_TOO_LARGE` | body exceeded `maxBytes` |
| 415 | `FILE_TYPE_DENIED` | detected type outside `types`; `fields.file` lists detected and accepted types |
| 422 | `INVALID_INPUT` | empty file, missing or unexpected `record` |

## Storage and limits

Bytes are written to `<uploads>/files/.incoming/<uuid>.part` (mode 0600, directories 0700), hashed with SHA-256 while
streaming, synced, then published with an exclusive hard link to `<uploads>/files/<bucket>/<aa>/<uuid>`; a failed or
rejected upload removes its part. Client file names never enter a path: they are reduced to their last component,
stripped of control characters and quotes, and bounded to 255 bytes. Downloads verify the stored size against the
record before streaming.

The upload route's body limit is the largest declared `maxBytes` plus 1 MiB of multipart framing; each bucket enforces
its own limit. Uploads have a 300-second request deadline and other file requests 30 seconds (downloads stream after
their headers, so large downloads are not cut by the deadline).

## Tested example

`crates/brain-backend/tests/files.rs` runs the `orders`/`invoices` example above, an any-type `attachments` bucket and
a public `banners` bucket against the disposable Postgres harness (`node scripts/backend-check.mjs --only-tests files`).
