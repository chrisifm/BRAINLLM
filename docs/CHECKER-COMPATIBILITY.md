# Compatible warning diagnostics in 0.4

A recognized name does not guarantee browser support or execution checkpoints.
The following boundaries intentionally add warnings to previously accepted code:

- B2101 reports an unsupported browser API for the configured target. A direct
  application call to `scheduler.yield()` remains a native browser API, with no
  injected shim. The runtime's own internal yielding fallback does not wrap that
  application call. A feature guard needs an actual fallback.
- B0912 reports an imported plain JavaScript module that runs outside compiler
  execution checkpoints. The dashboard example deliberately demonstrates this
  boundary and expects exactly one warning on its foreign API import. This
  fixture expectation does not exempt other files or imports.
- B2005 warns about a proven nullable dereference. A `bind:api`/`bind:ref` target
  is written by the runtime, so its initializer is not treated as a permanent
  value in later template events. Unbound nullable values remain checked.

Import typo hints remain confined to the inferred project, including standalone
components with a project-relative source identity. Candidates must be regular
files/directories; symlinks are excluded. An incomplete or oversized directory
listing produces a bounded explanation and no automatic import replacement.
