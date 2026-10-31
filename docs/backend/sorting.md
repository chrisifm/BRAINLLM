# Sorting (B-12)

## Contract

```js
// server/resources/products/query.jsbrain
export default {
  sort: ['price', 'title', { field: 'stock', nulls: 'first' }, 'released_on'],
  default_sort: ['-created_at'],
}
```

Sortable fields are public `int`, `decimal`, `date`, `datetime`, `str`, `email`, `enum` and `bool` fields, plus
`created_at` and `updated_at`. `nulls` is `first` or `last` (default `last`) and applies in both directions.

## Request

```http
GET /api/products?sort=-price,title
```

`-` means descending. One to four distinct keys. Ties always break by `id`, so the order is total and stable across
pages. Without `sort`, `default_sort` applies; without either, rows are newest first (`-created_at`).

Text sorts by its first 256 bytes in byte order (`COLLATE "C"`), independent of the server locale. Decimals sort
numerically, not as strings.

A field that is not declared answers `422` with the list of sortable fields.
