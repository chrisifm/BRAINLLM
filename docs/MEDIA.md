# Native private media library

The experimental Rust backend keeps a validated original and generates versioned image renditions with the maintained `image` crate (0.25.10). The default provider is the application's private filesystem storage. A generated optional worker uses the official Google Cloud Storage Node SDK (8.4.0) with Application Default Credentials. Frontend bundles contain no storage SDK or credentials.

## Input, images and metadata

Uploads accept one multipart field named `file`, containing PNG, JPEG or WebP. Client filenames and declared MIME types never choose paths or encoders. Input is limited to 5 MiB, 4,096 by 4,096 pixels and a 96 MiB decoded buffer. EXIF orientation is applied to derived images. The original validated bytes remain unchanged in private storage, including their original metadata; originals download as attachments. Derived images are re-encoded from pixels and do not copy source metadata or trailing content.

The default plan contains full/thumbnail plus 320/640/1280-width JPEG and WebP variants, with no upscaling. At most six named custom presets can accompany full/thumbnail. Presets select dimensions, proportional fit or centered crop, and png/jpeg/webp. JPEG exposes an actual quality setting from 1 through 100; alpha is composited on white. The current WebP encoder is lossless, so no lossy quality/compression claim is made. AVIF is not enabled.

Each output is limited to 20 MiB and the complete preparation to 64 MiB. Two image workers are shared by upload and rebuild operations; their permits remain with blocking encoders after request cancellation. Two independent GCS worker slots have no pending process queue. General HTTP requests retain a ten-second deadline; media requests allow thirty seconds, including the bounded SDK operation.

Alt text, captions, dimensions, MIME, bytes, hashes and versions are stored as metadata. Alt text is limited to 1,024 bytes and captions to 4,096 bytes. Captions are text; applications should render them through ordinary escaped bindings. The library returns rendition URLs and format-specific srcsets with unique width descriptors. Cropped aspect ratios are excluded from a source image's proportional srcset.

## API

The normal authentication, owner/admin checks and mutation Origin defenses apply.

| Method and path | Behavior |
|---|---|
| POST `/_brain/uploads` | Upload and prepare an image; returns id, actual mediaType, dimensions, bytes, version and URL |
| GET `/_brain/uploads/:id?thumbnail=true` | Compatible private attachment download; old flat PNG files remain readable |
| GET `/_brain/media?limit=24&offset=0` | Current user's ready image metadata and responsive rendition URLs |
| PATCH `/_brain/media/:id` | `{alt,caption}` for an owned image, or administrator access |
| POST `/_brain/media/:id/variants` | `{presets:[{name,width,height,crop,format,quality?}]}` creates a new version from the original |
| GET `/_brain/media/:id/v/:version/:preset` | Authorized rendition; `original` is an attachment |
| DELETE `/_brain/media/:id` | Removes metadata access and returns 202 with durable cleanup queued |

List limits are 1–100 and offsets 0–10,000. Full and thumbnail are reserved preset names. Rebuilds preserve all previously published rendition versions; a single image supports at most 64 versions. Requests for unpublished/pending versions are unavailable. Library responses expose application URLs, without private paths, bucket names or object keys.

Private media uses no-store for logout privacy. Version/hash identities keep bytes stable. Publishing derivatives to a public CDN needs its own explicit publication policy; originals do not become public automatically.

## Storage and atomic publication

Keys group objects by owner, UTC year/month, image UUID and version/source hash. Individual encoded assets include their own SHA-256 hash. Byte-identical aliases share one physical object; names/qualities/crops are still separate metadata contracts. A minimal byte difference selects a different immutable key.

Filesystem storage creates private directories/files, rejects symlink paths and publishes through an exclusive hard link from a synced owned temporary file. Existing objects are never overwritten. Reads have a 20 MiB bound and check the recorded byte count and hash before returning data. Source filenames never enter this namespace.

Uploads commit a private staged metadata record before writing. Only a successful final publication marks it ready. Rebuilds reserve prospective cleanup identities under a lease, write the new objects, then publish their metadata/version with a compare-and-swap check. Old versions stay live throughout. A failed or crashed operation leaves durable cleanup evidence; it does not expose a partially written new version.

