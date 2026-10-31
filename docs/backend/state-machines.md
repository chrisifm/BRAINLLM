# State machines (declared transitions)

A resource may declare one `enum` field as its state. After that, the field starts in a declared initial value and
changes only through named transitions: `POST /api/<r>/<uuid>/transitions/<name>`. Create, PUT, PATCH, batch and
operation `update` steps that try to change it answer 422.

`server/resources/shop_orders/transitions.jsbrain`:

```js
<meta server>
export default {
  field: 'status',
  initial: ['pending'],
  transitions: {
    pay: { from: ['pending'], to: 'paid', roles: ['owner', 'admin'], set: { paid_at: '$now' } },
    ship: { from: ['paid'], to: 'shipped', roles: ['admin'] },
    cancel: { from: ['pending', 'paid'], to: 'cancelled', roles: ['owner', 'admin'],
              input: { reason: 'str!' }, set: { reason: '$input.reason' } }
  }
}
```

The `field` must be declared as `enum(...)` in the schema; `from`, `to` and `initial` must be values of that enum.
Each transition accepts the same keys as an [action](atomic-updates.md): `roles`, typed `input`, extra `when`
guards and side effects in `set` / `increment` / `decrement`. Side effects are written in the same transaction as the
state change; for example, `due_on: '$now+14d'` on a `date` field stores today plus 14 days.

```http
POST /api/shop_orders/5f2a…/transitions/pay
content-type: application/json

{}
```

```json
{"id":"5f2a…","owner":"…","data":{"product":"0d6c…","quantity":1,"status":"paid","paid_at":"2026-10-31T09:12:44.120Z"},"…":"…"}
```

Running `pay` again:

```json
{"error":{"code":"GUARD_FAILED","fields":{"status":"Transition pay is allowed from pending"},"message":"…"},"rid":"…"}
```

| Case | Status |
|---|---|
| current state not in `from` | 409 `GUARD_FAILED` |
| role not allowed (`owner` = caller owns the row) | 403 |
| missing or wrongly typed input | 422 |
| unknown transition or record | 404 |
| `POST /api/shop_orders` with `status: "shipped"` | 422 (`Must start as one of: pending`) |
| PUT/PATCH/batch changing `status` | 422 (`Change this field only through a declared transition: cancel, pay, ship`) |

The row is locked during the transition, so two concurrent transitions from the same state (for example `ship` and
`cancel` on a paid order) produce exactly one 200 and one 409. Operations can run a transition as a step
(`{ as: 'paid', transition: 'shop_orders.pay', id: '$input.order_id' }`), together with other writes in one
transaction.

OpenAPI documents each transition as `<r>_transition_<name>` with `x-brain-transition` and `x-brain-state-field`.

Tested by `crates/brain-backend/tests/domain_writes.rs` (`postgres_state_transitions_are_the_only_way_to_change_state`)
with the contracts in [`examples/shop`](examples/shop/server).
