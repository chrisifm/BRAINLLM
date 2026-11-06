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
    by_tag: { group: 'tags', sum: ['price'] },
    by_category_tag: { group: ['category', 'tags'], sum: ['price'] },
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
- Grouped aggregates return at most 100 groups, largest first; `min`/`max`/`sum`/`avg` are per group. When a group was cut,
  the response lists that aggregate in `aggregates_truncated`.
- `sum`/`avg` take `int` or `decimal`; `min`/`max` also take `date` and `datetime`; `group` takes a public
  `str`, enum, bool, ref, date, int or `list<str>` field, or an array of 1..4 distinct such fields.
  Fields with restricted [visibility](visibility.md) are rejected at load time.
- A string `group` keeps the existing scalar `key`. An array of field names returns an object key,
  such as `{"category":"tools","tags":"red"}`; booleans, integers and null retain their JSON types.
  Each row contributes once per distinct list element, including to sums and averages. Repeated elements
  never multiply a row's values. An empty or absent list has no element bucket. Scalar missing values
  group under null. At most one list dimension is allowed per aggregate; use separate named aggregates
  instead of a Cartesian product of two lists. Grouping uses complete string values, not truncated search keys.
- Every named aggregate uses the same filtered readable row set as the list. Grouping never grants
  access to another owner's rows or a field hidden by its visibility policy. Several named aggregates
  can be requested together; `limit` and cursors limit the page of rows, not the aggregate's scope.

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
