# Development worker resource failures

The development server retains its 64-connection admission limit. Its accept, watch and request workers use
fallible thread creation. An unavailable startup worker returns an actionable error and releases the owned
listener, watcher/context and native socket worker instead of invoking the CLI panic hook.

If a request worker cannot start, the accepted connection and admission permit remain in a stack-owned pending
request. Dropping the rejected closure sends one best-effort, generic `503` with `Retry-After: 1`, using the original
socket and one write with a 100 ms socket timeout. It adds no duplicate file descriptor, pending queue or per-request
wrapper allocation. A started worker consumes the socket; its permit remains owned until its handler finishes.
Later requests can recover when process/thread capacity becomes available.

`BDEV_THREAD_DISPATCH` diagnostics use the existing bounded dev log ring. Repeated failures emit at most once per
second, plus a final flush of pending counts on shutdown. Each record contains the batch failure count, cumulative
count and a static fix. Request paths, headers, credentials, environment values and raw OS errors are excluded.
Startup errors use `BDEV_THREAD_START`; no system resource limits are changed automatically.

Tests inject failures into one server's thread factory without changing global limits or exhausting the host.
They verify the first `503`, immediate recovery, more failed dispatches than the entire admission budget, complete
count preservation under coalescing, and startup cleanup with watching/native sockets enabled.

The `503` is best-effort: a disconnected peer or unavailable socket write capacity may prevent a complete response.
This change does not prevent kernel OOM termination, allocator aborts, arbitrary callback panics or resource
failures inside external libraries. Production static serving retains its existing separate worker implementation.
