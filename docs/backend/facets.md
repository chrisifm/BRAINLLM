# Facets

Counts of each value of a field over the rows the reader may list, for filter sidebars.

```js
// server/resources/<name>/query.jsbrain
<meta server>
export default { filter: { status: ['eq'] }, facets: ['status', 'tags'] }
```

`GET /api/<name>?facets=status,tags` adds:

```json
{"facets": {"status": [{"key": "open", "count": 12}, {"key": "done", "count": 3}], "tags": [{"key": "a", "count": 2}]}}
```

- Up to 8 public fields of kind `str`, `enum`, `bool`, `ref`, `date`, `int` or `list<str>`; a restricted field cannot be declared.
- The reader's scope, search and other filters apply, so a row they cannot read is never counted.
- A field's own filter does not narrow its facet, so the other values stay available to pick. With typo-tolerant search
  fallback the candidates were already narrowed by every filter, so counts may be lower in that case.
- A row without a value counts under `key: null`; a `list<str>` counts a row once per distinct element.
- The 100 most frequent values are returned; `facets_truncated` names any field cut at that limit.
- A field named `facets` with a declared filter keeps its filter, and `?facets=` then means that filter.

From a server function: `ctx.backend.<name>.list({ facets: ['status'] })`.
