# Querying resources from server code (`ctx.backend`)

Server `load(ctx)`, server functions, actions and the development sidecar get `ctx.backend` when the app is connected
to the native backend (`BRAIN_BACKEND_URL`). It speaks the same contract as the HTTP API: the backend validates every
filter, sort and field against the resource's `query.jsbrain` and `schema.jsbrain`, applies the reader's policy and
ownership, and answers the same JSON. There is no second database client.

| Call | HTTP equivalent |
|---|---|
| `resource(name).list(options)` | `GET /api/<name>?…` ([filters](filters.md), [sorting](sorting.md), [pagination](pagination.md), [aggregates](aggregates.md), [search](search.md), [relations](relations.md)) |
| `resource(name).get(id, { include })` | `GET /api/<name>/<id>?include=…` |
| `resource(name).create(values)` | `POST /api/<name>` |
| `resource(name).update(id, values)` | `PUT /api/<name>/<id>` (full replace) |
| `resource(name).patch(id, partial)` | `PATCH /api/<name>/<id>` ([only the sent fields](patch.md)) |
| `resource(name).remove(id)` | `DELETE /api/<name>/<id>` |
| `slugRedirect(name, slug)` or `resource(name).slugRedirect(slug)` | `GET /api/<name>/redirects/<slug>` ([slug history](slugs.md)) |

Writes (`create`, `update`, `patch`, `remove`) only run inside a same-origin `POST` server function; `load()` reads.

## List options

```js
await ctx.backend.resource('products').list({
  filter: {
    category: { in: ['tools', 'garden'] },   // category[in]=tools,garden
    price: { between: ['10.00', '25.00'] },  // price[between]=10.00,25.00
    stock: { gt: 0 },                        // stock[gt]=0
    title: 'Mug',                            // title=Mug (eq)
    'shop.country': 'MX',                    // a declared related-field filter
    created_at: { gte: 'now-7d' },           // relative to the database clock
  },
  sort: ['-price', 'title'],                 // or '-price,title'
  limit: 20,
  include: ['shop'],
  aggregate: ['by_category'],
  count: ['reviews'],
})
```

| Option | Value |
|---|---|
| `filter` | `{ field: value }` for `eq`, or `{ field: { op: value } }`; ops `eq ne gt gte lt lte between in exists prefix iprefix has`. `in`/`between`/`has` take an array. Values are strings, numbers or booleans. |
| `sort` | one to four keys, `-` for descending |
| `limit`, `offset` | 1–100 (default 25) and 0–10000 |
| `cursor` (or `after`) | the previous page's `next_cursor`, with the same `sort` and filters |
| `search` | full-text search, up to 256 bytes |
| `include`, `aggregate`, `count` | names declared in `query.jsbrain` |

Only shapes the query string cannot express are rejected locally (400): unknown options, a malformed field name, an
unknown operator, a nested value, a comma inside an `in` item. Everything else is decided by the backend: an
undeclared filter or sort answers 422 and the call rejects with `error.status === 422` and `error.errors`, the same
field messages the HTTP API returns. A server function that lets it propagate answers the form with those errors.

## Example: a product listing with keyset pages

`server/resources/products/query.jsbrain`:

```js
<meta server>
export default {
  filter: { category: ['eq', 'in'], price: ['gte', 'lte', 'between'] },
  sort: ['price', 'title'],
}
```

`views/products.brain` (its `load`):

```js
export async function load(ctx) {
  const after = ctx.url.searchParams.get('after') || undefined
  const page = await ctx.backend.resource('products').list({
    filter: { category: ctx.url.searchParams.get('category') || 'tools' },
    sort: ['price'],
    limit: 24,
    ...(after ? { after } : {}),
  })
  return { products: page.items, total: page.total, next: page.next_cursor ?? null }
}
```

```json
{"items":[{"id":"7b1d…","data":{"title":"Hand trowel","price":"12.50","category":"tools"},
  "created_at":"2026-10-30T09:00:00.000Z","updated_at":"2026-10-31T10:05:00.000Z"}],
 "limit":24,"offset":0,"total":31,"next_cursor":"7b2273223a…"}
```

Every record carries `created_at` and `updated_at` (UTC ISO strings), so a page can use `updated_at` as its
`Last-Modified`, a feed date or a sitemap `lastmod`.

## Example: partial update from a form

```js
export async function restock(ctx, { id, stock }) {
  return await ctx.backend.resource('products').patch(id, { stock: Number(stock) })
}
```

An empty object rejects with 422 before any request. A race with another writer rejects with status 409.

## Example: permanent redirect for an old slug

```js
export async function load(ctx) {
  const slug = ctx.params.slug
  const page = await ctx.backend.resource('articles').list({ filter: { slug }, limit: 1 })
  if (page.items.length) return { article: page.items[0] }
  const current = await ctx.backend.slugRedirect('articles', slug)
  if (current) throw redirect('/articles/' + current, 301)
  throw notFound()
}
```

`slugRedirect` returns the current slug for an earlier slug of a record the reader may read, and `null` for the live
slug, an unknown slug or an unreadable record. Slugs are limited to `A–Z a–z 0–9 . _ ~ -`, up to 512 bytes.

## Limits

Request bodies are limited to 64 KiB and responses to 2 MiB; the call shares the request's ten-second deadline and
never retries a write. Tested by `runtime/test/server-native-queries.test.js` (query string mapping, local
validation, PATCH, slug redirects, field errors) and `runtime/test/server-native-resource.test.js` (transport,
cookies, budgets).
