# Record ids and keeping existing numeric ids (B-08)

Every native record gets a server-generated UUID (`id`). It is the primary key, the target of every `ref:<resource>`
foreign key, the key of file attachments and slug history, and the `<id>` in `/api/<resource>/<id>`. One id type for
every table keeps references, cursors and policies uniform, and a UUID does not reveal how many records exist.

An app that moves from another backend often already has numeric ids in public URLs (`/products/1042`,
`/jobs/garden-designer-1042`). Keep them as an ordinary unique field and look records up by it.

## Contract

`server/resources/products/schema.jsbrain`:

```js
<meta server>
export default {
  title: 'str!',
  legacy_id: { type: 'int', unique: true },
}
```

`server/resources/products/query.jsbrain`:

```js
<meta server>
export default { filter: { legacy_id: ['eq', 'in'] } }
```

`unique: true` creates a unique index on the value, and the declared filter gets its own expression index, so the
lookup is an index scan. Imports write the old number into `legacy_id`; new records may leave it empty.

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

## Not offered

Choosing `bigint` as the primary key type per resource is not supported: references, cursors, file records and slug
history all assume one key type. Use the unique field above for public numbers.
