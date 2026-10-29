# Changelog

All notable changes to BRAINLLM are recorded here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
and versions follow [Semantic Versioning](https://semver.org/). While the major version is 0, a minor bump may add
diagnostics that reject code an earlier version accepted.

Every npm package (`@christianirack/brainllm`, its seven native platform packages, `brainllm-runtime`,
`create-brainllm` and `create-brainll`) ships the same version as `[workspace.package]` in `Cargo.toml`.

## [Unreleased]

### Documentation

- Site and README: the title, hero, metadata and footer say up front that BRAINLLM is a compiler written in Rust.
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
- A page URL requested with `Accept: text/markdown` (preferred over `text/html`) answers its Markdown twin in
  `brain start` and the static server, with `Vary: Accept`; browsers still get HTML. `seo.markdown=false` turns it off
  together with the `.md` copies.

### Fixed

- An app whose name is a runtime helper (`brain new t`) builds again: a word `t` in server text or `logout(` no
  longer imports brain-i18n.js on the server.
- `<meta seo schema="Article">` on an SSR page becomes one JSON-LD node built from that request's title,
  description, canonical, `og:image` and `lastmod`; `brain start` used to write a `<meta name="schema">` tag and keep
  the build's stale node. The build's node now carries `headline` for article types, `image` and `dateModified`, and a
  leftover `<meta name="schema">` (a view's own `<head>` block) is turned into JSON-LD by the SEO step.

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
