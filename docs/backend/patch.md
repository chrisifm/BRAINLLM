# Partial updates (PATCH)

`PATCH /api/<resource>/<uuid>` changes only the fields in the body. Every other stored field keeps its value. The
merged record is then validated against the full contract (types, defaults, constraints) and written with the same
optimistic conflict check as `PUT`. No contract file is needed: every resource gets the route.

| Body | Effect |
|---|---|
| `{"stock": 5}` | sets `stock`, keeps every other field |
| `{"note": null}` | removes an optional field; a field with a `default` gets its default again |
| `{}` | 422 `INVALID_INPUT` (`body`: send at least one declared field) |
| `{"colour": "red"}` (undeclared) | 422 `INVALID_INPUT` (`colour`: Unknown field); nothing is written |

Policy, ownership and hooks are the same as `PUT`: the `update` grant applies, owner-only grants answer 404 for other
owners' records, and a declared `before_update` hook receives the merged record as its payload. When two writers race,
the second write answers 409 `RESOURCE_CHANGED` instead of overwriting a change it never saw.

## Example

`server/resources/products/schema.jsbrain`:

```js
<meta server>
export default {
  name: 'str!',
  sku: { type: 'str!', unique: true },
  stock: { type: 'int', default: 0 },
  price: { type: 'decimal(10,2)', default: '0.00' },
  note: 'str'
}
```

`server/resources/products/constraints.jsbrain`:

```js
<meta server>
export default { check: [{ field: 'stock', min: 0 }] }
```

Stored record: `{"name":"Mug","sku":"MUG-1","stock":3,"price":"9.50","note":"blue"}`.

```http
PATCH /api/products/7b1d…
content-type: application/json

{"stock": 5, "note": null}
```

```json
{"id":"7b1d…","owner":"…","data":{"name":"Mug","sku":"MUG-1","stock":5,"price":"9.50"},
 "created_at":"2026-10-31T10:00:00.000Z","updated_at":"2026-10-31T10:05:00.000Z"}
```

```http
PATCH /api/products/7b1d…

{"stock": -1}
```

```json
{"error":{"code":"INVALID_INPUT","fields":{"stock":"Must be >= 0"},"fix":"…"},"rid":"…","schemaVersion":1}
```

A resource with a declared state machine (`transitions.jsbrain`, see [state-machines.md](state-machines.md)) rejects a
PATCH that changes its state field with 422; use the transition route instead.

OpenAPI (`/_brain/api`) documents the operation as `<resource>_patch`: every field optional, `null` accepted for
optional or defaulted fields, `minProperties: 1`.

Tested by `crates/brain-backend/tests/domain_writes.rs` (`postgres_patch_updates_only_sent_fields_with_policy_and_conflict_checks`).
