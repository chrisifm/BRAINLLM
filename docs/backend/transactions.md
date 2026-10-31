# Multi-resource transactions (operations)

An operation is a server-side write that spans several records or resources. Each file
`server/operations/<name>.jsbrain` declares typed input and ordered steps. `POST /_brain/operations/<name>` runs every
step inside ONE Postgres transaction: the result is committed only when every step succeeds, and any failure (guard,
validation, missing record, unique or check constraint) rolls back all the steps. No application code runs: steps are
literal declarations checked when the app loads (`brain backend schema` fails with the file name and a fix).

## Example: reserve stock and create an order

Full app contracts: [`examples/shop/server`](examples/shop/server). The product action used by the first step
(`server/resources/shop_products/actions.jsbrain`):

```js
<meta server>
export default {
  reserve: { roles: ['admin'], input: { quantity: 'int!' },
             when: { status: 'active', stock: { gte: '$input.quantity' } },
             decrement: { stock: '$input.quantity' } }
}
```

`server/operations/place_order.jsbrain`:

```js
<meta server>
export default {
  roles: ['candidate', 'admin'],
  input: { product_id: 'str!', quantity: 'int!' },
  steps: [
    { as: 'product', action: 'shop_products.reserve', id: '$input.product_id', input: { quantity: '$input.quantity' } },
    { as: 'order', create: 'shop_orders', data: { product: '$steps.product.id', quantity: '$input.quantity', pay_by: '$now+3d' } }
  ],
  result: ['order', 'product']
}
```

```http
POST /_brain/operations/place_order
content-type: application/json

{"product_id": "0d6c…", "quantity": 2}
```

```json
{"operation":"place_order","result":{
  "order":{"id":"5f2a…","owner":"<caller>","data":{"product":"0d6c…","quantity":2,"status":"pending","pay_by":"2026-11-03"},"created_at":"…","updated_at":"…"},
  "product":{"id":"0d6c…","owner":"…","data":{"name":"Mug","sku":"MUG-1","stock":98,"price":"9.50","status":"active"},"created_at":"…","updated_at":"…"}}}
```

If the second step fails, the reservation is undone too:

| Request | Response | Database afterwards |
|---|---|---|
| quantity 60 (order check `max: 50`) | 422 `INVALID_INPUT`, `{"order.quantity":"Must be <= 50"}` | stock unchanged, no order |
| quantity larger than the stock | 409 `GUARD_FAILED`, `{"product.stock":"Must be >= 99"}` | unchanged |
| unknown product id | 404, `{"product":"Not found"}` | unchanged |
| `"quantity":"2"` | 422, `{"input.quantity":"Expected an integer JSON number …"}` | unchanged |

Error field keys start with the step name (`order.quantity`), so the client knows which step failed.

## Steps

| Step | Needs | Effect |
|---|---|---|
| `read: 'r'` | `id` | locks the row (`FOR UPDATE`) and returns it to later steps |
| `create: 'r'` | `data` | inserts with the caller as owner; contract and state rules apply |
| `update: 'r'` | `id`, `data` | PATCH semantics: only the listed fields change |
| `upsert: 'r'` | `data`, `key` | insert, or merge into the row with the same declared unique key |
| `delete: 'r'` | `id` | deletes the locked row |
| `action: 'r.name'` | `id`, `input?` | runs a declared [atomic action](atomic-updates.md) |
| `transition: 'r.name'` | `id`, `input?` | runs a declared [state transition](state-machines.md) |

Values may reference `$input.<field>`, `$user.id|role|email`, `$now`, `$now+7d` / `$now-2h` (units `s m h d`; the
transaction timestamp, formatted for `date` or `datetime` fields), and `$steps.<earlier step>.id|owner|data.<field>`.
`$$` escapes a literal `$`. Add `owned: true` to a step to require that the caller owns the target row (404
otherwise).

## Authority and limits

- `roles` lists configured role names (not `owner` or `public`); the caller must have one of them. The operation's
  roles replace the resource policies for its steps, so keep each operation narrow and use `owned: true` for
  per-user rows.
- Resources with a mandatory before-hook cannot be a step target (the contract is rejected at load); their writes stay
  on the single-record endpoints.
- 1–32 steps, at most 64 operations, input of up to 32 typed fields (`int!`, `str`, `decimal(10,2)!`, ...).
- After-write hooks of every step enqueue in the same transaction, and live subscribers hear about the changes only
  after commit.

OpenAPI documents each operation as `operation_<name>` with its input schema, the result schema per step and the
declared steps (`x-brain-steps`). `brain backend schema --json` includes the loaded contracts under `domain`.

Tested by `crates/brain-backend/tests/domain_writes.rs` (`postgres_operations_commit_every_step_or_roll_all_of_them_back`),
which loads this example through `crates/brain-backend/tests/shop_contracts.json`; a brain-cli test keeps that file
equal to what the CLI loads from `docs/backend/examples/shop`.
