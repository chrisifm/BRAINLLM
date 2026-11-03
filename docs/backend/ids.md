# Record ids and keeping existing numeric ids (B-08)

Every native record gets a server-generated UUID (`id`). It is the primary key, the target of every `ref:<resource>`
foreign key, the key of file attachments and slug history, and the `<id>` in `/api/<resource>/<id>`. One id type for
every table keeps references, cursors and policies uniform, and a UUID does not reveal how many records exist.

An app that moves from another backend often already has numeric ids in public URLs (`/products/1042`,
`/products/garden-chair-1042`). Keep them as an ordinary unique field and look records up by it, only if the number
is already public: the field below is returned to every reader of the record, and a consecutive number tells anyone how
many records exist and lets them walk through them. An old id that was never public does not belong in the record: see
[Keeping an old id private](#keeping-an-old-id-private).

## Contract

`server/resources/products/schema.jsbrain`:

```js
<meta server>
export default {
  title: 'str!',
  legacy_id: { type: 'int', unique: true, visible: ['public'] },
}
```

`server/resources/products/query.jsbrain`:

```js
<meta server>
export default { filter: { legacy_id: ['eq', 'in'] } }
```

`unique: true` creates a unique index on the value, and the declared filter gets its own expression index, so the
lookup is an index scan. Imports write the old number into `legacy_id`; new records may leave it empty.
`visible: ['public']` states that the number is meant to be public; without it, `brain check` warns B0797.

## Lookup

```http
GET /api/products?legacy_id=1042&limit=1
```

```json
{"items":[{"id":"7b1d…","data":{"title":"Hand trowel","legacy_id":1042},"created_at":"…","updated_at":"…"}],
 "limit":1,"offset":0,"total":1}
```

From a view, keeping the old URL shape `/products/<title>-<number>`:

```js
export async function load(ctx) {
  const number = Number(ctx.params.slug.split('-').pop())
  if (!Number.isSafeInteger(number)) throw notFound()
  const page = await ctx.backend.resource('products').list({ filter: { legacy_id: number }, limit: 1 })
  if (!page.items.length) throw notFound()
  return { product: page.items[0] }
}
```

A second `POST` or `PUT` with a `legacy_id` that another record already holds answers `409` with the field name, so an
import that runs twice cannot create duplicates.

## Keeping an old id private

A field `private`, `writeonly` or with `visible` including `owner` cannot be a filter, and one visible to named roles
only filters for those roles alone ([visibility](visibility.md)): other readers could infer its value. To resolve old ids without showing them, keep the pairs in a server-only resource and look them up from the
server:

```js
// server/resources/legacy_ids/schema.jsbrain
export default { old_id: { type: 'int!', unique: true }, product: 'ref:products!' }
// server/resources/legacy_ids/policy.jsbrain
export default { api: false, read: ['admin'], create: ['admin'], update: ['admin'], delete: ['admin'] }
// server/resources/legacy_ids/query.jsbrain
export default { filter: { old_id: ['eq'] } }
```

`ctx.backend.service('legacy_ids').list({ filter: { old_id: number }, limit: 1 })` then gives the current id. An
`api: false` resource answers no HTTP route and appears in no public catalog.

## Not offered

Choosing `bigint` as the primary key type per resource is not supported: references, cursors, file records and slug
history all assume one key type. Use the unique field above for public numbers.
