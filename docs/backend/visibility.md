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

Not yet implemented: `private`, `readonly` and `writeonly` modifiers, and hiding `owner` from public reads by default.
