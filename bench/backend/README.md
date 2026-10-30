# Native backend and actual reference API measurement

Run only against an explicitly selected, locally installed reference application's source.
The harness imports its real Fastify application/migrations/seeds into a disposable database;
it never reads that application's `.env`, starts its existing service or selects a developer database.
No reference source is copied into this repository.

```sh
BRAIN_REFERENCE_API=/absolute/path/to/reference/api \
BRAIN_BIN=/absolute/path/to/frozen/brain \
BRAIN_BENCH_PROFILE=release \
BRAIN_BENCH_OUTPUT=/absolute/path/to/results.json \
node bench/backend/run.mjs
```

`initdb`, `postgres`, `createdb` and `pg_isready` must be on PATH, or set `PG_BIN`.
The reference source must expose its existing `buildApp`, migration and seed modules
and have its own dependencies installed. The harness does not install or update them.
It uses one owned Postgres cluster and separate databases, eight connections per pool,
real loopback HTTP, five warmups, 100 samples and concurrency two per scenario.
Each HTTP failure or rate-limit response rejects the measurement. Each scenario restarts
only owned API processes so default production quotas remain enabled.

The supplied binary is copied into the owned scratch directory before execution; later compiler builds cannot change
the measured executable. Declare its actual debug/release profile explicitly. Debug builds are functional preflights,
not release throughput evidence. The report includes binary SHA256, declared build profile, actual reference source
checksum (and Git commit when one exists), Fastify version, hardware,
dataset projection checksum, completed requests/second and empirical p50/p99/max latency.
Response bodies are consumed and bounded; body values, session cookies and credentials
are excluded from the report. The total operation has a ten-minute deadline; owned
children are terminated/reaped and private databases/uploads removed in `finally`.

These are **operational scenarios**, not equivalent implementations or a framework ranking:

- The existing reference login uses scrypt and loads domain profiles. Native login uses Argon2id.
- Both databases receive the same published job title/body projection. The reference list also
  executes its domain joins/counts; native resource listing returns canonical records.
- The reference accepts a 64 KiB PDF, validates magic bytes and replaces a private CV.
  Native upload accepts a 64 KiB PNG, decodes it, generates responsive variants and retains originals.

A p99 from 100 samples is a descriptive local observation. Repeat controlled runs and
match domain behavior before inferring a production latency or throughput advantage.
The harness never calls AI, mail, payments, cloud storage or Google services.
