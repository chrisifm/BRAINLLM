# Queue policies: priorities, retries, dead letters, long jobs and drain

Every job queue (`server/jobs/<name>/`) may declare `queue.jsbrain`. Every key is optional; an absent file keeps the
defaults shown in the table.

`server/jobs/order_receipts/queue.jsbrain`:

```js
<meta server>
export default {
  priority: 10,
  attempts: 5,
  backoff: { type: 'exponential', delay: 5, max: 600 },
  timeout: 900,
  lease: 60,
  drain: 30,
  concurrency: 4
}
```

| Key | Default | Range | Meaning |
|---|---|---|---|
| `priority` | 0 | -100..100 | default enqueue priority; higher runs first, equal priority stays FIFO |
| `attempts` | 3 | 1..10 | attempts before the job becomes `dead` |
| `backoff` | `{ type: 'exponential', delay: 2, max: 300 }` | seconds, 0..86400 | `fixed` waits `delay`; `exponential` waits `delay * 2^(attempt-1)`, capped at `max` |
| `timeout` | 60 | 1..3600 s | time one attempt may run before it is cancelled and retried |
| `lease` | 90 | 10..600 s | visibility timeout; the worker renews it every third of the lease while the handler runs |
| `drain` | 0 | 0..600 s | on shutdown, stop claiming and let in-flight jobs finish for this long |
| `concurrency` | 2 | 1..8 | workers per process (`--concurrency` overrides) |
| `runAs` | `key` | `key` or `system` | identity of `ctx.backend`: `key` uses `BRAIN_JOBS_API_KEY`, `system` uses `BRAIN_JOBS_SYSTEM_KEY` ([job-backend.md](job-backend.md#system-identity)) |

Invalid values fail `brain backend schema` with the file name and the rule, for example
`server/jobs/mail/queue.jsbrain: invalid queue policy; fix: lease: 10..600 seconds`. The policy is projected under
`queues.<name>` in `brain backend schema --json`.

## Commands

```sh
brain jobs enqueue order_receipts --payload '{"order_id":"A-17","email":"ana@example.com"}' --key receipt-A-17
brain jobs enqueue order_receipts --payload '{"order_id":"A-18","email":"bo@example.com"}' --priority 50 --delay 120
brain jobs work order_receipts
brain jobs stats order_receipts
brain jobs stats                                 # every queue: pending, oldest wait, live workers
brain jobs dead order_receipts
brain jobs retry order_receipts --id 6b1f…        # one dead job; without --id, up to 1000
```

```json
{"schemaVersion":1,"stats":{"queue":"order_receipts","pending":3,"delayed":1,"running":2,"succeeded":40,"dead":1}}
{"schemaVersion":1,"dead":[{"id":"6b1f…","queue":"order_receipts","attempts":5,"maxAttempts":5,"priority":10,"lastError":"Handler failed or timed out; inspect the queue handler.jsbrain","finishedAt":"2026-10-31T01:02:03.4+00:00"}]}
{"schemaVersion":1,"revived":1}
```

## Wake-up

A worker takes the next job as soon as a slot frees, and sleeps only when the queue is empty. An idle worker is woken by
`NOTIFY` on `_brain_jobs_ready`, sent when an enqueue commits, about 50 ms from enqueue to finished for an instant
handler. A poll every second (every 250 ms if the listening connection is lost) covers delayed jobs, expired leases and
a connection pooler that cannot `LISTEN`. `NOTIFY` takes a brief global commit lock, so an application with thousands of
enqueues per second should measure it.

A job ends when its handler settles: the runner process exits 100 ms later even if a handle is still open, so await
everything the job must finish.

## Is anything running it?

`brain backend serve` and `brain start` do not process queues. Each queue needs its own `brain jobs work <queue>`
process; until one runs, jobs wait in Postgres. To make that visible:

| Where | What it says |
|---|---|
| `brain doctor` | B0765 per queue with no worker reporting to this database in the last 45 s (a worker in another process counts; one on another database does not, and `brain doctor` checks the development database), B0767 for schedules with no scheduler, B0768 for active jobs in a queue with no declaration. Warnings: a queue may be worked on another machine |
| `brain backend serve` | a `queue-workers` line at startup listing each queue, then `queue-stalled` every 60 s (`BRAIN_QUEUE_CHECK_SECONDS`, 5 to 3600) for a queue with due jobs and no worker, or with a job waiting 5 minutes or more. Counts and ages only, never a payload value |
| `brain start`, `brain run` | the same warnings once at startup, on a background thread. The interactive `brain run` board omits them; `brain doctor` covers that case |
| `brain jobs stats` | `pending`, `oldestWaitSeconds`, `workers`, `declared` |

A worker reports every 15 s to `_brain_workers` and counts as live for 45 s, so a killed worker disappears within a
minute. When the database cannot answer, the check reports B0766 rather than guessing: stored jobs are safe and wait,
but no worker can claim them and no new job can be enqueued until it returns.

For a small deployment, `brain backend serve --with-workers` starts one `brain jobs work` child per declared queue and
`brain jobs cron` when there are schedules. A child that dies restarts after 1 to 30 s, a stopped server lets each child
drain, and a child stops by itself if its server is killed. Use separate processes when workers must scale apart from
the API.

## Semantics

- **Idempotency:** `--key` (or `Enqueue { key }`) returns the existing job id for a repeated key on the same queue.
- **Delay:** `--delay` seconds (up to 30 days) keeps the job `pending` but not due; `stats` counts it as `delayed`.
- **Dead letters:** an exhausted job stays `dead` with a generic error (never the handler message or the payload).
  `retry` moves it back to `pending` with fresh attempts and respects the 10000 active jobs per queue capacity.
- **Long jobs:** a handler may run up to `timeout` (one hour at most) even when that is longer than `lease`. The worker
  renews the lease while the handler runs. If the lease is lost anyway (for example the process froze), the worker
  abandons the attempt without acknowledging it, so a newer owner is never overwritten.
- **Drain:** Ctrl+C (or the shutdown signal of an embedding server) stops new claims at once. In-flight jobs get `drain`
  seconds to finish and are acknowledged normally; after that they are released for retry. Delivery is still at least
  once, so handlers must stay idempotent by `jobId`.

Rust embedders use the same policy through `brain_backend::jobs::QueuePolicy`, `Queue::work_with`,
`Queue::enqueue_with`, `Queue::dead`, `Queue::retry_dead`, `Queue::stats` and `Queue::heartbeat`.

Tested by `crates/brain-backend/tests/queues.rs` (priorities and delays, dead letters and retry, a 14-second job with a
10-second lease delivered exactly once, drain with and without grace) and by the CLI projection test with
[`examples/shop/server/jobs/order_receipts`](examples/shop/server/jobs/order_receipts).
