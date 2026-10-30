# Live local compiler feedback for agents

Development compilation persists a private structured compiler event before returning an error component to an HTTP request. HMR uses the same compilation path, and its agent event callback runs before WebSocket publication. Agents can react to the source error before the browser overlay receives it; the browser fallback remains available to the person debugging the app.

This is a local delivery channel, not automatic model execution. It does not spawn an LLM, guess a model, edit source, apply fixes, deploy or publish. An existing agent polls/subscribes, reads the native diagnostic location and uses the normal leased revision-checked changes protocol. The event's source revision is the version that actually compiled, not a claim that the source has remained unchanged afterward.

## Persistent bounded spool

`.brain/feedback/events/` contains 64 fixed event slots, a state index and a kernel writer lock. Each event is at most 16 KiB; state is at most 64 KiB. At most sixteen sanitized diagnostic details are retained per event, reduced further if the byte budget requires it. The full reported diagnostic count and detail truncation remain explicit. Active error tracking is bounded to 128 files; exceeding that capacity marks tracking incomplete instead of claiming every resolution is known.

Events have schema version, producer, epoch, monotonic sequence, record hash, `compile-error` or `compile-resolved`, app-relative source file, source revision, bounded locations, known diagnostic code/title, catalog fix guidance and evidence. Raw compiler messages/fixes, source snippets, environment values, arbitrary runtime payloads, credentials and exception stacks are never copied into this spool. A diagnostic referring to another file has an unknown source revision unless that file itself was the compiled input. Exact value-containing edits remain in the native checker interface; this feed does not invent an automatic patch from redacted text.

Unchanged sanitized diagnostics for the same source revision are deduplicated. A later successful compilation records a resolution only for a tracked active error. Sequence persists across process restarts. A missing/replaced store or different framework generation changes its epoch. This is not an inferred app deployment identifier: consumers must also check source revisions, and a framework-version reset is explicitly observable. No unseen build is declared clean.

A slot is synced and atomically renamed before its state index. A verified slot published immediately before an interrupted index write is recovered on the next compiler publication without reusing its sequence. A corrupt slot, overwritten retention range, mismatched epoch, partial publication or failed try-lock/write is explicit gap/unknown delivery, not a complete event history. OS/filesystem stalls cannot have a hard synchronous latency guarantee; the write work and bytes are bounded, and no arbitrary external operation runs in that path. Normal filesystems also cannot promise a cross-file transaction or exclude an unrelated editor racing the final rename. Metadata directories, event slots and lock files refuse symlinks and unsafe object types.

If persistence cannot be confirmed, the server emits a structured `compiler-feedback-gap` notification before the browser fallback/overlay. It does not silently call that durable delivery. Existing MCP messages, dev logs and browser diagnostics retain their separate channels; the spool adds sanitized persistent evidence rather than retaining arbitrary console content.

## Poll and finite watch

CLI: `brain feedback query --request '{"afterSequence":0,"limit":64}' --json`; MCP: `brain_feedback` with the same options. Managed `brain_run` also emits live notifications.

`feedback::query(root, options)` accepts `afterEpoch`, nonnegative `afterSequence` and `limit` 1..128. It returns retained events, next sequence, earliest/latest retained sequence, reset/gap indicators, missing sequences, partial-publication status and whether the snapshot advanced during the read. A producer can overwrite old bounded slots while an agent is away; `gap` means it must reconcile with current native `brain check` and source revisions.

CLI: `brain feedback watch --request '{"durationMs":5000,"maxRecords":64}'` emits bounded NDJSON and a final continuation cursor. Pass its epoch/sequence to the next invocation.

`feedback::watch(root, options, writer)` writes NDJSON event/control records, polls every 150 ms, runs for at most 30 seconds and emits at most 128 events plus bounded control records. It returns a continuation epoch/sequence. It does not leave a persistent watcher process behind, enqueue unbounded work or claim that absence of events proves absence of errors. Querying an app without a spool reports unavailable and creates no metadata store.

Verification targets include real HTTP fallback ordering, a real socket proving the agent callback precedes WebSocket publication, persistent redaction, error/resolution deduplication, reopen behavior, bounded retention gaps, partial-index recovery, corrupted-slot detection, symlink refusal and finite watch deadlines. Test source exists independently of execution results: report actual completed focused/workspace gates rather than assuming these cases passed from implementation alone.

Public UI query keys use a fixed opaque component namespace rather than local filesystem paths. Dialog defaults prefer a stable ref/id and then the ordinal of dialog declarations; formatting and CSS hash shortening do not change them. Explicit urlKey is required for links that must survive structural dialog insertion/removal or component renames. B0917 identifies thread capability/module-contract failures separately from B0916 local SQL failures, so an agent receives relevant correction guidance.

Production Node server diagnostics are a separate private channel: arbitrary exception messages are discarded even
there, because driver/application errors can contain credentials. Up to forty bounded V8 source frames, the request
reference, event and remediation remain available. URL query/fragment values are excluded. A configured log sink
receives this sanitized shape too. No source trace is included in public production responses.

Impact measurement coalesces repeated saves into one worker with at most 128 pending paths; diagnostic history
tracks at most 512 paths and marks an incomplete comparison after eviction or overflow. HMR URL paths are resolved
inside the app, rather than mistaken for absolute filesystem paths. Source resource/job/auth edits add a compact
`backendContract` diff with hashes, changed fields, conservative breaking/security flags and migration guidance.
These are current source contracts, not a claim that a running backend or database was automatically replaced.
Inspect `brain backend schema --json`, migrate explicitly when required, then restart the selected backend.
