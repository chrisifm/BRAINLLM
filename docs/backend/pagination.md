# Pagination (B-15)

Lists support offset pages (`limit` 1–100, default 25; `offset` 0–10000) and keyset cursors for deep or changing
data.

## Keyset cursor

Each page that has more rows returns `next_cursor`. Pass it back unchanged as `cursor` (or its alias `after`) with the
same `sort` and filters:

```http
GET /api/products?sort=price&limit=2
```

```json
{"items": [ … ], "limit": 2, "offset": 0, "total": 5, "next_cursor": "7b2273223a…"}
```

```http
GET /api/products?sort=price&limit=2&after=7b2273223a…
```

The last page has no `next_cursor`. A walk visits every row exactly once even when sort keys tie or are null, and rows
inserted meanwhile do not shift later pages. `total` keeps counting the whole filtered set.

## Rules

| Request | Result |
|---|---|
| cursor from a different `sort` | `422`, restart from the first page |
| `cursor` (or `after`) together with `offset` | `422` |
| `cursor` and `after` together | `422` |
| ranked `search` with a cursor and no `sort` | `422`; add `sort=` to page search results by cursor |

The cursor is opaque (it encodes the sort signature, the last row's sort keys and its id). It is not a secret and
carries no authorization: every page re-applies the reader's scope.
