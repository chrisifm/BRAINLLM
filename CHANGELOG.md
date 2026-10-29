# Changelog

All notable changes to BRAINLLM are recorded here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
and versions follow [Semantic Versioning](https://semver.org/). While the major version is 0, a minor bump may add
diagnostics that reject code an earlier version accepted.

Every npm package (`@christianirack/brainllm`, its seven native platform packages, `brainllm-runtime`,
`create-brainllm` and `create-brainll`) ships the same version as `[workspace.package]` in `Cargo.toml`.

## [Unreleased]

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

[Unreleased]: https://github.com/chrisifm/BRAINLLM/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/chrisifm/BRAINLLM/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/chrisifm/BRAINLLM/releases/tag/v0.1.0
