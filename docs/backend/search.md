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

Not yet configurable: the text-search language (today the language-neutral `simple` configuration, so there is no
stemming), and a degraded mode for servers without `pg_trgm`. Both are tracked as remaining B-14 work.
