# Search (B-14)

A resource opts fields into search (see [BACKEND.md](../BACKEND.md)). `?search=` (at most 256 bytes and 16 words)
matches those fields two ways and merges the candidates: accent-insensitive full-text search (`websearch_to_tsquery`)
and trigram word similarity for typos and partial words. Ranking is full-text rank, then similarity, then id.

```http
GET /api/products?search=garden%20trowel&category=tools&limit=10
```

Search combines with declared [filters](filters.md), `total`, [aggregates](aggregates.md) and the reader's
[ownership](ownership.md) scope; every candidate branch carries the same authorization predicate.

- Without `sort`, results are ranked by relevance and paged with `offset`.
- With `sort`, results follow that order and can be paged with a [cursor](pagination.md).
- Fields with restricted [visibility](visibility.md) cannot be searchable.

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
