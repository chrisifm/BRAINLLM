# Changelog

All notable changes to BRAINLLM are recorded here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
and versions follow [Semantic Versioning](https://semver.org/). While the major version is 0, a minor bump may add
diagnostics that reject code an earlier version accepted.

Every npm package (`@christianirack/brainllm`, its seven native platform packages, `brainllm-runtime`,
`create-brainllm` and `create-brainll`) ships the same version as `[workspace.package]` in `Cargo.toml`.

## [Unreleased]

### Added

- Backend contracts accept exact `int`, `decimal(p,s)`, `date`, `datetime`, `enum(a|b)` and `list<str[,N]>` fields,
  object field options `{type, default, unique}` and declarative checks in `constraints.jsbrain`. Decimals travel and
  are stored as canonical strings with a lossless database CHECK; unique and check violations answer 409/422. Records
  expose `created_at`/`updated_at`. Existing contracts record byte-identically and gain no database objects.

### Changed

- The shared lexer decodes ASCII directly, scans ASCII identifier runs as bytes and indexes punctuation by first byte
  in the same longest-first order. Source-map scanning skips slash-free trivia gaps and builds its generated line index
  only to locate a failure. Emitted JS, server JS, CSS, maps and diagnostics are byte-identical; no compiler-speed
  claim is made until paired measurement.

### Tests

- Pin that a native socket rejected at the handshake stops retrying within its bounded budget.

## [0.5.0] - 2026-10-30

### Added

- Opt-in `brain add resource ... --admin-ui` prepares project-owned, role-guarded list/detail/create/edit views,
  progressive forms and missing Design System controls in one revision-checked change plan. Existing files are
  never overwritten. The fixed reference edit fixture has no headroom at 30,720 B; arbitrary layouts are not
  guaranteed to fit that budget.
- Experimental native invalidation sockets with explicit exact-Origin configuration, persisted-session/JWT
  authentication, row/role revalidation, bounded rooms and graceful shutdown. Events carry invalidation metadata,
  not records or a durable replay log; this is a single-backend-process capability.
- Development `liveResource(resourceHandle, options)` attaches owner-scoped, coalesced HTTP refresh to those
  metadata events. Public principal changes release the previous connection; unmount releases owned timers and
  handles. Six actual compiled native/browser cases passed on the release binary.
- One `brain.json` declaration feeds native HTTP/socket origins, private build metadata and fixed development,
  static-production and Node gateways. Explicit false overrides disable Node activation. Disabled projects do not
  acquire a socket listener or live runtime asset.
- nginx/PM2 templates route opt-in sockets through the appropriate fixed Rust or Node authority. Exact route,
  query/method/protocol checks, credential projection and disabled-topology compatibility have component coverage.

### Fixed

- Development thread-admission failures return actionable startup errors or best-effort503 responses instead of invoking the CLI panic hook. Admission permits recover and repeated diagnostics are coalesced; the64-connection cap and production worker behavior remain unchanged.
- Development imports of invalid value modules now reject with their original compiler diagnostics instead of returning a component-shaped fallback. Component fallbacks, durable agent feedback and recovery remain intact; static named-import linking retains its explicit limitation.
- Accept the existing GET `cache: 'no'` runtime contract in the checker, including when app-wide data caching is disabled. Keep that explicit endpoint privacy intent instead of suggesting its removal.
- Source-map scanning requests validated token boundaries without copying discarded identifier and literal values.
  The parser keeps its normal token payloads; marker ownership, source maps and generated-code diagnostics remain intact.
- Build DOM access-path strings by appending instead of repeatedly copying their growing prefixes. Generated traversal expressions remain byte-identical; this is not a claim that browser sibling traversal is already linearized.
- Doctor retains fresh build measurements when the payload budget fails, runs SEO against those current pages, and keeps its overall build gate failed. Source failures never reuse stale SEO output; absent test files no longer appear as a measured zero-leak result.
- Future release publication requires the same-commit source verification job to pass, including when reusing reviewed package artifacts. Canceled runs and manual pack-only dispatch cannot publish.
- Release change-metadata, preview-admission and legacy-writer locks explicitly when their leases end,
  including when a forked child temporarily retains the same descriptor. Contention still fails immediately;
  interrupted lock syscalls retry at most three times and other errors preserve their safe error category.
- Release the project-manifest writer lock explicitly when its lease ends. A child forked concurrently by the same
  process (for example an MCP session or a parallel test) no longer makes the next metadata refresh report busy.
- Preserve author strings, template text, regular-expression literals and generated HTML containing source-marker
  lookalikes such as `/*@L71*/`. Source-map stripping recognizes only exact compiler marker comments in the
  framework lexer's trivia; literal tokens remain data, including inlined components and public environment
  constants. Author JavaScript comments are discarded by the parser before emission and cannot redirect maps.
  Parenthesize regex-leading expression statements so compiler-added blocks preserve lexical context. Generated
  syntax that cannot be scanned now stops compilation with an explicit generated-location diagnostic.

### Internal

- Add an opt-in Node Upgrade bridge for native invalidation sockets, including private production/helper
  packaging and development sidecar copies. It delegates protocol authentication and authorization to Rust,
  preserves trusted Origin and native credentials, and bounds connections, buffering and lifetime. Public
  project configuration, browser subscriptions and Rust gateways are integrated and covered by the combined source
  gates and the native/browser acceptance run. The private manifest boolean and explicit overrides are
  strictly typed; helper presence does not activate transport.

### Upgrading

Socket configuration is disabled by default. To opt in, declare canonical `backend.origins` and its explicit
`sockets.allowedOrigins` subset, select the public site URL, and configure a fixed private backend authority.
Development/test servers fail if their actual Origin is not declared; an automatically chosen port is never
silently authorized. Access-token renewal before socket reconnection is not automatic. Administrative views are
an explicit generator flag and preserve existing project sources.

## [0.4.0] - 2026-10-30

### Added

The following 34 points are the detailed feature inventory for this release. Deferred work is labeled explicitly.

1. **Project-owned Design System foundation.** New projects ship editable design tokens and agent instructions for consistent colors, typography, spacing, radii and component organization. Blueprint, seed, configuration and adoption tools extend the project's own CSS system without installing a CSS framework.

2. **Editable basic components.** Project-owned controls expose explicit prop contracts and loading, busy, disabled and validation states. Numeric Input defaults use step="any" only for number controls; existing controls are preserved.

3. **Component gallery and manifests.** A searchable static gallery and per-component manifests expose component contracts, sources and design references. Agents can discover reusable UI without reading every implementation file.

4. **Mobile-first design guidance.** Foundations and generated guidance address responsive layouts, safe areas, touch controls and mobile input behavior. Exact, unambiguous design-token reuse suggestions remain author-reviewed warnings rather than automatic cascade rewrites.

5. **Guarded upgrades and design adoption.** Upgrade previews inventory existing files, preserve unknown configuration/design fields and prepare revision-checked package-pin edits. Design adoption is separate; installation and behavior migrations remain explicit decisions.

6. **Scoped CSS deduplication.** Component styles retain encapsulation while equivalent declarations can share compiler-owned output under compatible selector/context/order rules. Small semantic differences and public contracts keep distinct output.

7. **Production cleanup with agent feedback.** Proven unused bindings, imports and declarations are removed during production compilation, with structured source-cleanup feedback. Observable effects, throwing reads, live bindings and module evaluation are preserved; this is not a claim of complete ESLint/Stylelint coverage.

8. **Deterministic compilation and bounded caches.** Runtime pruning converges in stable bounded passes rather than hash-map iteration order. Content/configuration/compiler-aware caches reject unsafe or incomplete entries, and source reads have explicit file, byte and time bounds.

9. **Zero executable JavaScript for eligible static pages.** Successful static prerenders can emit HTML/CSS without hydration or router startup, including in mixed sites. Interactive, guarded, request-dependent and server-rendered routes retain the execution they require.

10. **Final-asset byte accounting.** Build ledgers attribute emitted raw/gzip/Brotli JS and CSS assets and distinguish initial, deferred and conservative route references, including .mjs modules. Asset accounting is separate from observed network transfer, browser execution time and per-rule compressed-byte provenance.

11. **Predictive navigation workers.** Page-local next-route/asset plans support bounded speculative fetching from link visibility and user intent. Time/byte/concurrency limits, session-write invalidation and cancellation keep speculation within its declared lifecycle.

12. **Immutable assets and precompression.** Content-hashed assets retain immutable caching. Build-time Brotli quality 11 and gzip results are cached by content/settings; native Rust and Node delivery negotiate encoding weights and explicit exclusions without applying static-asset caching to private HTML or APIs.

13. **Fragment hot reload.** Proven-compatible template text and attribute edits preserve live nodes, focus and state. Changed wiring and unproven fragment updates use an explicit fallback; development proof metadata is excluded from production output.

14. **Native types and browser-target diagnostics.** Conservative inference connects declarations, signals, schemas, API bodies and form bindings with mechanical warnings B2001-B2005. Pinned Baseline evidence adds B2101-B2103 for app code; unsupported browser globals remain visible warnings, and unknown flows are not presented as proven types.

15. **Experimental Rust/Postgres backend.** Literal resource schemas and policies drive validation, CRUD, search and OpenAPI. PostgreSQL migrations are explicit and checked; the HTTP/database stack uses maintained Rust infrastructure while the compiler keeps its own parser, bundler and minifier.

16. **Persistent native sessions and JWT.** Argon2id authentication, revocable persisted sessions and optional access/rotating-refresh JWT sessions provide native identity. The backend chooses the configured mode; failed JWT authentication never falls back to another provider.

17. **One native sign-in authority for SSR, RPC and forms.** ctx.backend.login relays validated native HttpOnly cookies and returns only the canonical sanitized user from a same-origin POST server function. New sign-in pages share this flow with JavaScript and progressive no-JavaScript submission; deployed runtimes cannot activate the development demo implicitly.

18. **Google sign-in, recovery, verification and roles.** Google nonce/JWKS verification, password recovery, email verification and configured roles have project-owned recipes. Optional Resend delivery requires private operator configuration, and first-admin promotion is an explicit command rather than a signup privilege.

19. **Request-scoped server services and repositories.** ctx.backend.resource exposes list/get/create/update/remove through the same native schemas, policies and cookie identity used by the API. Operations share bounded transport/cancellation; writes require same-origin POST and are not retried automatically.

20. **Native resource support generation.** brain add resource creates schema/policy/hooks/search contracts, typed collection/detail clients, server services/repositories and a discoverable schema-revision snapshot. It preserves routes, guards, controls and budgets; administrative UI is marked planned and its unfinished renderer is excluded from the public generator.

21. **Enforced hooks and persistent queues.** Configured before-write hooks run through bounded private Node executors on Unix, revalidate returned data and reject same-record conflicts. After-write events commit with the mutation in a PostgreSQL outbox; leased workers, retries and terminal states provide at-least-once delivery, so handlers must be idempotent.

22. **Signed webhooks.** Inbound HMAC verification checks event identity, timestamp and replay state. Durable encrypted payloads, signed outbound deliveries, bounded retries and validated HTTPS destinations keep provider integration explicit; configuring or building a project does not send external messages.

23. **Search, slugs, short links and HTTP helpers.** Explicit public search fields use PostgreSQL full-text/trigram indexes. Native utilities also cover slugs, authenticated short-link creation, validated redirects, status/response helpers and safe default error pages with real HTTP statuses.

24. **Private image library and optional cloud storage.** Content-validated originals, bounded renditions/crops, versioned ownership metadata and leased cleanup support native media. Optional Google Cloud Storage uses an official-SDK adapter and explicit credentials/configuration; it does not create buckets or upload during scaffolding.

25. **Concurrent agent changes.** Per-file leases, SHA-256 revisions, checked plans, individual atomic replacement and conditional recovery protect cooperating agent edits. Multi-file changes are recoverable logical transactions, not an instantaneous filesystem transaction; external-editor races remain explicit.

26. **Native CLI and MCP discovery.** New projects include Claude Code/Cursor MCP registrations, CLAUDE.md importing AGENTS.md, optional edit-time checks and compact tool profiles. Component search, contracts, graph/context/impact/why tools provide structured navigation; registration is distinguished from a verified live client connection.

27. **Actionable error feedback.** Structured diagnostics preserve known source/generated locations, bounded frames, causes, mapping precision, truncation and request references. Durable compiler feedback reaches local agents before browser overlay publication; production public messages exclude raw traces, internal paths and arbitrary exception messages.

28. **Quality panel and project history.** Private JSON/Markdown/panel output exposes code-quality observations, proposed fixes and acceptance evidence. Project-local tasks, history and lesson proposals distinguish reported claims from verified results; they do not imply model retraining or automatic external issue publication.

29. **Native form validation and runtime safeguards.** Required/blank/type checks, field errors and duplicate-submit locking retain client/server consistency. Proven flat literal forms can use smaller capability-specific runtime paths, while dynamic/async validation remains available; owned reactive/worker budgets do not claim to predict every leak or stop all foreign code.

30. **Shareable public navigation state.** Selections and native dialogs can synchronize declared public state with URL parameters and restore direct links/back/forward while preserving unrelated parameters. Credentials, private form values and transient state are excluded from this navigation contract.

31. **Browser data and worker capabilities.** IndexedDB records, small public preferences/cookies and an optional official SQLite Wasm/OPFS adapter support bounded local storage. Dedicated/shared worker RPC and explicit worklet, transfer, shared-memory and canvas capability checks preserve browser prerequisites and fallbacks.

32. **Official integrations and selective dependencies.** Discoverable OpenAI/Claude/Gemini and Google-service adapters favor official SDK compatibility and selective installation/upgrades. Installed ESM imports and explicit integrity-pinned CDN acquisition use the compiler graph with side-effect preservation; native date/hash helpers and library tools are also discoverable.

33. **PM2 and nginx deployment configuration.** Reviewable configurations cover static/Node SSR frontends, optional Rust backends and supervised queue workers with fixed loopback upstreams and graceful lifecycle settings. Four topology configurations passed actual PM2/nginx parsers; syntax validation is not a claim that a public host was activated.

34. **Agent-oriented end-to-end and benchmark tooling.** Detected connected Codex Viewer/Claude Viewer adapters support scenario execution, explicit profile selection and owned-tab cleanup; headless accessibility/optional heap and engine-matrix checks are complementary evidence. Provider-neutral LLM benchmarks record explicit models, frozen spec/binary hashes and real first-pass outcomes, rejecting compiler crashes or malformed diagnostics.

### Changed

- Production public error output uses safe references; raw traces remain in private diagnostic channels.
- Runtime export pruning now converges in stable bounded passes instead of depending on hash-map iteration order;
  removed functions no longer permanently retain another runtime module's unused parser or core exports.
- Server-rendered view styles use immutable compressed stylesheet assets, separate from personalized HTML.
  Proven flat literal form rules use tracked specialized validators; dynamic/bounded/nested schemas retain validation.
- JavaScript ledgers include emitted `.mjs` SDK modules and their static dependencies.
- Safe HMR template text/attribute edits preserve live nodes, focus and state; incompatible wiring has an explicit fallback.
- npm packages foreground the Rust-powered full-stack compiler and carry precise LLM, frontend/backend, CLI,
  Design System and testing discovery keywords, without claiming unrelated IoT, robotics or mathematics capabilities.

### Fixed

- Server module file URLs use platform-native path conversion: Windows verbatim drive/UNC paths and reserved filename characters no longer corrupt module specifiers; modification-time cache busting is retained.
- Native release smoke failures report their phase, allowed diagnostic/Node codes and known exit status before cleaning private scratch output.
- Resources with no configured search fields no longer fail their first write: additive migration 7 handles PostgreSQL's null trigger-argument array without changing prior migration checksums.
- Development HMR proof ranges account for prepended guard imports; location bookkeeping cannot mask the wrong generated string.
- Feedback writers explicitly release their own file lock even when a child retains a duplicate descriptor. Locale generation keeps its existing writer guard while applying translation fixes.
- Runtime npm tarballs retain their actual JavaScript modules alongside notices. Native packages carry verified Cargo/std/toolchain notice inventories, and publication includes native and anonymous-registry smoke checks.
- Decoded static routes cannot expose private server modules, environment files or deployment configuration; private Node traces discard arbitrary exception messages while preserving source frames.
- Native account adapters honor hosting prefixes, reject foreign application bases and never inherit external API origins.
- LLM benchmarks reject compiler crashes/invalid diagnostic output and use valid scaffold names for dotted model identifiers.

### Documentation

- The site hero shows the `npm create` command with a copy button (cbb0917).

### Upgrading

- New type/browser diagnostics are warnings in this minor development cycle. Inspect `brain preflight`; do not
  disable security diagnostics. `brain upgrade <exact-version>` prepares pin edits only; installation and semantic
  migrations remain explicit. Atomic plans require leases and exact revisions, including null for new files.
- Existing app: run `brain agent setup` to merge local MCP registrations; add `--hooks` only to opt into Claude Code
  edit feedback. Restart the client and verify its tools/hooks. Source installations must use the installed brain
  command in their MCP config; npm installations use the pinned local binary.
- New diagnostic B0734 is a doctor warning about missing local MCP configuration, not a failed connection test.
  B0735 rejects invalid opt-in resource/job schemas, policies or hook references. Existing B04/B05 severities do not
  change, and ordinary server folders without backend metadata keep their previous meaning.
- Connect the experimental backend through a fixed loopback BRAIN_BACKEND_URL for native proxy/SSR auth; migrate
  only the selected Postgres environment explicitly. Validate each existing production API slice before replacement. Account email needs explicitly configured delivery; admin provisioning is explicit. Native authenticated socket rooms and production Upgrade tunnels are deferred beyond this checkpoint.
- A fresh frozen LLM generation gate and production topology/latency benchmarks remain acceptance work for this slice.

### Measured migration and known limits

The real npm 0.3.1 compiler generated the migration fixture; its archive integrity was verified before execution.
The same source/data inputs retained `budget.viewKB: 30`. These are verified pre-release build-ledger snapshots,
not measured network traffic or a speed comparison:

| Fixture / compiler snapshot | Conservative `/login` bytes | Original 30 KiB budget |
|---|---:|---|
| Released 0.3.1 starter, compiler 0.3.1 | 26,984 | Pass |
| Same starter, initial 0.4.0 checkout | 32,782 | Fail |
| Same starter, optimized 0.4.0 checkout | 29,836 | Pass |
| New 0.4.0 starter, optimized checkout | 30,223 | Pass |

Totals include gzip JS/CSS plus raw personalized SSR HTML, including conservative deferred-code references.
The real adoption/CAS regression preserved 40 pre-existing non-package files, unknown configuration/design fields,
token/font bytes and opaque local data. It changed the package declaration without installing the destination package.
Two separate cold-cache compiler processes produced identical complete build files and signatures for the new starter.
These checkout snapshots are not release-binary performance benchmarks.

- Native authenticated sockets/rooms and their Node/Rust/nginx Upgrade integration are not part of this checkpoint.
- Source mapping is line-level where proven; source columns remain null when unavailable. Precise arbitrary minified-byte
  provenance and `brain explain-output <offset>` are not implemented. `brain explain <code>` explains diagnostics.
- General cross-module constant propagation, strength reduction, common-subexpression elimination and loop-invariant
  hoisting are not established capabilities. Existing cleanup/mangling is not proof of these mathematical passes.
- Asset ledgers measure emitted payloads and distinguish initial/deferred references; they do not prove browser timings,
  attribute every compressed byte to a CSS rule, or establish a universal advantage over another framework.

## [0.3.1] - 2026-10-29

### Changed

- Releases: every `v*` tag publishes the 11 npm packages through npm Trusted Publishing (OIDC, no stored token, no
  approval step); `docs/NPM-RELEASE.md` describes the new flow (57264a4).

### Documentation

- README, npm package descriptions and the site lead with "built in Rust" and the ~34 ms build benchmark; the npm
  README links the cover and the badges to the website.

## [0.3.0] - 2026-10-29

### Added

- `brain guide [topic] [--json]` and the MCP tool `brain_guide`: task playbooks for agents embedded in the binary
  (`start`, `views`, `data`, `forms`, `server`, `seo`, `a11y`, `webmcp`, `tests`, `i18n`, `performance`), each with
  steps, verify commands, common mistakes with the diagnostic that catches them and a minimal correct example. The
  `seo` guide starts from the target query and covers answer-first pages, FAQ, authors, freshness and the AI crawler
  policy. Tests reject a guide that cites an unknown diagnostic code or command. `brain new`'s AGENTS.md, SPEC.md
  and `brain --help` point to it.
- WebMCP runtime `tool(name, description, params, run, { title, readOnly, consequential, untrusted })` passes
  `title` and `annotations: { readOnlyHint, consequentialHint, untrustedContentHint }` to the registration (WebMCP
  draft 2026-10-27).
- B0411 (error): an `<a href>` with no accessible name (no text, `aria-label` or image with alt).
- B0412 (warning): link text that is only a generic phrase such as "click here" or "read more".
- B0413 (warning): an `alt` that is a file name or a lone generic word such as "image" or "logo".
- B0414 (error): an `<iframe>` without `title` or `aria-label`.
- B0415 (warning): a `<video>` with controls or sound that has no captions or subtitles track.
- `<meta seo keyword="…">` is part of the authoring contract: an indexable view without it is warning B0502, and
  B1816 now also asks for the keyword in the description (the home page `/` is not asked for it in its URL).
  `brain new`, `brain add view|collection`, the examples and the generated AGENTS.md carry a keyword.
- B1836 (info): a page without `keyword` whose title or description does not name its H1 topic ("Home" over an
  "Offline task planner" page). The `brain new` sample posts name their topic in the description.
- Every page gets one JSON-LD `@graph` tying `WebSite`, `Organization`, `WebPage` (name, description, language,
  `dateModified`, primary image, breadcrumb), `BreadcrumbList`, FAQ and `BlogPosting` together by `@id`; before,
  `WebSite` and `Organization` were on the home page only and nothing said which page a node belonged to. A type
  the author declares still replaces its automatic node.
- A page URL requested with `Accept: text/markdown` (preferred over `text/html`) answers its Markdown twin in
  `brain start` and the static server, with `Vary: Accept`; browsers still get HTML. `seo.markdown=false` turns it off
  together with the `.md` copies.

- Every built page names what built it: `<meta name="generator">` is followed by a comment with the BRAINLLM home page
  and repository, llms.txt and llms-full.txt end with a `## Built with` section, and `brain start` answers with
  `X-Powered-By: BRAINLLM (https://chrisifm.github.io/BRAINLLM/)`.
- `rss.xml`, a site-wide feed like WordPress `/feed`, plus `<section>/rss.xml` for every section without a content
  collection, and `sections.json`, one manifest of every section with its title, description, home page, feed and pages
  (URL, Markdown copy, lastmod). Every page links both. On by default; `seo.feed` and `seo.sections` turn them off.
- Predictive prefetch in the client router: internal links that scroll into view are fetched while the browser is
  idle, hover, focus or touch fetch a link first, and the next navigation swaps from memory with the page's chunks
  already preloaded. Any write (a form submit, an `api()` write, a server function call) forgets the fetched pages;
  files, `/api/`, `/auth/`, log-out, unsubscribe and confirm links and URLs with a token are never fetched, and the
  server's `load` sees `ctx.prefetch`. `data-prefetch="false"`, `<meta name="brain-prefetch" content="off">` and
  Save-Data turn it off. Pages no longer carry Speculation Rules: the router intercepts every click, so the browser's
  prerender was never used and only doubled the request.
- `ai`, a primitive for Chrome's built-in AI (Gemini Nano): `ai.ready()`, `ai.prompt`, `ai.stream`, `ai.session`,
  `ai.summarize`, `ai.translate`, `ai.detect`, `ai.write`, `ai.rewrite` and `ai.proofread`, feature-detected and
  `'unavailable'` in other browsers and on the server. `brain guide ai` covers when and how to use it.

### Fixed

- `brain seo audit` reads each view's `keyword`, so it reports the same B1816 findings as `brain build`.
- An app whose name is a runtime helper (`brain new t`) builds again: a word `t` in server text or `logout(` no
  longer imports brain-i18n.js on the server.
- `<meta seo schema="Article">` on an SSR page becomes one JSON-LD node built from that request's title,
  description, canonical, `og:image` and `lastmod`; `brain start` used to write a `<meta name="schema">` tag and keep
  the build's stale node. The build's node now carries `headline` for article types, `image` and `dateModified`, and a
  leftover `<meta name="schema">` (a view's own `<head>` block) is turned into JSON-LD by the SEO step.
- B1831 (questions without FAQ) no longer sells FAQPage as a Google rich result: Google stopped showing FAQ rich
  results on 2026-05-07; the markup stays because answer engines and Bing read it.
- `brain check` on a view outside any app no longer walks up to `/` looking for `_guard.jsbrain`; only the view's
  own folder is searched.

### Documentation

- The served spec (`brain spec`) points to `brain guide forms|server|seo` for form messages, sessions and live
  sitemaps instead of spelling them out, and says that a module the app does not have yet is declared in the file or
  created with `brain add`: Haiku 5.5 had dropped to 17/20 first-try passes on the 0.2.0 spec (bench/llm/README.md).
- Site and README: the title, hero, metadata and footer say up front that BRAINLLM is a compiler written in Rust.

## [0.2.0] - 2026-10-29

### Added

- `export async function sitemap({ env })` in a dynamic view without `paths()`: the server adds those live pages to
  `/sitemap.xml` (cached 5 minutes, capped at 50 000 URLs, `/sitemap-dynamic.xml` when the build wrote an index).
  `sitemap()` outside a dynamic view is B0117 (#8).
- `notFound()` from `load` (thrown or returned) answers 404 with `views/404.brain` in `brain start`, `brain run` and
  prerender; routes whose `load` can return it are not streamed (#7).
- B0133: replacing a form's `f.v` is rejected, because the inputs keep reading the original field signals (#6).
- Validation messages are localized through the app dictionary (`"form.required"` and friends in `i18n/<locale>.json`), falling back to English (#6).
- `brain schema` lists `seo.ai`, `seo.og`, `seo.search` and `seo.news` (#9).

### Fixed

- `<select value>` and `<option value>` keep their value in SSR, prerender and the browser (#7).
- `canonical` and `og:url` on dynamic pages use the request path instead of the route pattern; a view's head tags
  replace the layout's tags with the same name, property or canonical (#7).
- A client-side `_guard.jsbrain` that reads `ctx.user` falls back to a full page load so the server decides (#7).
- Logout no longer stalls in Chrome: session clearing sends `Clear-Site-Data: "storage"` only, and session-bearing
  SSR HTML is `private, no-store` (#7).
- llms.txt section headings use the title of the section's own page (#8).
- The `*` route and special `!` routes never appear in sitemap.xml or llms.txt (#8).
- The starter login button uses `busy={f.busy}`, so it disables while signing in (#9).

### Documentation

- SPEC: `ctx.session` and `ctx.env`, signed session cookies, server function error handling, `load` only in views,
  and that `brain run` does not render view markup on the server (#9).
- Site: a real-world testing section with the fixes above.

## [0.1.0] - 2026-10-28

### Added

- First public npm release for seven native platforms (macOS arm64/x64, Linux x64/arm64/musl, Windows x64/arm64),
  the runtime and the `npm create @christianirack/brainllm` and `npm create @christianirack/brainll` initializers (#5).

[Unreleased]: https://github.com/chrisifm/BRAINLLM/compare/v0.4.0...HEAD
[0.4.0]: https://github.com/chrisifm/BRAINLLM/compare/v0.3.1...v0.4.0
[0.3.1]: https://github.com/chrisifm/BRAINLLM/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/chrisifm/BRAINLLM/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/chrisifm/BRAINLLM/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/chrisifm/BRAINLLM/releases/tag/v0.1.0

<!-- Maintainer acceptance is tracked locally; this section describes implemented source, not a release verdict. -->
