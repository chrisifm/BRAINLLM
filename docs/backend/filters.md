# Filters (B-11)

List endpoints accept only the filters a resource declares in `query.jsbrain`. Undeclared, repeated or malformed
parameters answer `422` with one message per parameter, so a typo is never silently ignored.

## Contract

```js
// server/resources/products/query.jsbrain
export default {
  filter: {
    price: ['eq', 'gte', 'lte', 'between'],
    stock: ['gt', 'exists'],
    category: ['eq', 'in'],
    title: ['prefix', 'iprefix'],
    released_on: ['lt', 'gte'],
    created_at: ['gte'],
    'shop.country': ['eq'],
  },
}
```

## Request syntax

`field=value` means `eq`; any other operator is `field[op]=value`. Lists are comma separated.

```http
GET /api/products?category[in]=tools,garden&price[between]=10,25&stock[gt]=0
GET /api/products?title[iprefix]=hand&released_on[gte]=2026-01-01
GET /api/products?created_at[gte]=now-7d
```

| Field type | Operators |
|---|---|
| `int`, `decimal(p,s)`, `date`, `datetime` | `eq ne gt gte lt lte between in exists` |
| `created_at`, `updated_at` | `eq gt gte lt lte between` |
| `str`, `email`, `enum(...)` | `eq ne in prefix iprefix exists` |
| `bool` | `eq exists` |
| `ref:<resource>` | `eq ne in exists` |
| `list` | `has exists` |

`num`, `obj` and fields with restricted [visibility](visibility.md) cannot be filtered. `in` takes 1–50 values,
`between` exactly two (inclusive). `exists` takes `true` or `false`.

## Values

Values are validated with the field's exact type before they reach SQL: decimals are compared as exact `numeric`,
dates and datetimes as calendar values, enums against their listed values. Date and datetime filters also accept a
time relative to the database clock: `now`, `now-7d`, `now+1h` (units `m`, `h`, `d`, `w`).

```json
{"error": {"code": "INVALID_INPUT", "fields": {"price[gte]": "Expected a decimal(10,2) string such as \"19.50\""},
  "fix": "Apply the field errors and send only fields declared in schema.jsbrain"}, "rid": "…", "schemaVersion": 1}
```

Every filter is a bound parameter. Declared filters get an expression index at migration time.

## Combined with everything else

Filters narrow the same row set used by [sorting](sorting.md), `total`, [aggregates](aggregates.md),
[pagination](pagination.md) and [search](search.md), always inside the reader's [ownership](ownership.md) scope.
