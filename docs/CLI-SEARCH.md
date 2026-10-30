# Native source navigation

`brain find`, `brain symbols` and `brain refs` inspect app source through the native
parser without compiling the app, starting a browser or installing a parser.
They return identifier metadata rather than source bodies or literal values.

```sh
brain find card --kind component --json
brain find save --kind function --limit 20 --cursor 0 --json
brain find format --kind export --json
brain symbols comp/user-card.brain --json
brain refs render --json
brain find card --watch
```

`find` matches identifier names or source-file paths case-insensitively. `symbols`
indexes one guarded app-relative file. `refs` matches an identifier name exactly,
including declarations and actual identifier uses found in the AST. String and
markup text matches are excluded. A property name without an identifier-binding
use is not advertised as a resolved reference.

The Rust interface is `query(root, operation, args) -> Result<Value, String>`.
Arguments are `{ query }`, `{ file }` or `{ name }` for the respective operation,
plus optional `kind`, `limit`, `cursor` and `metrics`. `run(args)` expects its first
argument to be `find`, `symbols` or `refs`. CLI/MCP registration belongs to the
callers of these interfaces.

## Records and resolution boundaries

Records contain stable opaque IDs, names, kind, source file, line, column, source
revision and a grouping parent. Declarations in a component point to that
component/view and include its source component name; modules group by
`source-file`. These parents do not claim to reconstruct the lexical scope tree.
Discoverable entities include components/views, function declarations, callable
variables, identifier-named object methods, imports, variables, parameters, props,
exports and bindings nested in template/function scopes. `bindingKind` preserves
the underlying declaration kind when a callable variable appears as `function`.
The `export` kind filter selects `exported: true` records. An anonymous default
export has the explicit name `default` and `export-statement` precision.

References are labeled AST identifier occurrences rather than a complete
cross-module binding or call graph. A local import carries its normalized literal
target and imported name. Identifier uses receive that link only when exactly one
binding with that name exists across the complete source, excluding shadowing.
This proves the import binding and literal target path, not the target export's
implementation or runtime execution. Target path safety and availability are
checked again on every query, including cache hits.

Unsupported JavaScript syntax is reported as unknown rather than analyzed with
another parser. Native `.brain` and `.jsbrain` files are the primary inputs; plain
`.js` and `.mjs` are supported only within the existing native parser's grammar.
Skipped, private, secret-containing, oversized or unparseable files make
`coverage.complete` false. A missing result is not proof that a symbol or reference
does not exist in a skipped file.

## Bounds, cache and live snapshots

The walker skips hidden/private paths, generated/dependency directories, symlinks
and nested apps. It is limited to depth 32, 2048 files and 8192 directory entries.
Guarded reads process at most 256 KiB per file and 16 MiB per query. Results retain
at most 16384 records, with pages of 1–200 records and explicit `nextCursor`.
Coverage reports skipped files and truncation; pagination covers the materialized
bounded snapshot, not an unbounded project.

The process-local cache stores only AST symbol/reference metadata, keyed by the
canonical project, file and exact SHA-256 source revision. It retains at most
128 files and 8 MiB of serialized metadata, evicting least-recently-used entries.
Every query rereads guarded source bytes before accepting a cache hit; modified
files are reindexed without running a whole build. No environment, source body,
source literal or account credential is cached. Changes across different files do
not form a filesystem transaction.

On Unix, `--watch` emits debounced NDJSON source snapshots. It polls bounded file
metadata, waits 150 ms of stability and stops after 30 seconds, 20 snapshots,
Ctrl+C or stdin EOF. It owns and restores its interrupt handler, starts no child
process or service, and retains no detached reader/watcher. Metadata-preserving
external edits can escape mtime polling; a fresh explicit query still hashes its
actual guarded input. On other platforms, snapshot queries work and watch reports
its current platform limitation.

## Measuring the actual operation

API callers can request `metrics: true` to inspect `parsedFiles`, `cacheHits` and
`elapsedMicros`. Metrics are omitted by default. Inline regression fixtures record
cold, warm and changed-file timings, assert that a warm unchanged query reuses its
metadata and that a content change forces reindexing. They also cover callable
and nested symbols, aliases, shadowing, target deletion, pagination, default
exports, methods, private paths, secret guards and size bounds.

These measurements characterize the actual fixture and machine. They do not
claim that native queries are universally faster than `rg`, editor indexes or a
large project's existing language server. Use `rg` for raw text/body searches;
these commands supply structured source entities and clearly qualified references.
