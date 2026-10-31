# Search (B-14)

A resource opts fields into search (see [BACKEND.md](../BACKEND.md)). `?search=` (at most 256 bytes and 16 words)
matches those fields with accent-insensitive full-text search (`websearch_to_tsquery`). Trigram word similarity for
typos runs only when full text finds nothing. Text under 3 characters matches by prefix, and text made only of stop
words finds nothing. Full-text ranking is rank, then id; typo ranking adds similarity.

```http
GET /api/products?search=garden%20trowel&category=tools&limit=10
```

Search combines with declared [filters](filters.md), `total`, [aggregates](aggregates.md) and the reader's
[ownership](ownership.md) scope; every candidate branch carries the same authorization predicate.

- Without `sort`, results are ranked by relevance and paged with `offset`.
- With `sort`, results follow that order and can be paged with a [cursor](pagination.md).
- Fields with restricted [visibility](visibility.md) cannot be searchable.

## Totals and slow typo searches

`total` is exact for full-text, prefix and filter results. Typo matching looks at the first 256 characters of each
searchable field, has a 400 ms budget and keeps at most 500 candidates. The response marks the two cases where it is not
exact:

| Field | Meaning |
|---|---|
| `total_approximate: true` | `total` counts the typo candidates that were kept, not every possible match |
| `degraded: true` | Typo matching ran out of budget: the page may be empty and a `search-degraded` event is logged |

A search that cannot finish answers `503 DATABASE_BUSY` with `Retry-After`; retrying is safe because nothing changed.
Invalid input (a NUL character, a malformed cursor) answers `422`.

Rows stored before the 256-character bound are rebuilt once by the next `brain backend migrate`, in one transaction
with no statement deadline and without touching `updated_at`. Until then, typo searches on long text can answer
`degraded`; full-text searches are unaffected. No `brain doctor` check is needed because the bound applies to every row.

## Language and weights

By default search uses the language-neutral `simple` configuration (no stemming) and fixed weights (`title`/`name` A,
`summary` B, everything else D). A resource can choose a built-in Postgres text-search configuration and per-field
weights in `query.jsbrain`:

```js
<meta server>
export default {
  search: { language: 'english', weights: { body: 'A', title: 'B' } },
}
```

With `english`, `?search=runs` also finds "running" and stop words such as "the" are not indexed. `language` must be
one of the configurations Postgres ships (`simple`, `english`, `spanish`, `portuguese`, `french`, `german`, ...); weights
are `A`-`D` and may only name fields listed for search. Either mistake is a contract error (B0735) in `brain check` and stops `brain backend migrate`.

Changing the language or the weights is a normal `brain backend migrate`: the search trigger is replaced and existing
rows are reindexed in place, without touching `updated_at`. Resources without a `search` block keep their exact
trigger, so upgrading does not reindex them. Tested by `crates/brain-backend/tests/data_queries.rs`
(`search_language_weights_and_legacy_numeric_ids`).

Not yet available: a degraded mode for servers without `pg_trgm` (the extension stays required).
