# Offline browser dependencies and optimization proposals

The production graph resolves installed browser ESM packages without a registry request, installation or third-party JS parser/bundler/minifier. Server SDK bare imports remain external Node dependencies. Browser packages are emitted as reachable hashed modules; the compiler does not copy the entire installed package tree.

`brain deps analyze` exposes bounded metadata through `dependencies::query(root, "analyze", args)`, with an optional file. `preview` accepts one project source file, its expectedRevision and proposed content, checks it and returns CAS operations. The CLI/MCP adapters share this API. No LLM call runs during a build or proposal check.

## Implemented resolution

The package resolver searches installed node_modules ancestors, reads bounded package manifests and preserves JSON field order. It resolves exact exports, single-star subpath patterns and nested active browser/import/default conditions in their declared order. Installed package imports aliases are supported. Browser entry strings and contained browser map redirects are supported; false/blocked targets fail explicitly rather than receiving an invented polyfill. Package module entries and type-module main entries provide explicit legacy ESM entry paths.

These semantics follow the relevant [Node package export and import contracts](https://nodejs.org/api/packages.html#conditional-exports), with browser enabled as the browser-target condition. This is a documented supported subset, not complete Node/bundler resolution. Export fallback arrays, nonliteral dynamic imports, extension guessing, untransformed TS/JSX, CommonJS conversion and Node built-ins are not silently handled. Unknown condition-only exports fail. Relative package modules must stay inside their package and explicitly name .js or .mjs. Symlinked dependency trees are refused; workspace/pnpm symlink layouts need explicit reviewed support.

Installed package JS/MJS is loaded as JavaScript rather than compiled as BrainScript. Own lexical eligibility checks refuse known unsupported constructs; they are not a complete ECMAScript grammar validator. Fixed process.env.NODE_ENV reads become the literal production value only when no other process reference or ambiguous property/shadowing is present. No host environment is read and no general process polyfill is injected.

The existing content-hashed emitter diagnoses import cycles. This slice does not claim cyclic SDK graphs are supported. Dynamic literal imports retain every export, and their targets still need a statically bounded graph. Arbitrary external URLs and CDN imports currently require an explicit supported vendoring flow; builds never fetch them automatically.

## Integrity, bounds and side effects

Each resolved manifest/file has a SHA-256 revision. An existing npm-shrinkwrap.json or package-lock.json is hashed lazily when an actual package is used. The [npm lockfile](https://docs.npmjs.com/cli/v11/configuring-npm/package-lock-json/) revision identifies the reviewed installation input; hashing it does not attest that installed bytes match npm archive integrity. Files, manifests and the captured lock are rechecked before graph output. An installation edited during the snapshot fails with a retry instruction.

Package/file paths are contained, private segments are refused, symlinks are inspected, and Unix reads use O_NOFOLLOW/O_NONBLOCK plus regular-file/inode checks. This is not a sandbox for a malicious concurrently replaced filesystem. Do not mutate the dependency installation during a build.

The resolver bounds 128 package manifests, 512 modules, 4096 references, 1 MiB/module, 256 KiB/manifest and 32 MiB total read work, including revision rechecks, with a 30 second graph deadline. It reads only imported packages and their reachable dependencies. Private environment files are excluded. Unrelated builds do not parse a lockfile merely because it exists.

Package sideEffects values are reported as declarations, not trusted proof. Module evaluation edges are retained. Named export cleanup uses the existing conservative scope/effect analysis only when every incoming consumer has a supported static named import. Default, namespace, dynamic, worker and re-export consumers retain all exports. An unprovable cleanup retains the original module rather than guessing effect freedom.

Foreign modules use the own printer without runtime export remapping, app debug-log removal or public-member mangling. App-owned class, public-asset and base-link rewrites skip dependency source. SDK public API names, methods, logs and side effects remain external contracts. This is intentionally conservative; it does not claim complete SDK dead-code elimination or universal byte savings.

## Agent metadata and per-file proposals

Analysis scans at most 128 project files/2 MiB source, then the bounded foreign graph. It reports exact source/manifest/lock revisions, literal edges, statically imported names, unknown consumers, actual input/retained module bytes and literal-free cleanup facts. These are module-text measurements before final URL rewriting and page compression, not browser timing or final-page byte claims. Output is capped at 2 MiB and marks incomplete coverage. No module bodies, environment values or payloads are returned.

A proposal accepts only .brain/.jsbrain project source, never installed SDK source. It requires the exact current revision, rejects detected secrets and runs the real static checker. Accepted proposals return one file's revision-checked operation for the atomic change planner; failed checks return bounded diagnostic codes and source locations. Static acceptance does not establish equivalent behavior, faster runtime, smaller final output or a confirmed root cause. Review actual emitted byte deltas and runtime behavior before applying an optimization.

The focused production fixture compiles a real Brain view, resolves an installed ESM entry, verifies unused export removal and retained effects, and evaluates its emitted SDK chunk in Node without remote calls. Focused execution evidence belongs to the recorded test run; fixture source alone is not a passing result.

## Literal URL assets, SDK workers and Wasm

Literal native new URL(resource, import.meta.url) references are classified separately from module imports. Relative URL filenames use URL semantics, including a filename without a dot-slash prefix. Existing scope analysis establishes that URL refers to the native identifier rather than a custom local binding. A binary URL never becomes a Brain worker wrapper. Only an owned app plain .worker.js receives the compiler's bridge; SDK workers/proxies keep their original code and protocol.

Reachable SDK modules and declared resources retain their relative directory layout inside a SHA-256 versioned namespace. The namespace includes selected module code, resource hashes, manifest/lock metadata and framework version. This supports a dynamic filename default referring to an already reached sibling without guessing or copying unreferenced package files. Unknown dynamic resources still need an explicit supported declaration; arbitrary folders are not copied. Namespace invalidation is conservative across the reached dependency graph, not a claim of minimal per-package cache churn.

Supported static resources include bounded Wasm, image, font, CSS, text and JSON files. Wasm headers are validated; each resource is limited to 8 MiB within the graph's total read budget. Generated resources enter the build signature, immutable asset headers and existing precompression pipeline. The source's copyright, @license and @preserve comment gaps are retained in foreign module output. Review the package's distribution license requirements for a release.

A packaged Wasm asset adds only the wasm-unsafe-eval CSP keyword, never general unsafe-eval. The policy remains unchanged for applications without packaged Wasm. No SDK is automatically loaded by an application that does not import it.

The optional official SQLite test compiles a Brain worker module importing the SDK and native sqlite adapter, builds its assets, then opens the compiled frontend with an owned local Chrome profile and verifies a SQL row through the compiled worker bridge. It requires BRAIN_SQLITE_SDK_FIXTURE pointing to an explicitly verified extracted package and is marked ignored in the default offline suite. Run it explicitly with the ignored-test flag; a default workspace pass does not establish that this optional browser check ran. No provider SDK code is executed on the build host merely to build the graph.

## Explicit immutable CDN fetch

`brain deps fetch --request '<json>'` accepts `url`, `package`, `version`, `sha256` and `expectedRevision` (null only when the lock does not exist). It downloads one explicitly reviewed immutable HTTPS JavaScript file, verifies its SHA-256, and stores the bytes at `vendor/brain/<sha256>.mjs` with a versioned `brain.dependencies.json` entry. Existing unknown lock fields survive. Updating an existing lock requires its current revision; fetch never installs packages or executes SDK code.

The allowlist covers exact file/version paths on cdn.jsdelivr.net, unpkg.com and esm.sh. Redirects, query strings, fragments, user info, custom ports, encoded paths and mutable version aliases are refused. Requests have a ten-second deadline and one-MiB response cap; JavaScript MIME, UTF-8 and the declared hash must match. A single-file artifact containing further imports or resource URLs is refused: use the installed npm graph for such SDKs. This command does not promise arbitrary CDN graph traversal.

App authors can import the exact locked URL or the returned local artifact path. The checker permits only URLs imported by the current component whose local regular, non-symlinked artifact matches the bounded lock and SHA-256. Missing, altered or invalid artifacts retain B0122. The packer rechecks the lock and bytes before emission. Neither checking nor building performs network fetches; explicit fetch is the only network step. Hash verification establishes byte identity with the selected revision, not a security review of third-party code.

Operational evidence: the official `@googlemaps/js-api-loader` 2.1.3 distribution was explicitly fetched as an 8,047-byte self-contained ESM artifact, matched the verified npm archive SHA-256, and reused from the local cache on replay without executing provider code. Optional SDK and browser tests remain separate from the offline default test suite.

The full compiled SQLite browser proof also serves every generated production `_headers` rule. It verifies the packaged Wasm CSP permission without JavaScript unsafe-eval and completes a SQL query through the compiled worker. The measured production fixture reports COOP same-origin, no COEP and crossOriginIsolated false. This proves the SQLite worker path under the generated production policy; it does not establish persistent OPFS availability under that policy. Persistent OPFS requires the appropriate isolated deployment and remains a separate explicit browser/storage verification.
