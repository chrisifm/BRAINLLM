# Credits

A balance per subject (a user or an organization) and an append-only ledger that explains it.

| Route | Who | Does |
|---|---|---|
| `POST /_brain/credits/{subject}` | global `admin` | Applies `{ delta, reason, idempotency_key }`: positive grants, negative debits |
| `GET /_brain/credits/{subject}` | that user, the organization's managers, `admin` | Returns `balance` and ledger `items`, newest first (`before`, `limit`) |

```json
{"id": 41, "delta": -3, "balance": 97, "replayed": false}
```

Each change locks the subject's balance row, so concurrent debits serialize and the balance cannot go below zero: a debit that does
not fit is 409 `CREDITS_INSUFFICIENT` and changes nothing. A database check refuses a negative balance as well.

The `idempotency_key` is required and unique per subject. Sending it again returns the first result with `replayed: true`
and charges nothing; the same key with another `delta` or `reason` is 409 `CREDITS_KEY_REUSED`.

The ledger refuses `UPDATE`, `DELETE` and `TRUNCATE`. Amounts are integers up to 9007199254740991; choose your smallest unit.
Direct changes require an administrator's session. To spend on a user's behalf, declare an operation with a `debit` step ([transactions.md](transactions.md#charging-credits)).
Payments, prices and expiry are outside this feature.
