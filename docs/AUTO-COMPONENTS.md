# Automatic project component imports

A PascalCase component tag can omit its explicit import when exactly one safe project file has that component name:

```html
<UserCard label="Ada" />
```

If `comp/user-card.brain` is the only `UserCard`, the compiler inserts its relative default import into the source it compiles in memory. It does not write the source file. Diagnostic **B0222 (info)** reports that normalization and supplies an exact edit; `brain fix` can persist the explicit import when desired.

Names follow the parser's existing `component_name` mapping, such as `user-card.brain` → `UserCard`. This is exact matching, not fuzzy guessing. It does not install modules or search outside the application.

## Priority and ambiguity

Explicit imports, script/prop bindings and template-local bindings keep their existing meaning. The built-in UI kit, `Img` and `Portal` retain their existing priority; overriding a kit name still requires an explicit declaration/import. Native HTML and lowercase custom elements remain element nodes and are never reinterpreted as project components.

Two project matches are ambiguous even when one is under `comp/` and the other under `ui/`. B0203 stays an error and lists bounded app-relative candidate paths. Choose a path explicitly. Missing names retain the existing B0203 behavior. The current component cannot discover/import itself automatically; direct recursive self tags require an intentional explicit design.

Discovery indexes normal `.brain` source files inside the nearest application root (`app.brain`, `brain.json`, a token file or the existing type-file boundary). It includes component/UI/custom source directories and ordinary view files. It excludes test files, underscore framework files, `app.brain`, hidden/generated/dependency/private folders, root public/server/content/docs/bench folders and nested applications. A missing project boundary causes no filesystem-wide search.

Symlinks are not followed. An unreadable path, unsupported filename or exhausted budget makes that index incomplete; no partial unique match is auto-resolved. Explicit imports remain available.

## Normalization and validation

All resolved names in one component produce **one grouped edit**, avoiding colliding zero-width insertions when several imports are missing. The edit adds imports to an existing script, expands an empty self-closing script, or creates a script after leading metadata/comments and before head/markup. Script-like text inside comments or markup expressions is not treated as a script block.

The normal compiler normalization loop applies info edits only when no error diagnostic blocks the pass. It then parses/checks the normalized component again. Imported child prop, event and exposed-method contracts are validated through the same path as explicit imports. B0203 is suppressed only for successfully resolved names; ambiguous or incomplete discoveries remain errors. Other diagnostics are not weakened.

This uses one normalization pass for all missing imports and stays within the compiler's existing four-pass limit. Source remains the editable authority. In-memory imports appear in compiled output and dependency tracking, while optional persisted imports make the dependency explicit in the authoring file.

## Cache and bounds

The metadata index holds at most eight application roots. Directory size/timestamps and Unix change times invalidate cached entries; the index never caches component source content. Every scanned directory is also returned as a compiler dependency, including after normalization has introduced explicit imports. Creation, deletion, renaming and a newly ambiguous candidate therefore invalidate normalized compilation caches and watch/HMR context.

Limits are 256 directories, 4,096 component files, 8,192 directory entries, 12 directory levels and 256 requested names per component. Candidate paths shown in an ambiguity message are capped at 12. Incomplete indexes are not reused as successful cache entries.

Regression tests cover grouped imports, existing/empty scripts, metadata/head/comment placement, unique and ambiguous names, explicit/local/built-in/native priorities, real child-prop validation, direct self rejection, nested project isolation, creation invalidation, symlinks and exhausted bounds.
