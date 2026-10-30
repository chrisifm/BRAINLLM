# Optimization feedback for agents

Production cleanup facts describe operations the native minifier actually
performed, including removed bindings and retained initializer/import effects.
The original `binding`, `action`, `kind`, `reason`, `fix`, `start`, `end`, `line`
and `column` remain unchanged. Their coordinate space is the minifier's input
JavaScript, which may differ from the author's BrainScript.

`optimization_feedback::annotate(root, source_file, report)` enriches a report
without changing source files. A verified declaration receives:

```json
{
  "mapped": true,
  "mappingReason": "unique-binding-declaration",
  "originalLine": 7,
  "location": {
    "file": "comp/card.brain",
    "line": 7,
    "column": 9,
    "precision": "source-declaration"
  },
  "generatedLocation": {
    "file": "comp/card.brain",
    "line": 18,
    "column": 11,
    "coordinateSpace": "input-js"
  }
}
```

The generated file label identifies the report's source module, not a claim that
line 18 belongs to the original BrainScript. A source declaration is similarly
not an exact source mapping of the removed instruction. Agents should inspect
the declaration and its uses before editing anything.

## Revision and identity proof

The pack wrapper must supply `originalSourceHash`, the SHA-256 of original source
bytes captured when the compilation graph read them. Annotation uses the existing
bounded, secret-guarded changes reader and compares its actual source revision.
A missing hash gives `source-revision-unverified`; changed bytes give
`stale-source`. Neither case assigns an original line, even when the same unique
binding still exists at a new line after a rapid edit.

Only one declaration of the binding across the entire parsed source qualifies,
and its declaration kind must match the cleanup fact. Imports, variables,
functions, destructured patterns, parameter defaults and bindings inside nested
functions, conditions, loops, templates, slots, metadata expressions and head
content all participate. Property names, literal values and identifier reads are
not declarations. Shadowing anywhere makes a repeated name ambiguous; there is
no nearest-line or scope-name guess.

Unknown, ambiguous, unsupported-kind, runtime/compiler, private, unreadable,
secret-containing, oversized and unparseable sources retain `mapped: false`,
`originalLine: null`, and null original coordinates. Their precision is `generated`
when input coordinates exist, otherwise `module-only`. `mappingReason` explains
the uncertainty without returning reader errors or source snippets.

## Bounds and privacy

Reports accept at most 4096 object facts. Original source parsing is limited to
256 KiB, 65536 AST visits, 8192 declarations and depth 128. Exhausting any AST
budget invalidates attribution for the entire file, since unvisited scopes might
contain shadowing declarations. The shared source reader also rejects private
paths, symlinks and detected secrets. Source bodies and literal values never enter
the annotation output.

`sourceMapping` provides the actual public-source revision, mapped/unmapped and
ambiguous counts, status and explicit limits. This feedback does not generate new
diagnostic codes, modify files, run a browser, or claim that an optimization
should be applied mechanically to every matching source declaration.
