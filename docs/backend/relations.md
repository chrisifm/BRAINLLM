# Relations (B-01)

A `ref:<resource>` field stores the id of a record of another resource. The backend keeps a real foreign key for it,
validates that the referenced record exists on every write, and can include the referenced record in reads.

## Contract

```js
// server/resources/shops/schema.jsbrain
export default { name: 'str!', country: 'str!' }

// server/resources/products/schema.jsbrain
export default {
  shop: { type: 'ref:shops!', on_delete: 'cascade' },
  title: 'str!',
  price: 'decimal(10,2)!',
}

// server/resources/orders/schema.jsbrain
export default {
  product: { type: 'ref:products', on_delete: 'set_null' },
  quantity: 'int!',
}

// server/resources/products/query.jsbrain
export default { include: ['shop'], filter: { shop: ['eq'], 'shop.country': ['eq'] } }
```

`on_delete` is one of:

| Value | Deleting the referenced record |
|---|---|
| `restrict` (default) | is refused with `409 REFERENCE_IN_USE` while any row still points at it |
| `cascade` | also deletes the rows that point at it |
| `set_null` | removes the key from the referencing rows (only for optional fields) |

Self references and cycles are allowed: foreign keys are installed after every resource table exists. A reference
inside a job payload is rejected; jobs carry ids as plain strings.

## Writes

```http
POST /api/products
{"shop": "9b1f…", "title": "Hand trowel", "price": "12.50"}
```

An id that does not exist answers:

```json
{"error": {"code": "REFERENCE_NOT_FOUND", "fields": {"shop": "No shops record with this id"},
  "fix": "Reference an existing record of the target resource in the listed fields"}, "rid": "…", "schemaVersion": 1}
```

Existence is enforced by the database foreign key, so a concurrent delete cannot leave a dangling reference.

## Include

```http
GET /api/products?include=shop
GET /api/products/4c2e…?include=shop
```

```json
{"items": [{"id": "4c2e…", "data": {"shop": "9b1f…", "title": "Hand trowel", "price": "12.50"},
  "included": {"shop": {"id": "9b1f…", "data": {"name": "North Garden", "country": "CA"}}}}],
 "limit": 25, "offset": 0, "total": 1}
```

Only fields listed in `include` can be requested. The included record follows the target resource's read policy and
field visibility; a record the reader may not see is returned as `null`. Includes run as one query per included field,
never one query per row.

### Nested levels

A dotted path follows references into the attached records, up to three levels:

```http
GET /api/reviews?include=product.shop.region
```

```json
{"id": "71aa…", "data": {"product": "4c2e…"}, "included": {"product": {"id": "4c2e…", "data": {"shop": "9b1f…"},
  "included": {"shop": {"id": "9b1f…", "data": {"region": "e07b…"}, "included": {"region": {"id": "e07b…", "data": {"name": "North"}}}}}}}}
```

Each level must be listed in the `include` of its own resource, so `reviews` declares `product`, `products` declares
`shop` and `shops` declares `region`. Every level applies that resource's read policy and field visibility: a hidden row
renders as `null`, exactly like a missing one, and nothing below it is read. A reference field the reader cannot see is
not followed. A request takes at most 16 paths and attaches at most 500 records in total; past that it answers
`422` instead of a partial result. With a project API key, every level also needs a read scope (see
[api-keys.md](api-keys.md#related-resources)).

## Filter by a related field

`'shop.country': ['eq']` filters products by the country of their shop:

```http
GET /api/products?shop.country=CA
```

The related field must be a filterable field of the referenced resource; the predicate is an indexed
`ref_shop IN (SELECT id FROM shops WHERE …)` subquery.

## Schema projection

`brain backend schema --json` lists each reference with its target and `on_delete` action. The executed example lives
in `crates/brain-backend/tests/data_queries.rs` (shops, products, reviews and orders).
