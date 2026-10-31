# Field visibility (B-20)

A field can be restricted to some readers with `visible`. Restricted fields are removed from every response the reader
is not entitled to: lists, reads, includes, and the responses of create and update.

## Contract

```js
// server/resources/products/schema.jsbrain
export default {
  title: 'str!',
  price: 'decimal(10,2)!',
  cost: { type: 'decimal(10,2)', visible: ['owner', 'admin'] },
}
```

Grants: `public` (every reader of the record), `owner` (the row's owner, including relational owners from
[ownership](ownership.md)) and role names. There is no implicit administrator bypass; list `admin` explicitly.

## Response

Anonymous reader:

```json
{"id": "4c2e…", "data": {"title": "Hand trowel", "price": "12.50"}}
```

The shop owner:

```json
{"id": "4c2e…", "data": {"title": "Hand trowel", "price": "12.50", "cost": "7.10"}}
```

## Closed query surface

Restricted fields cannot be declared as filters, sort keys, search fields, aggregate inputs or group keys; the
contract fails to load with a fix message. A reader therefore cannot infer hidden values by filtering or sorting on
them.

## Access modifiers

A modifier after the type (or `access` in the object form) says who writes and who reads a field:

```js
// server/resources/posts/schema.jsbrain
export default {
  title: 'str!',
  status: { type: 'enum(draft|live)', default: 'draft', access: 'readonly' },
  api_key: 'str writeonly',
  score: 'int private',
}
```

| Modifier | Client may send it | Returned to readers |
|---|---|---|
| `readonly` | no; defaults and before-write hooks set it | yes (OpenAPI `readOnly`) |
| `writeonly` | yes | never (OpenAPI `writeOnly`) |
| `private` | no; defaults and before-write hooks set it | never |

Sending a `readonly` or `private` field answers 422 before any hook runs:

```json
{"error": {"code": "INVALID_INPUT", "fields": {"status": "Field is readonly; it is set by the server, remove it from the request"},
  "fix": "Apply the field errors and send only fields declared in schema.jsbrain"}, "rid": "…", "schemaVersion": 1}
```

A full replacement (`PUT`) that omits a field with a modifier keeps its stored value, so a write-only secret is not
erased by a client that cannot read it back. `writeonly` and `private` fields are closed to the query surface like
restricted fields. After-write hooks and jobs still receive the full record on the server.

## Owner id

The record `owner` id is returned only to the owner, to relational owners and to roles named in the read policy.
Anonymous and other readers do not receive it. To publish it (for example an author link), declare it:

```js
// server/resources/notes/query.jsbrain
export default { expose_owner: true }
```
