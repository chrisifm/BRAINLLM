# Slug history

A resource can mark one unique text field as its slug. When a record's slug changes, the previous value keeps pointing
at the record, so a page can answer a permanent redirect instead of a 404.

## Contract

```js
// server/resources/articles/schema.jsbrain
export default {
  title: 'str!',
  slug: { type: 'str!', unique: true, slug: true },
}
```

The slug field must be a public, unique `str` (it may be `readonly`). At most one per resource.

## Lookup

```http
PUT /api/articles/0f3c…   {"title": "Hello", "slug": "hello-world-2026"}
GET /api/articles/redirects/hello-world
```

```json
{"slug": "hello-world-2026"}
```

| Case | Answer |
|---|---|
| an earlier slug of a readable record | `200 {"slug": "<current>"}` |
| the live slug, an unknown slug, or a deleted record | `404` |
| a record outside the reader's read scope | `404` (slugs are never confirmed to other readers) |

A slug that a record takes again (the same record moving back, or another record) stops redirecting: the live slug
always wins. History is kept in `_brain_slug_history` by a database trigger, so it covers every write path.

Pages use the record's `updated_at` for `lastmod` and validators. Server code calls the lookup with
`await ctx.backend.slugRedirect('articles', slug)` (see [server-queries.md](server-queries.md)).
