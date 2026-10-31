# Cron schedules (single run per slot)

`server/cron/<name>.jsbrain` enqueues a declared job on a cron schedule. Run `brain jobs cron` in as many processes as
you like: every slot is enqueued exactly once, and the job then follows its [queue policy](queues.md).

`server/cron/daily_summary.jsbrain`:

```js
<meta server>
export default {
  schedule: '0 7 * * *',
  job: 'order_receipts',
  payload: { order_id: 'daily-summary', email: 'shop@example.com' },
  catchUp: 600
}
```

| Key | Required | Meaning |
|---|---|---|
| `schedule` | yes | five fields `minute hour day month weekday` (or six with leading seconds), evaluated in UTC |
| `job` | yes | a job declared under `server/jobs/<job>/` |
| `payload` | no (`{}`) | validated against the job schema when the app loads |
| `catchUp` | no (300) | seconds after its time that a slot is still enqueued, 0..86400 |
| `priority` | no (0) | enqueue priority, -100..100 |

```sh
brain jobs cron          # long-running scheduler; Ctrl+C stops it
brain jobs cron --once   # enqueue the slots due now and exit (for an external timer)
brain jobs work order_receipts
```

```json
{"schemaVersion":1,"event":"cron-enqueued","schedule":"daily_summary","slot":"2026-10-31T07:00:00Z","id":"75e4678a-…"}
```

## Guarantees

- **Single run per slot:** each slot is enqueued with the idempotency key `cron:<name>:<slot>`. The per-queue enqueue
  lock and the unique `(queue, key)` index make concurrent schedulers return the same job id instead of a second job.
- **No late backlog:** after a restart, only the latest slot inside `catchUp` is enqueued; older missed slots are
  skipped, never replayed in a burst. A fresh deploy does not fire a daily job that was due hours ago.
- **Delivery:** the enqueued job is delivered at least once like any other job, so handlers stay idempotent by `jobId`.
- **Validation:** an invalid expression, an unknown job or a payload that does not match the job schema fails
  `brain backend schema` with the file name. Schedules are projected under `cron` in `brain backend schema --json`.

Tested by `crates/brain-backend/tests/queues.rs` (catch-up window, and eight concurrent schedulers creating one job
per slot) and the CLI projection test with [`examples/shop/server/cron`](examples/shop/server/cron).
