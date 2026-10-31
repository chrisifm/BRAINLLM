# Totals, aggregates and related counts (B-13)

## Total

Every list response carries `total`: the number of rows matching the same filters and search inside the reader's
scope, independent of `limit`, `offset` and the cursor.

```json
{"items": [ … ], "limit": 25, "offset": 0, "total": 132}
```

## Named aggregates

```js
// server/resources/products/query.jsbrain
export default {
  aggregate: {
    totals: { sum: ['price', 'stock'], avg: ['price'], min: ['price', 'released_on'], max: ['stock'] },
    by_category: { group: 'category', sum: ['price'] },
  },
}
```

```http
GET /api/products?category=tools&aggregate=totals,by_category&limit=1
```

```json
{"items": [ … ], "limit": 1, "offset": 0, "total": 3,
 "aggregates": {
   "totals": {"count": 3, "sum": {"price": "47.00", "stock": 12}, "avg": {"price": "15.6667"},
              "min": {"price": "9.50", "released_on": "2026-02-01"}, "max": {"stock": 7}},
   "by_category": [{"key": "tools", "count": 3, "sum": {"price": "47.00"}}]}}
```

- Decimal sums and averages are exact strings (`numeric`), never floats; an average has the field's scale plus two.
- `int` `min`/`max` are numbers; other values keep their stored string form. An empty set gives `null` values.
- Grouped aggregates return at most 100 groups, largest first.
- `sum`/`avg` take `int` or `decimal`; `min`/`max` also take `date` and `datetime`; `group` takes a filterable field.
  Fields with restricted [visibility](visibility.md) are rejected at load time.

## Counts of related records

```js
export default {
  count: {
    reviews: { resource: 'reviews', field: 'product' },
    five_star: { resource: 'reviews', field: 'product', where: { rating: 5 } },
  },
}
```

```http
GET /api/products?count=reviews,five_star
```

Each row gains `"counts": {"reviews": 2, "five_star": 1}`. Counts apply the related resource's read policy, so a
reader only counts records it could read.
