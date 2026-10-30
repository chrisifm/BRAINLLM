# Production cleanup and machine-readable feedback

Production `brain_min::minify_module(source, true)` removes proven unused imports,
local/module declarations and unreferenced function declarations by default.
It reuses the existing in-house JavaScript scope parser, reference marks and
module tree-shaking model. No ESLint parser, bundler or third-party minifier is
installed. The compiler's existing source diagnostics remain responsible for
broader language/style rules; this report describes actual production cleanup,
not complete ESLint or Stylelint rule coverage.

The string-returning entry points remain available. Their report variants expose
what changed before local-name mangling:

```rust
minify_module_with_report(source, true) -> Result<Minified, CleanupError>
tree_shake_with_report(source, &live_exports) -> Result<Minified, CleanupError>
// Minified { code: String, report: CleanupReport }
// report.to_json() and error.to_json() provide deterministic structured JSON.
```

`prod: false` does not run cleanup and returns an empty report even when ordinary
minification renames variables. Renaming is never counted as source removal.

## Effects and scope are contracts

- `const unused = effect()` becomes an expression that still executes `effect()`.
  A possibly throwing identifier read, getter, coercion, iterator or allocation
  is retained unless the model proves it effect-free. Global constructors such
  as `Map` are not assumed pristine or free of effects.
- Proven unused import bindings are removed. If none remain, the import becomes
  a side-effect import. Default, namespace and named/aliased imports are handled;
  module evaluation and import attributes remain. There is no default assumption
  that an imported module is pure.
- Lexical reads can disappear only when initialization/order and the owning
  function scope are established. Captured values, temporal dead zones, imported
  live bindings and module cycles retain throwing reads. `this`/`super` are not
  discarded as harmless literals.
- Destructuring, defaults, loop headers, control bodies and ambiguous declarations
  remain when their binding/iterator semantics cannot be safely rewritten. Their
  retained facts tell the LLM to review source instead of applying a guessed fix.
  Writes are treated as uses; write-only-state elimination is not claimed here.
- Existing export contracts are preserved by module cleanup. Tree shaking may
  remove unused exported declarations only under its caller's explicit live-export
  contract. Function parameter count, defaults and live lexical shadowing remain.
- Direct unbound `eval` prevents an unused-binding proof. Its scope is retained
  with structured feedback. A failure to understand syntax is a typed analysis
  failure, not permission to erase code.
- Production debug removal applies to unshadowed global `console.log` calls.
  Observable argument evaluation, exceptions and spreads remain in order. Local
  objects named `console` remain application code. No runtime logging shim or
  cleanup-report code is injected into the application.

Statement boundaries are retained when an initializer becomes an expression.
This matters with automatic semicolon insertion: replacing a declaration with
`(effect())` must not turn the previous expression into a function call.

## Feedback is evidence of transformations

Each fact identifies `action`, `kind`, a safe optional public `binding`, byte
`start`/`end`, input `line`/`column`, a reason and a source cleanup instruction.
Actions distinguish `removed`, `initializer-retained`,
`side-effect-import-retained` and `retained`. The first three can describe removal
of a binding; `retained` never increments a removal count. Debug-call removal is
not a variable-binding removal.

`removedBindings` counts targeted binding sites actually transformed, including
removed import bindings. `removedImportBindings` is its import subset.
`retainedEffects` counts preserved potentially observable expression/module
operations; it does not claim that a runtime event occurred. Removing a parent
function/declaration covers its body with the parent fact; nested declarations
are not independently counted when their entire enclosing operation disappears.
Overlapping nested rewrites are deferred so a retained initializer is not falsely
reported as emitted after its parent has disappeared.

The report's coordinate space is **`input-js`**. Columns are one-based UTF-8 byte
columns and offsets are byte offsets. For generated chunks these positions are
in the generated input, not automatically in the author's `.brain` file. A pack
integration may attach source filenames and map a declaration only when it has
unique evidence. Otherwise it must retain generated positions and report
`originalLine: null`; token offsets are not a source map.

Reports never contain initializer text, source bodies, literal values or import
specifier strings. Public binding names are bounded and sanitized. Failure JSON
contains a code, stage, optional input position, reason and fix; raw parser
snippets are deliberately excluded. This is build/tooling metadata, not application
console output.

Detailed facts stop at 4,096, but optimization and total counts continue.
`factsTruncated` and `omittedFacts` explicitly identify omitted details. A truncated
report is not a complete per-declaration log; CLI/MCP consumers should retain the
summary and paginate their own bounded presentation. Truncation alone does not
fail a build.

The engine bounds input to 8 MiB, 200,000 tokens, 8,192 scopes, 256 delimiter depth and an arrow-count guard per statement. Cleanup has a work/iteration budget and reports an actionable typed error
when analysis cannot finish safely. It does not fall back to deleting unproven
code to meet a byte target.

## Verification

```sh
cargo test -p brain-min -j 2
```

Regression fixtures execute original and transformed JavaScript with Node. They
cover effects, getters, destructuring/defaults, iterators, coercion, TDZ and imported
live bindings, real module evaluation, lexical shadowing, export/default parameter
contracts, direct eval, ASI, nested rewrite accounting and bounded/redacted reports.
These complement the existing lexer/mangler/template tests. The ordinary compiler,
packing, runtime, fixture, privacy and LLM gates still apply to an integrated
release; this subsystem does not certify the entire release.
