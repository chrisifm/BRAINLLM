# Private project manifests

Project manifests make component contracts available to agents without returning source bodies or running application code. `manifests::query(root, target)` returns the project index or one exact component/view contract; `refresh(root)` refreshes the private cache. A target may be an app-relative file or a unique component name. Ambiguous names require an exact file.

The cache lives in `.brain/manifests`, outside public assets and distribution. Its index and document records use schema version 1, a generator identifier, framework version and SHA-256 revisions. Metadata includes component/view identity, source locations, prop types and optional/bindable/default-presence flags, declared emits and exposed names, slots, static imports, used token names and structural state evidence. It excludes source bodies, prop default values, environment values and secret files.

## Confidence and invalidation

Manifests describe static evidence. Await branches, empty each branches and disabled attributes can establish declarations, not runtime coverage. Missing states are marked unknown. Parse failures produce bounded diagnostic codes and locations with unknown confidence; manifest generation does not run the complete semantic checker. `semanticCheck: not-run` is explicit. Consumers must inspect coverage and truncation flags rather than assume a complete catalog.

Dependency fingerprints include the component, proven relative import dependencies, token stylesheets, the design foundation, application configuration and relevant ancestor layout/guard files. Changing an imported module invalidates dependent metadata even if the component's own source revision is unchanged. Dynamic and external imports remain unresolved evidence; no arbitrary dependency or effect is inferred.

Queries re-read bounded source revisions before reusing the cache. A matching input fingerprint permits metadata reuse. The index records source completeness separately from unknown or truncated contracts, so an unchanged parse failure is not misrepresented as successful checking.

## Bounds and ownership

One scan considers at most 500 source files, 256 KiB per source and 16 MiB total input, with a bounded traversal and a 15 second source-scan deadline checked between files. Symlinks, hidden paths, dependency/build directories, nested application roots and credential files are excluded. The index reports skipped files and incomplete coverage. Contract collections have their own bounds. Total current document metadata plus the index is limited to 2 MiB; excess output produces an explicit error or truncation evidence.

The private cache uses restrictive permissions, an owned writer lock and bounded atomic file replacement. Documents are prepared before publishing the index. Index publication happens last; readers verify each document's recorded metadata hash. Interrupted replacement can require a refresh rather than returning mismatched metadata. Failure cleanup removes only unchanged newly created owned files. Refresh prunes only unchanged obsolete documents previously indexed by this generator; unrelated files and modified documents are preserved.

The cache stores metadata, not an executable component snapshot. A source revision is an invalidation signal, not proof that application runtime behavior or browser compatibility has been tested. See [the design foundation](DESIGN-SYSTEM.md) for the private static contract gallery.
