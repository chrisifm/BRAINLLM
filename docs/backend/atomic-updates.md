# Atomic updates: guarded increments, compare-and-set and upserts

Read-modify-write from the client (`GET`, change, `PUT`) loses updates under concurrency. Declare the change instead:
the backend locks the row (`SELECT … FOR UPDATE`), checks the guards on the current value, applies the change,
revalidates the full contract and writes, all in one transaction. Concurrent requests queue on the row lock, so none of
them sees a stale value.

## Actions: `server/resources/<r>/actions.jsbrain`

```js
<meta server>
export default {
  restock: { roles: ['admin'], input: { amount: 'int!' },
             when: { status: { ne: 'archived' } }, increment: { stock: '$input.amount' } },
  reserve: { roles: ['admin'], input: { quantity: 'int!' },
             when: { status: 'active', stock: { gte: '$input.quantity' } }, decrement: { stock: '$input.quantity' } },
  reprice: { roles: ['admin'], input: { expected: 'decimal(10,2)!', price: 'decimal(10,2)!' },
             when: { price: '$input.expected' }, set: { price: '$input.price' } }
}
```

`POST /api/<r>/<uuid>/actions/<name>` with the input as the JSON body returns the updated record.

| Key | Meaning |
|---|---|
| `roles` | configured roles or `owner` (caller owns the row) allowed to run the action |
| `input` | typed request fields (`int!`, `decimal(10,2)!`, `str`, ...); unknown keys answer 422 |
| `when` | guards on the locked row: a literal (equals) or `{eq, ne, in, not_in, gt, gte, lt, lte, exists}` |
| `set` | assignments; `null` removes an optional field |
| `increment` / `decrement` | exact deltas on `int`, `num` or `decimal(p,s)` fields (decimals use exact string arithmetic) |

Values may use `$input.<field>`, `$user.id|role|email`, `$now` and `$now±N{s,m,h,d}`. A field appears in only one of
`set`, `increment` and `decrement`. The result is checked against types and `constraints.jsbrain`, so a decrement
below a `min` answers 422 and never reaches the database.

| Outcome | Status |
|---|---|
| guard not met on the current row | 409 `GUARD_FAILED`, e.g. `{"stock":"Must be >= 3"}`; nothing written |
| result breaks a type or constraint | 422 `INVALID_INPUT` |
| role not allowed | 403 · unknown record or action: 404 |

### Example: no overselling

Stock 10, 30 clients each reserve 1 at the same time (through the [`place_order` operation](transactions.md)):
exactly 10 answer 200, 20 answer 409 `GUARD_FAILED`, and the stock ends at 0. 20 concurrent `restock` calls of 1
add exactly 20.

### Example: compare-and-set

```http
POST /api/shop_products/0d6c…/actions/reprice

{"expected": "0.00", "price": "12.00"}
```

The first request returns the record with `"price":"12.00"`. A second client that also read `0.00` gets
`409 {"error":{"code":"GUARD_FAILED","fields":{"price":"Must equal 0.00"}}}` and must reload before deciding again.

## Upsert (insert if absent)

Any resource with a single `unique: true` field or a composite unique set in `constraints.jsbrain` gets
`POST /api/<r>/upsert`:

```json
{"key": ["code"], "data": {"title": "Write docs", "code": "T-1"}, "on_conflict": "ignore"}
```

```json
{"record": {"id": "…", "data": {"title": "Write docs", "code": "T-1", "points": 0}, "…": "…"}, "created": true}
```

`key` must be exactly one declared unique set. The insert uses Postgres `ON CONFLICT … DO NOTHING` on the unique
index, so 12 concurrent upserts of one key create exactly one row; the others return that row with `created:false`.
`on_conflict: "update"` (default) merges the sent fields into the existing row (fields not sent keep their values) and
requires the `update` grant on that row (403 otherwise); `"ignore"` returns the existing row unchanged. The `create`
grant is always required.

Resources with a mandatory before-hook keep their single-record endpoints only: actions and declarations against them
are rejected at load, and upsert answers 422 `BEFORE_HOOK_REQUIRES_SINGLE_WRITES`.

Tested by `crates/brain-backend/tests/domain_writes.rs`
(`postgres_atomic_actions_and_upserts_are_race_safe_under_concurrent_requests`) against the contracts in
[`examples/shop`](examples/shop/server).
