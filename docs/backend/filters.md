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
| `geo` ([details](geo.md)) | `near bbox exists` |

`brain backend capabilities` prints this table from the code of the installed version. `num`, `obj` and fields with restricted [visibility](visibility.md) cannot be filtered. `in` takes 1–50 values,
`between` exactly two (inclusive). `exists` takes `true` or `false`; a stored `null` and a missing value both count as not existing.
`ne` keeps records without a value, as action guards do: `stock[ne]=1` also returns a product with no `stock`; add `stock[exists]=true` to leave those out.

## Alternatives: `or`

Filters combine with AND. For field A OR field B, declare a named group; each alternative gets its own index:

```js
export default {
  or: { live: { expires_on: ['gte', 'exists'], source: ['eq'] } },
}
```

```http
GET /api/listings?live.expires_on[gte]=2026-01-01&live.source=legacy&rank[gte]=2
```

The branches you send under `live.` OR with each other, and the group ANDs with everything else (read policy included).
Total, pages, aggregates and search see the same condition. A resource has at most 4 groups of 2..4 local, public fields;
all filters and branches of a request count toward its limit of 16. `brain check` and `brain backend migrate` validate the
declaration. New groups need `brain backend migrate` to build their indexes.

## The reader's own rows: `owner=me`

`?owner=me` limits the list to rows the signed-in reader owns, on top of the read policy, even when the resource is public.
The id comes from the session or key, so a client cannot name another account; any other value answers `422`, and a
request with no identity answers `401`. A filter declared on a field called `owner` or `fields` keeps its meaning.

## Choose the fields

`?fields=title,price` keeps only those keys in each item's `data`; `id`, timestamps, `included` and `counts` stay. It
runs after the read policy and field visibility, so it can only remove what the reader already receives. A field no reader
receives (`private`, `writeonly`) answers `422` like an undeclared name, and a field limited with `visible` is absent for
readers who lack the grant. `include` still follows a reference that `fields` drops. Lists only; a single-record read
keeps its full shape.

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
