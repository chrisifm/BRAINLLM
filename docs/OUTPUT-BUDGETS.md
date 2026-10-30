# Output accounting and static-route budgets

Output metrics come from the final compiled assets and their generated `.gz` and `.br` siblings. Compressing `runtime/brain-router.js` source, comments included, is not a measurement of its minified production chunk. Record the exact compiler/source/binary identity and workload whenever publishing results.

`brain build --json` exposes the route `ledger`; `brain build --report` keeps the existing conservative summary.

`brain-pack::Output.ledger` contains one `PageLedger` per route. `PageLedger::to_json()` exposes a versioned report with HTML, executable inline-JavaScript bytes, initial/deferred/conservative JavaScript totals and every reachable emitted JS/CSS asset. Asset paths are dist-relative; source identifiers are project-relative or `runtime/<module>`, never absolute local paths. Aggregated CSS may have no single source identifier.

- `bytes.raw` is the final emitted asset length, after minification and tree shaking.
- `bytes.gzip` and `bytes.brotli` are the actual precompressed transfer lengths when smaller than raw; otherwise they report the raw serving fallback.
- `initial: true` identifies scripts/styles referenced by executable HTML, module preloads and transitive **static** imports. A preload counts as initial even if execution happens later.
- Deferred assets include dynamic imports and island state references not statically imported/preloaded. Future-page assets listed only in navigation planning metadata do not enter this route's graph.
- `entryJs + deferredJs = conservativeJs`. Each asset is counted once per route. Inline scripts are counted separately as `inlineJsBytes` and are already part of the compressed HTML; adding their size to compressed HTML would double count them.

This is emitted-reference accounting, not an observed network trace. Runtime-dependent imports, worker requests, external URLs, images, fonts, headers and service-worker precache traffic require separate measurement. Conservative graph totals include deferred chunks and must not be labeled initial load. Initial/deferred classification describes emitted references, not browser execution time or response cache hits.

The existing `ReportRow` and `budget.viewKB` retain their conservative graph semantics. Their `js_raw` column is pre-minification codegen size; ledger `bytes.raw` instead means the bytes of the final compiled asset. No existing budget silently becomes an initial-load estimate.

## Static-route contract

A successfully prerendered route without interactive runtime usage, SSR, fetches, failed components, executable scripts or guards/middleware ships no executable page JavaScript. It carries `<meta name="brain-zero-js" content="1">`; the build omits hydration, router startup and service-worker registration for that page. JSON-LD and inert navigation-planning metadata remain data. CSS and hashed public assets retain their normal caching.

Static pages use normal browser navigation. Interactive pages give their router a `fullLoad` list of the known static route paths, so entering a static route performs a complete navigation instead of carrying a router lifecycle into that page. Predictive fetching can still anticipate those static destinations; privacy-oriented `skip` remains a separate exclusion. Existing service workers may still serve these pages; a static page does not install a new worker by itself. Static pages therefore do not obtain first-visit offline installation until an interactive page installs the worker.

SSR routes, guarded routes, loading/data views, client-only widgets and interactive layouts retain their existing loading and hydration behavior. A failed or unavailable prerender is not evidence that a page is static. Custom executable script tags prevent zero-JavaScript classification.

Regressions cover static-only output, static routes inside mixed sites, static offline pages, interactive pages, dynamic imports, initial preloads and planning metadata. Browser network measurements and comparison against equivalent handwritten implementations remain separate acceptance work; no new byte-performance claims are inferred from source inspection alone.

Native-library packing regressions in `crates/brain-pack/tests/native_libraries.rs` inspect the final date-only/hash chunks and their actual codec siblings. They verify unused date operations and an unrelated hash module do not enter a date-only fixture, and server imports resolve to an emitted native hash chunk. These tests complement helper behavior tests; source-file gzip is not their oracle.
