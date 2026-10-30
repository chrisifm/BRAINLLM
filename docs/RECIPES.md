# Product-flow recipes

`brain recipe` prepares files for review. Generation does not write an application, start a server, change a policy or provision accounts. Apply the returned `operations` through the atomic change-plan workflow after inspecting the contract and requirements. Every new file has `expectedRevision: null`; a recipe refuses an existing destination.

The CLI and MCP expose the same recipe list and generator:

```sh
brain recipe list --json
brain recipe resource-ui '{"resource":"notes","view":"notes-manage"}' --json
brain recipe auth-ui '{"view":"account-access"}' --json
```

`recipes::generate(root, name, params, dry_run)` always returns a preview plan, even when `dry_run` is false. `dryRun` records caller intent; `previewOnly` is always true. Parameters reject unknown keys and unsafe filenames. Generated paths are under `views/` and never overwrite existing files.

## Resource management

`resource-ui` reads the existing `server/resources/<name>/schema.jsbrain`, policy and hook contracts through the backend contract reader. Its response includes that contract. It generates one complete view with search, 25-row offset pagination, create, edit and delete. The backend remains the authority for permissions and validation. Generate the default `auth-ui` companion at `/account-access` for the resource error state sign-in link. Unauthorized operations produce visible errors; showing an editor never grants access.

Fields come from the contract: text, email, number and checkbox controls plus a JSON-object editor for `obj`. The form derives its schema from the same metadata, converts object editor values before saving, and preserves the entire record shape when replacing a record with PUT. A malformed JSON object produces a form error instead of a request. Empty optional object input becomes `{}`. Required boolean fields allow `false`, matching the backend's field-presence semantics.

The view handles loading, rejected reads, retry, empty lists, save success, delete success, field errors, request errors and disabled actions while writing. Each row uses its stable record ID. Search changes reset pagination; deletion returns to the previous offset to avoid a stranded empty last page. Next-page availability follows the backend's `{items,limit,offset}` contract, so a full last page can lead to an empty next page; totals are not fabricated.

Edits copy the loaded row. Updates use the backend's full-replacement PUT semantics, not PATCH. Backend validation catches schema changes made after generation. Regenerate under a new destination after changing the resource schema, then review the diff. Existing generated views do not dynamically change their fields.

## Account access

`auth-ui` generates signup, login, current-session checking and logout, with errors, retry and busy states. Signup creates a candidate account and then asks the user to sign in. The generated client never grants administrator privileges. Password reset, email verification, administrator provisioning and invitations are not implemented by this recipe.

## Deployment requirements

These flows target the **experimental Rust backend**, whose service is currently separate from the JavaScript SSR/dev server. Migrate the database, start the backend, and expose `/api/*` and `/_brain/auth/*` on the frontend origin through a reverse proxy. Use HTTPS outside loopback development so Secure session cookies work. The backend validates write origins; configure its allowed origin to match the frontend.

The generated endpoint paths include `/api` explicitly. Use an empty application HTTP base and deploy at the origin root; an existing `<meta http base="/api">` would duplicate the prefix. Review this requirement before applying a plan to an existing app.

Configure fixed loopback `BRAIN_BACKEND_URL` to make native Rust sessions authoritative for SSR loads, guards and server functions through the native bridge. Without that configuration the legacy signed-session demo remains separate. Do not treat a browser session indicator as authorization. Resource policy must protect every backend operation. The backend marks responses `Cache-Control: no-store`; hashed static assets retain their normal immutable caching.
