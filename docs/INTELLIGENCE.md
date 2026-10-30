# Static intelligence and preventive diagnostics

The CLI intelligence module produces bounded JSON (`schemaVersion: 1`) for agents.
It connects component and module imports, view routes, shared state reads/writes,
client API declarations, backend resource fields, policies, hook queues and cache
invalidation targets. It never evaluates application code or returns source bodies.

## Interfaces

- `graph(root)` returns the first 100 nodes and edges.
- `graph_page(root, cursor, limit)` independently pages each deterministic collection;
  the limit is clamped to 1–500. Totals and `nextCursor` identify omitted records.
  An edge can reference a node on another page.
- `context(root, task, budget)` ranks identifiers by lexical task relevance. The budget
  is UTF-8 bytes, a conservative token upper estimate without a model tokenizer;
  it is clamped to 512–32768 bytes. The complete serialized response fits the limit.
  It includes structural contracts rather than executable source or literal values.
- `impact(root, target, operation)` accepts a file, node ID, resource name or field
  (`posts.title`, `field:posts.title`). Operation is absent, `create`, `update` or
  `delete`. It returns reverse dependency evidence and a proposed invalidation plan.
- `why(root, target)` combines that evidence with actual compiler diagnostics.
- `preflight(root, target)` runs the real checker, exposes the first blocking
  diagnostic, exact source locations and preventive findings. An absent target
  requests project diagnostics; a target limits them to its static consumers.

These are Rust module interfaces; CLI/MCP wiring is maintained by their callers.

## Evidence boundaries

Imports must resolve within the project. Symbolic links are not scanned. A literal
`api('/api/posts', ...)` is connected only to an explicitly declared backend
`posts` resource. The runtime uses API paths verbatim. Other endpoints, dynamic
paths, external dependencies and parse failures produce `unknown` entries. This
endpoint match does not prove the backend server is attached to the frontend or
SSR server. Import consumers are potential consumers; execution is not inferred.

Shared state edges reuse `brain_check::contract::deps`. Computed dependency keys
with source literals are omitted. Policies and hooks reuse the backend literal
contract loader. Their evidence identifies the contract file; the line is null
when the contract loader does not preserve a source span. Import and API edges
carry the parser's actual line and column. No SQL effects or job handler side
effects are guessed.

`impact` walks reverse static consumer edges. A declared resource mutation also
lists its operation-specific hook queues. Field targets conservatively affect the
whole resource. Cache keys are proposed resource invalidation targets, not claims
that runtime caches already implement those keys. `performed: false` and
`runtimeProof: false` remain explicit. Unknown edges can make the plan incomplete;
it must not be used as proof that unlisted resources are unaffected.

`preflight` and `why` reuse checker findings rather than assigning speculative root
causes. Each finding contains code, file, line, column, severity, message, fix and
a diagnostic help command. Quoted source literals and sensitive diagnostic detail
are redacted. Run `brain check` locally for exact replacement text. Static checker
findings are distinct from observed runtime failures; runtime trace collection is
owned by the existing runtime diagnostics interface.

## Bounds

Scanning stops at 2048 source files, depth 32, 256 KiB per source and 16 MiB total.
Graph nodes and edges are each capped at 16384. Hidden directories, dependencies,
output directories, vendor trees and nested projects are skipped. Collection
truncation is explicit. Impact output has at most 500 affected IDs and paths,
100 views/cache keys and 100 unknowns. Diagnostic reports have at most 100
findings. Context output has at most 100 nodes and 100 edges and also enforces its
serialized byte budget. Graph responses omit environment files, values and source
bodies. The graph is rebuilt from declarations on each call rather than retaining
a stale process cache.

## Verification

Inline unit tests cover reverse import propagation, separation of hooks from
reverse dependencies, literal redaction, path containment and concrete graph
contracts. Run them through the normal workspace gates after integration. Runtime
execution and cache residency require separate trace or browser evidence.