Deleting metadata records exact object identities in the existing deletion outbox. Cleanup claims finite batches of at most eight with leases and acknowledges only the claimed token. Staged uploads older than one hour are eligible for cleanup. Filesystem deletion stays within the configured root and verifies that the key belongs to the claimed image. Missing objects are idempotent successes. An unavailable GCS configuration defers its work without consuming retry attempts. No broad folder/bucket deletion is used.

## Optional GCS provider

`media::gcs_files()` supplies an offline three-file setup plan under `server/storage`: an adapter, a private package manifest pinned to `@google-cloud/storage` 8.4.0, and provisioning instructions. Applying the plan does not install packages, create a bucket, change IAM or contact Google.

Configure private backend values `BRAIN_GCS_BUCKET`, `BRAIN_GCS_PREFIX` and absolute `BRAIN_GCS_ADAPTER`, plus ADC through the normal server credential store. The selected bucket must enforce public access prevention and uniform bucket-level access. The service identity requires bucket metadata read and the intended object permissions for its configured scope. No credential file is generated in public/dist or passed to a browser.

The Rust worker starts only the configured Node adapter, with a clean allowlisted environment. Request JSON is limited to 32 KiB, stdout to 64 KiB, and each process to fifteen seconds plus bounded termination/reaping. SDK errors become static public codes; private SDK messages, tokens and credentials are not logged or returned. Read staging files are owned and cleaned on completion/cancellation.

The adapter validates its bucket/prefix/object identities before importing the SDK. Conditional creation uses generation zero; a pre-existing object is accepted only after reconciling its expected size/type/hash metadata. Reads and deletes pin the recorded generation. A delete with an unknown generation first resolves the exact immutable hash identity. CRC32C transport validation and SHA-256 content checks protect transferred bytes. Automatic SDK retries are disabled; the operation's immutable identity remains available for explicit reconciliation.

Cloud-backed uploads use private local staging, then release those local copies after confirmed cloud persistence. Durable filesystem receipts cover partial/cancelled staging work as well. Existing GCS records are scoped to their bucket/prefix; changing provider configuration does not silently read or delete another scope.

## Rust interface and evidence

The library exposes `Store::new(root,gcs)`, `prepare(bytes,presets)`, `insert`, `library`, `asset`, `update_metadata`, `build_variants`, `delete`, `cleanup`, and `gcs_files`. Storage/provisioning choices are explicit; the library does not start a service or install a provider.

Focused Rust tests cover decoding/format rejection, original preservation, no upscaling, centered crop, alpha/JPEG handling, hashes, exclusive writes, integrity checks, symlinks and srcset descriptors. Disposable Postgres tests cover owner isolation, metadata, old/new version reads and cleanup without deleting unrelated files. Node tests inject an SDK stub for generation preconditions, private bucket policy, transport/content checks, idempotent reconciliation, scope rejection and invalid input. Synthetic provider tests do not claim an external cloud upload occurred.

Sources checked: [image 0.25.10](https://docs.rs/image/latest/image/), [WordPress image-size/crop contract](https://developer.wordpress.org/reference/functions/add_image_size/), [official Cloud Storage Node SDK](https://docs.cloud.google.com/nodejs/docs/reference/storage/latest), and [public access prevention](https://docs.cloud.google.com/storage/docs/public-access-prevention). SDK 8.4.0 was verified against the official npm registry; the repository main branch's package version was older and was not used as the version authority.

## Native CLI

`brain media setup-gcs --json` returns guarded create-only operations for the private official SDK adapter; existing files are preserved. `brain media list --owner <uuid> --limit 50 --offset 0 --json` queries that explicit owner's library. `brain media cleanup --limit 8 --json` processes only recorded deletion/staging work with bounded leases. Both database operations use the selected `--env` and require DATABASE_URL; neither migrates automatically. Cleanup is an explicit mutating operation, not part of metadata inspection. API upload and rendition routes remain available for authenticated application workflows.
