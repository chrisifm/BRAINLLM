# Runtime error traces

`brain-err.js` captures observed render, reactive, handler and global failures.
It preserves the legacy fingerprint, component, `line`, `stack`, `src` and
`brainStack` fields. Existing fallback and handler recovery behavior is unchanged.
The inspector forwards the additional structural trace fields in its existing
`{ t: 'm', v: 2, seq, at, d }` envelope. Each event still has `k: 'err'`.

## Trace contract

- `location`: `{ component, file, line, column, precision }`. Component names come
  from the actual owner when available. Unknown names or coordinates are null.
  `precision` is `generated`, `source-line`, or `component-only`.
- `generatedLocation`: the first non-runtime stack location, or null. It remains
  separate after source mapping so generated and source coordinates cannot mix.
- `frames`: ordered entries `{ generated: { file, line, column }, source, runtime }`.
  A source entry is null until a valid map resolves it. The framework's line maps
  provide `{ file, line, column: null }`; generated columns are never presented as
  original source columns. Function arguments and stack headers are not captured.
- `causes`: the actual `Error.cause` chain, nearest cause first. Entries contain
  `depth`, `type`, `code`, bounded `msg`, `location`, `generatedLocation`, `frames`
  and their own stack/frame truncation flags. Cycles terminate the chain. Arbitrary
  error properties, variable values, response payloads and cookies are not copied.
- `mapping.status`: `disabled`, `pending`, `mapped`, `unavailable`, `timeout` or
  `limited`. Source locations and legacy `src`/`brainStack` arrive asynchronously;
  subscribers receive an enriched notification with the same fingerprint.
- `truncated`: booleans for `stack`, `frames`, `causes`, `componentChain`,
  `breadcrumbs`, `locations`, `mapping` and `output`. Consumers must preserve these
  flags when presenting a partial trace.

A source location identifies the nearest mapped application frame. If the first
throw frame has no map, inspect `frames` and `generatedLocation` to distinguish it
from a mapped caller. The source path is derived from the loaded module URL, as
in the existing framework line-map format; map contents do not supply arbitrary
output paths. Production modules without source maps retain generated locations.

These records prove that a failure was captured. A chained cause was explicitly
attached by application code; the recorder does not infer hidden causes or predict
future failures. Compiler preventive diagnostics remain a separate `preflight`
interface. Agents should inspect the structured location, then the ordered frames
and attached causes, and reproduce a failure before attributing unobserved effects.

## Privacy and resource bounds

Public URLs contain paths, without query strings or fragments. Diagnostic URL text
is similarly scrubbed. Stack headers, local variables, signal values, input values,
custom error properties, cookies and authorization headers are not collected. Error
messages are application-authored text and should themselves avoid private data.

Each record retains at most 24 frames, six causes, 32 component ancestors and 30
breadcrumbs. Stack parsing reads at most 16384 characters and 256 lines per error.
Paths are limited to their last 1024 characters. Messages, labels and identity
fields have independent length limits. Serialized records are capped at 65536
UTF-8 bytes; nested frames and then remaining trace detail are removed if needed.
At most 100 deduplicated records remain in memory. Inspector event buffering is
capped at 2000 events, retaining the newest events when a consumer stops draining. An overflow adds
`dropped` to that drain envelope, reporting exactly how many events were evicted.

Source mapping uses at most two active loads, 64 pending requests and 64 cached
maps, with at most 100 record enrichments in flight. Each map has at most 65536 generated lines, with a maximum source response
of 1 MiB. Successful maps are evicted by least recent use; failed loads are not
retained as durable cache entries. Cache identity includes the module revision
query internally; those queries are never emitted in traces. Requests target the
same origin, with local `file:` modules supported by development loaders.

The two-second deadline covers queue wait and load time together. Owned fetches
receive an AbortSignal. `capture({ load(url, { signal }) })` loaders must honor that
signal to release their own work. An uncooperative custom loader keeps its active
slot until it settles, preventing unbounded abandoned work; trace enrichment still
finishes at the deadline. Waiting tasks expire without extending their deadline.

## Verification

Run:

```sh
node --test runtime/test/errors.test.js runtime/test/errmap.test.js runtime/test/inspect.test.js
```

Regression coverage includes source-line precision, preservation of generated
coordinates, duplicate file names, unavailable maps, cyclic/deep causes, bounded
frames and ancestry, URL redaction, inspector propagation, mapping concurrency,
deadlines, abort delivery and continued operation of existing fallbacks.
