# File uploads of any type (native backend)

Declared file buckets hold the content types they list (PDF, Word, spreadsheets, zip, audio, video, images, plain
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
  types: '*',                     // MIME patterns, or '*' for any type; omitting it warns B0770
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
| `types` | `application/pdf`, `image/*`, or `'*'` for any type. Matching uses the type detected from content (below), never the file name. Omitting it, or `[]`, still accepts any type and warns B0770. |
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

Write the official MIME type; it matches what detection reports:

| In `types` | Accepts |
|---|---|
| `application/vnd.openxmlformats-officedocument.wordprocessingml.document` (and the `spreadsheetml.sheet`, `presentationml.presentation` forms) | a zip whose central directory has `[Content_Types].xml` and the format's main part. Any other zip is refused. |
| `application/msword`, `application/vnd.ms-excel`, `application/vnd.ms-powerpoint` | a compound file with the format's main stream. Other compound files, installers among them, are refused. |
| `image/heic` | detected `image/heif` |
| `image/jpg`, `audio/mp3`, `audio/wav`, `audio/mp4`, `application/x-zip-compressed` | detected `image/jpeg`, `audio/mpeg`, `audio/x-wav`, `audio/m4a`, `application/zip` |
| `application/zip`, `application/x-ole-storage` | the container and every Office format stored in it |

This is identification, not validation: a zip with those names passes without its parts being parsed. Downloads are
always `nosniff`, with `attachment` unless `?inline=true` on a type that is safe inline.

A PNG sent as `invoice.pdf` with `Content-Type: application/pdf` is therefore stored as `image/png` and rejected by a
PDF-only bucket with 415 `FILE_TYPE_DENIED`. Unrestricted buckets accept it and record what it really is.

## HTTP

The same-origin gateways stream `/_brain/files` in both directions, without buffering, up to the bucket limit. They
forward `Range`, `If-Range` and the conditional headers, relay `206` and `304` with their validators, and close the
backend connection when the client disconnects. Access checks stay in the backend.

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

## Deleting a file

`DELETE /_brain/files/<bucket>/<id>` removes the record and, in the same transaction, a database trigger records its
bytes in `_brain_file_deletions` (migration 0020). The route then removes the bytes and clears that tombstone, so a
normal delete answers 204 with nothing left behind.

If storage refuses, or the process stops first, the delete still counts: the file answers 404 at once and the tombstone
stays. Every running `brain backend serve` sweeps due tombstones at boot and then every minute, with waits doubling from
1 minute to 1 hour, and logs `file-deletion-failed` while removals keep failing. Rows removed by hand through SQL are
recorded the same way. Run several processes freely: each tombstone is claimed by one. The bytes of a rejected upload
that storage cannot drop are recorded too.

Deleting a user does not cascade to files: `_brain_files.owner` is `ON DELETE RESTRICT`, so Postgres refuses the
account while it owns files and neither rows nor bytes are orphaned. Delete the files first. Deleting a linked record
does not delete its files; remove them through this route when they should go with it.

The upload route's body limit is the largest declared `maxBytes` plus 1 MiB of multipart framing; each bucket enforces
its own limit. Uploads have a 300-second request deadline and other file requests 30 seconds (downloads stream after
their headers, so large downloads are not cut by the deadline).

## Tested example

`crates/brain-backend/tests/files.rs` runs the `orders`/`invoices` example above, an any-type `attachments` bucket and
a public `banners` bucket against the disposable Postgres harness (`node scripts/backend-check.mjs --only-tests files`).

## Keys and server code

`/_brain/files` accepts a project API key with a `files.<bucket>` scope under the same rules as a session, except that
a system key skips the bucket's roles. Jobs and routes use `ctx.backend.files(bucket)` ([job-backend.md](job-backend.md));
server `load()` and server functions use it with `BRAIN_SERVICE_API_KEY` and answer `503` without that variable.

## Who reads a file

- **`related`** evaluates the same rule as `GET /api/<resource>/<id>` on the linked record, through the same evaluator,
  for signed-in and anonymous readers. Whoever can get the row gets the file, including `owner.<ref>` owners. When the
  linked resource has row conditions, a hidden row answers `404` like its record does. Otherwise a denial is `403
  POLICY_DENIED`, and its `fix` names the rule classes evaluated and no data.
- **Public buckets** serve every file to anyone: they do not evaluate row conditions, so do not link them to rows that
  can be hidden.
- **`/meta`** of a public bucket answers anonymous readers with public fields only. `owner`, `resource`, `record`,
  `field` and `sha256` go to the uploader and to roles the bucket grants.
