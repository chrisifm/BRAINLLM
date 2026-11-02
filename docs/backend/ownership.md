# Ownership through relations (B-09, B-21)

A policy grant `owner` means "the authenticated user created this row". A grant `owner.<ref>` means "the user owns
the record this row references"; `owner.<ref>.<ref>` follows two references. Paths are limited to two hops and every
hop must be a `ref:` field.

## Contract

```js
// server/resources/products/policy.jsbrain — shop owners manage their products
export default { read: ['public'], create: ['owner.shop'], update: ['owner.shop'], delete: ['owner.shop', 'admin'] }

// server/resources/reviews/policy.jsbrain — authors and the reviewed shop's owner read reviews
export default { read: ['owner', 'owner.product.shop'], create: ['user'], update: ['owner'], delete: ['owner'] }
```

## Behavior

- **Lists, counts, aggregates and includes** apply the scope as one SQL predicate
  (`ref_product IN (SELECT id FROM products WHERE ref_shop IN (SELECT id FROM shops WHERE owner = $user))`).
- **A read with no grant for this identity** answers `401 SIGN_IN_REQUIRED` to an anonymous caller and
  `403 POLICY_DENIED` to a signed-in one, for any id.
- **Read by id** answers `404` when the row is outside the scope, so ids of other tenants are not confirmed.
- **A write the policy does not grant** on an existing row answers `403 POLICY_DENIED` when the writer may read that
  row, and exactly like a missing row when they may not. This holds for update, patch, delete, batch, actions,
  transitions and owned operation steps. A role with no grant for the operation at all answers `403` for any id, so
  that answer confirms nothing either.
- **Create and update** check the submitted references: a user cannot create a product for a shop they do not own, or
  move one to another shop. That answers `403`.

```http
POST /api/products   (as the owner of shop A)
{"shop": "<shop B id>", "title": "Spade", "price": "20.00"}
```

```json
{"error": {"code": "POLICY_DENIED", "fix": "The declared policy does not grant this operation to this identity (rules evaluated: role, owner, owner.<ref> and row conditions); sign in if anonymous, otherwise use an account those rules allow"}, "rid": "…", "schemaVersion": 1}
```

Unknown hops are rejected when the contract loads (`brain backend schema --json`), not at request time.

## Row conditions on read grants

Any read grant can carry a `where`: the grant only covers rows that match. Grants are OR-ed, so a row is readable when
any grant covers it.

```js
// server/resources/jobs/policy.jsbrain — visitors read published jobs, the shop's owner and admins read every job
export default {
  read: [{ public: { where: { status: 'published' } } }, 'owner.shop', 'admin'],
  create: ['member', 'admin'], update: ['owner.shop', 'admin'], delete: ['admin'],
}
```

- A condition compares a declared scalar field with a static value, a list of values (any of) or `null` on an optional
  field, and always runs as bound SQL. An unknown field, or a value the field's type rejects, fails contract validation
  with a `fix`. `where` on `create`, `update` or `delete` is rejected.
- The same scope applies to lists, totals, filters, sort, cursors, search, aggregates, counts, includes, relational
  filters, get by id, slug history, sockets, `related` file buckets, API keys and `ctx.backend`.
- A hidden row answers exactly like a row that does not exist: `404` with the same body, and no `304`. A filter through
  a relation only matches parents its reader may read.
- A write never returns a row its author may not read: the response keeps the `id` and sends `data: {}`.

One evaluator (`ownership::row_allowed`) decides every by-id read and every `related` file, so those answers cannot
drift apart.

### What a write can still confirm

Holding a write grant confirms facts the write needs: a `409 UNIQUE_CONFLICT` names the field (never the row), an upsert
answers `created: false`, and `REFERENCE_NOT_FOUND` or `GUARD_FAILED` describe the submitted input. Keep unique fields
of hidden rows away from roles that should not learn them by not granting them `create`.

## Warning B0749 and intentional public rows

`brain check` warns when `read: ['public']` has no condition on a resource with a state field (an `enum`, or a field
such as `status`, `published` or `visibility`). Add a condition, or declare that every row is public on purpose:

```js
export default { read: ['public'], expose_all_rows: true, create: ['admin'], update: ['admin'], delete: ['admin'] }
```

## Server-only resources

`api: false` removes a resource from the public surface: every `/api/<resource>` route answers like an unknown
resource, OpenAPI omits it, and no API-visible resource can include, count or filter through it. Omitted grant lists
default to `admin` and `public` is rejected.

```js
// server/resources/ledgers/policy.jsbrain
export default { api: false }
```

Server code reaches it with an API key whose scope names it (`ledgers:read`, never the `*:*` wildcard) through
`ctx.backend.service('ledgers')`; see [server queries](server-queries.md).
