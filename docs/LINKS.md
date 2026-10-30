# Native slugs and short links

The Rust backend exposes authenticated `POST /_brain/slugs` with `{title,limit?}`. Maintained Rust `slug` normalizes public text; the default limit is 80 ASCII characters. Empty results and inputs over 4 KiB are rejected. A slug is presentation, not an authorization token or a uniqueness guarantee.

Authenticated `POST /_brain/links` accepts `{target,code?,expires_in?}`. Targets must be absolute HTTP(S) URLs on an explicitly configured application origin. Credentials, control characters and credential-like query keys are refused. Automatic codes use 64 random bits from UUID generation, a database uniqueness constraint and at most five collision retries. Custom codes contain 1..80 lowercase ASCII letters, digits or hyphens. The owner is locked during creation; each owner has a bounded 10,000-record namespace. Expiry is between one minute and one year, with a 30-day default.

`GET /s/{code}` revalidates the destination against current allowed origins and returns a non-cacheable 302 only for an enabled, unexpired record. `DELETE /_brain/links/{id}` revokes only the current owner's record. Unknown and unauthorized IDs are not distinguished. Short-link paths are backend paths; deployment routing applies the application's configured base prefix. User deletion cascades its links. Database migrations are explicit and tracked by checksum.

OpenAPI and `brain backend schema --json` advertise these native routes. No redirect fetches the destination on the server, and request logs exclude the target and its query data. Private target resources must still enforce their own authorization.
