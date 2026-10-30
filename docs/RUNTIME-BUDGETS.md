# Runtime execution and resource budgets

Production and development compiled BrainScript functions have execution checkpoints. Function entry and each `for...of` iteration charge a shared synchronous execution budget. Nested calls share the current budget; recursion is rejected before the native stack overflows. Generated `try/finally` releases frames on return, throw, break and rejection. An exceeded budget poisons that execution: catching the error does not restore its remaining allowance. A subsequent independent event starts a new execution budget.

`runtime/brain-budget.js` is a separate internal runtime module. It is automatically imported by generated checkpoint calls, without author imports or a console loop. The small `brain.js` hook runs before owned signal construction and before list rows are materialized. Existing B0902 effect-cycle cutoff, B0903 component recursion checks, B0905 unbounded-growth warnings, B0906 missing cleanup warnings, B0907 probe growth checks and B0908 component circuit breakers remain applicable.

| Guard | Default maximum | Scope |
| --- | ---: | --- |
| Execution checkpoints | 1,000,000 | One execution, including resumptions |
| Active function depth | 128 | One active execution |
| Synchronous elapsed work | 200 ms | Active phases, checked at checkpoints |
| List rows | 100,000 | One list effect |
| Signal allocations | 100,000 | One disposable owner's lifecycle or effect run |
| Explicit resources | 1,024 | One disposable owner |
| Helper watchdog pending calls | 32 | One explicitly supplied owned worker |
| Helper watchdog deadline | 30,000 ms | One pending call |

These are internal finite safety ceilings. Helper options can lower a ceiling; zero, negative, non-finite values and attempts to raise it cannot disable the ceiling. Existing `<meta budget>` controls the separate B0908 post-render circuit breaker, not these execution ceilings. No new author-facing configuration syntax is implied. The RPC worker wrapper has its own bounded capacity, deadline and cancellation contract; its documented defaults take precedence over the optional helper watchdog defaults above.

## Await, ownership and disposal

Generated await uses `budgetSuspend(frame, value)` and `budgetResume(frame, await outcome)`. Suspension removes the active frame before waiting. The settled outcome carries either the original value or the original rejection; resume restores execution state before returning or throwing. IO waiting does not consume the synchronous elapsed-work allowance. JavaScript still measures elapsed synchronous phases rather than operating-system CPU time; GC and scheduling can affect that measure. The settled-outcome helper can add a microtask to continuation scheduling.

Component setup captures its actual reactive owner once. Generated callbacks retain this owner even when invoked by delegated event handlers, where the global reactive owner is absent. Modules without a component owner use execution budgets but do not acquire component lifecycle quotas for shared module-level signals. Function parameter default expressions are evaluated by JavaScript before function-entry checkpoints.

`budgetClaim(owner, kind, units, dispose, meta)` reserves a bounded quota before acquisition and returns an idempotent release. Disposal releases every reservation and invokes each registered disposer once. Its cleanup is a child reactive root, preserving the existing single cleanup callback contract. A live effect rerun discards its old ledger and creates a fresh one; a disposed owner cannot acquire resources. Zero-unit claims still consume bounded disposal bookkeeping capacity.

`budgetWatchWorker(worker, rejectAll, meta, options)` is available to wrappers that create a worker exclusively. `track(id)` reserves a bounded pending slot and returns an idempotent completion function that clears its deadline. Deadline or disposal clears all timers, terminates only that supplied worker and rejects its pending work. It does not discover, enumerate or terminate other tabs' workers. A wrapper must attach watchdog disposal to its own lifecycle; this helper alone does not establish ownership.

## Structured feedback and practical limits

Budget exceptions carry stable `code`, `kind: 'budget'`, `budget`, `limit`, bounded source `file`/`component`, one-based `line`/`column` when a generated checkpoint is active, and a mechanical `fix` direction. B0910 identifies execution/deadline exhaustion; B0911 identifies resource quota exhaustion. An allocation outside any active compiled checkpoint can have unknown source location. Messages contain no application values, payloads, source excerpts or credentials. Existing error boundaries and the structured error collector report the exception; this module adds no console output.

B0912 warns at an explicit foreign `.js`, `.mjs` or `.cjs` browser import: those bodies have no compiler checkpoints. Move untrusted or CPU-heavy work to a bounded owned worker with cancellation, deadline and transferables. Worker modules and `<meta server>` files do not receive a browser-main-thread warning. The diagnostic makes no automatic source edit and does not imply a complete transitive dependency audit.

This is prevention at known checkpoints and allocation sites, not a cross-browser heap-size predictor. Native operations, foreign JavaScript, getters, proxy traps and an iterator's own `next()` can block before the next checkpoint. A main-thread timer cannot interrupt a synchronous native call. Building an array, string, image or decoded payload may allocate memory before assigning it to a signal or passing it to a list. List and signal quotas bound owned runtime amplification, not every application allocation. Virtualize large lists with the existing kit Table, paginate data, bound caches and queues, transfer large binary buffers and isolate risky computation in a worker. An owned worker deadline can terminate that worker, but cannot recover memory already exhausted by unrelated main-thread code.

## Verification

Run `node --test runtime/test/budget.test.js`, `cargo test -p brain-codegen --test runtime_budgets -j2`, and `cargo test -p brain-check --test foreign_execution -j2`, followed by the repository gates. Tests cover real core allocation hooks, no rows created above the ceiling, owner reruns/disposal, loop cutoff, recursion, original return/throw/rejection behavior, await IO exclusion, independent suspended executions and owned watchdog timer cleanup.
