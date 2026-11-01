# Batch writes

`POST /api/<resource>/batch` creates, updates, upserts or deletes many records in ONE transaction. Either every item
is written or none is. Every item is validated before the first write, and errors are keyed by item index.

| `op` | `items` | Notes |
|---|---|---|
| `create` | record data | the caller owns every new record |
| `update` | `{ "id", "data" }` | PATCH semantics per item (only sent fields change) |
| `upsert` | record data, plus `key` and optional `on_conflict` | same rules as [`/upsert`](atomic-updates.md#upsert-insert-if-absent); the response adds `created: [bool]` |
| `delete` | `{ "id" }` | returns `{id, owner}` per item |

Policy is the same as for single writes: `create` for create/upsert, `update`/`delete` for the others. Owner-only
grants make other owners' rows answer 404. Rows are locked in id order, so concurrent batches cannot deadlock.
A duplicate id inside one batch answers 422.

```http
POST /api/posts/batch
Content-Type: application/json

{"op":"create","items":[{"title":"First"},{"title":"Second"}]}
```

```json
{"count":2,"items":[{"id":"6a1d...","owner":"0b7e...","data":{"title":"First"}},{"id":"7c2e...","owner":"0b7e...","data":{"title":"Second"}}]}
```

## Size limit: `server/resources/<r>/batch.jsbrain`

```js
<meta server>
export default { max: 5 }
```

The default is 100 items; the allowed range is 1–1000. The batch body may be up to 1 MiB. Resources with a mandatory
before-hook for the operation answer 422 `BEFORE_HOOK_REQUIRES_SINGLE_WRITES`, so hooks are never bypassed.

One batch is one transaction, so it also has to fit the time limit of an HTTP transaction: 9 seconds on Postgres 17 and
later, with 5 seconds per statement on every version. Past it the answer is 503 `BATCH_TOO_SLOW`, without `Retry-After`:
nothing was written, and the same batch will need that time again, so split it. Long text, many indexes and search
fields make each row slower. A value the database cannot store or index answers 413 `VALUE_TOO_LARGE`.

## Example

```http
POST /api/task_items/batch
content-type: application/json

{"op": "create", "items": [
  {"title": "Write docs", "code": "T-4"},
  {"code": "T-5"},
  {"title": "Review", "code": "T-6", "points": "9"}
]}
```

```json
{"error":{"code":"INVALID_INPUT","fields":{
  "items.1.title":"Required","items.2.points":"Expected an integer JSON number …"}, "fix":"…"}, "rid":"…"}
```

Nothing was written. After fixing the items:

```json
{"op":"create","count":3,"items":[{"id":"…","owner":"…","data":{"title":"Write docs","code":"T-4","points":0},"…":"…"}, "…"]}
```

| Failure | Status | Fields |
|---|---|---|
| invalid item | 422 | `items.<i>.<field>` |
| unique conflict on item 2 | 409 `UNIQUE_CONFLICT` | `items.2.code`; items 0 and 1 are rolled back |
| unknown or foreign id | 404 | `items.<i>` |
| more than `max` items | 422 | `items`: `Send 1..5 items` |

Tested by `crates/brain-backend/tests/domain_writes.rs` (`postgres_batches_are_all_or_nothing_with_per_item_errors`)
with the contracts in [`examples/shop`](examples/shop/server).
