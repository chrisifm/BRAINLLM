# Compiler-selected form capabilities

The author-facing `form(schema, options)` API is unchanged. A closed flat
literal schema can use tracked internal text/scalar helpers. When the options
are also proven not to configure asynchronous checks, the compiler can omit
that optional capability from the emitted runtime graph.

The synchronous selection requires an absent options object or a literal with
only unique `init` and `submit` properties. A submit callback must be an arrow
or an unshadowed server-function wrapper whose identity is known. A normal
function can mutate its options receiver through `this`, so it keeps the
complete engine. Unknown callbacks, mutable/indirect options, cross-field
validation, checks, computed keys, duplicate keys, getters and spreads also
keep the complete engine. Dynamic schemas keep the original public validator.

Internal synchronous helpers reject unexpected check/validation configuration
rather than silently skipping it. Public forms and non-specialized compiled
forms continue to read mutable check options at execution time. Asynchronous
checks still await completion, reject stale answers, report checking state
and block submission when invalid.

The shared engine retains field validation, scalar coercion, server-returned
errors, no-JavaScript restoration, focus/ARIA behavior, busy locking, reset,
dirty tracking, redirect behavior and owner cleanup. The optional remote-check
function is passed into that engine only by factories that require it; normal
module tree shaking can remove its code from the synchronous graph.

Evidence must include equivalent observable form traces, conservative codegen
fallback cases and final emitted asset accounting. The pack regression compares
actual compressed helper assets with and without configured checks. Source
file gzip size alone does not establish a route improvement. Route budgets
continue to include deferred modules and private HTML without raising limits.
