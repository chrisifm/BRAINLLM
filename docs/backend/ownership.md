# Ownership through relations (B-09, B-21)

A policy grant `owner` means "the authenticated user created this row". A grant `owner.<ref>` means "the user owns
the record this row references"; `owner.<ref>.<ref>` follows two references. Paths are limited to two hops and every
hop must be a `ref:` field.

## Contract

```js
// server/resources/products/policy.jsbrain — shop owners manage their products
export default { read: ['public'], create: ['owner.shop'], update: ['owner.shop'], delete: ['owner.shop', 'admin'] }

// server/resources/reviews/policy.jsbrain — authors and the reviewed shop's owner read reviews
export default { read: ['owner', 'owner.product.shop'], create: ['candidate'], update: ['owner'], delete: ['owner'] }
```

## Behavior

- **Lists, counts, aggregates and includes** apply the scope as one SQL predicate
  (`ref_product IN (SELECT id FROM products WHERE ref_shop IN (SELECT id FROM shops WHERE owner = $user))`).
- **Read, update and delete by id** answer `404` when the row is outside the scope, so ids of other tenants are not
  confirmed.
- **Create and update** check the submitted references: a user cannot create a product for a shop they do not own, or
  move one to another shop. That answers `403`.

```http
POST /api/products   (as the owner of shop A)
{"shop": "<shop B id>", "title": "Spade", "price": "20.00"}
```

```json
{"error": {"code": "POLICY_DENIED", "fix": "Sign in with a role permitted by this resource's policy"}, "rid": "…", "schemaVersion": 1}
```

Unknown hops are rejected when the contract loads (`brain backend schema --json`), not at request time.
