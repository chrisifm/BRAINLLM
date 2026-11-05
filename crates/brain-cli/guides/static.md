# guide: static — portable HTML sites without a server

## When
Use BRAINLLM for a website whose complete pages can be generated at build time: landing pages, documentation,
portfolios, product information or content collections. The host only needs to serve HTTP files and directory
indexes. BRAINLLM, Rust and Node are build tools; they do not need to be installed on the host.

## Working loop
1. Run `brain new my-site --static`, then enter my-site. Read `brain spec` and this guide.
2. Keep `"output": "static"` in brain.json. It applies to `brain build`, `brain doctor` and MCP `brain_build`.
   `brain build --static` also enforces it on an existing project without editing its configuration.
3. Write public routes in views/, components in comp/ and ui/, assets in public/, and local collections in content/.
   Set the real PUBLIC_SITE_URL in .brain_env, or pass the real absolute origin with `--site`.
4. Give every page useful `<meta seo>` text, semantic headings, descriptive links, alt text and accessible controls.
   Read `brain guide seo` for canonical URLs, JSON-LD, social tags, sitemap, robots and assistant-readable files.
5. Run the verification commands below. Preview dist/ with a generic HTTP file server, including direct nested
   URLs and refresh. Then upload **the contents of dist/** to the host's document root.

## Output contract
- The root route becomes dist/index.html; /about becomes dist/about/index.html. The not-found view becomes
  dist/404.html. A one-page source produces one page plus its assets and SEO files; no single-file inlining mode
  is implied. The normal .html, .js and .css extensions are required, even if brain.json selects ext:brain.
- Wholly zero-JavaScript exports omit unused compiled framework modules; explicitly linked public downloads remain.
- Pages contain prerendered HTML. A page with no browser behavior downloads no page JavaScript. Signals,
  events and browser-only components remain available; the build bundles the JavaScript they actually need.
- The existing production pipeline minifies HTML, CSS and JavaScript, hashes assets, applies budgets and emits
  gzip/Brotli siblings when smaller. Ordinary files remain present: a host can ignore compressed siblings.
  Precompression negotiation needs host support; merely uploading a .br file does not enable compression.
- Generated text includes format-valid compiler attribution. brain-build.json identifies the generator for binary
  assets and compressed variants; copied assets retain their byte identity and original licenses.
- No server.mjs or _server/ artifacts are exported. A successful static rebuild removes stale server output
  and old chunks from dist/. Upload a complete build together rather than mixing revisions.
- _headers, _redirects and _brain/ host configurations are optional deployment hints. HTML and assets do not
  depend on the host understanding them. HTTP redirects, security/cache headers and a real HTTP 404 status
  still require the corresponding host configuration. Inspect the generated policy before deployment.

## Data and routes
A literal page may use a build-time content-collection load. A dynamic route must export paths() to enumerate
its URLs; load(ctx) for each listed URL runs during the build. Build-time seo() and head() are supported.
URL segments with spaces, Unicode and literal percent signs map to decoded filenames; encoded separators and
dot-segments are rejected. Avoid names that differ only by case or Unicode normalization; case aliases and
collisions on the build filesystem fail before publishing. Use a catch-all for multiple path segments, not a slash inside a simple parameter.
Rebuild when data changes. Request-time load(), guards, middleware, server: functions, server forms, live
sitemap(), native backend contracts, sockets and access policies require an application server and fail static
export. The build never removes a guard just to make the export succeed. Failed prerenders and invalid route
parameters also stop export. Explicitly returning an empty paths list generates no pages for that route.
Browser fetch() to an external service still depends on that service and its CORS policy. Static export cannot
prove that arbitrary authored JavaScript is offline or free of external services. Never put private keys in it.

## Verify
```sh
brain check
brain test
brain doctor --json
brain build --static --site https://example.com --report
```
Static doctor adds a `staticArtifact` gate after its fresh build: checks every emitted HTML/SVG reference, srcset,
CSS URL/import, literal module import and new URL resource against plain-host files, anchors, SVG symbols and final
SHA-384 integrity, duplicate IDs and ARIA/HTML document ID references. Missing files or ID targets, duplicate IDs,
server artifacts, invalid references or incomplete analysis fail the gate. It reports
unchecked external origins; dynamic URLs and interaction-dependent requests still need browser tests.
Template contents do not belong to the document ID scope; test their instantiated fragments and shadow roots in a browser.
Replace example.com with the real public origin. `brain build --static --json` returns outputMode:static;
MCP clients can create a starter with `brain_new` and `{ "name": "my-site", "static": true }`, or call `brain_build` with `{ "static": true, "site": "https://example.com" }`.
The build needs Node for prerendering. The deployed output needs only an HTTP static host with index.html support.
Use brain.json base for a subdirectory deployment; do not assume opening an HTML file through file:// is hosting.

## Common mistakes
Doctor builds/audits isolated temporary output, then removes it. It preserves dist/; build explicitly to deploy.
`brain reload` reads the existing build; after editing source, build again first. Its bounded timestamp advisory can
flag newer sources or incomplete analysis, but does not compile or prove source/output hash consistency.

- Exporting the default full-app starter, which includes protected account/login examples: start with --static,
  or deliberately replace server features first. --static and --backend cannot be combined.
- Symlinks or special files in public/: copy intended ordinary assets into public/; builds reject these inputs.
- Uploading the project folder: deploy dist/ only, never source, .brain_env, .brain or node_modules.
- Expecting request-time sessions, native API routes, runtime secrets or server-rendered personalization on a file host.
- Treating 404.html or a redirect file as a universal hosting API: verify the provider's status and routing behavior.

## Customize the build

Keep the normal output by omitting `staticOutput`. To choose generated folders and separate stylesheets:

```json
{
  "output": "static",
  "staticOutput": {
    "js": "src/js", "css": "css", "images": "img", "fonts": "fonts", "assets": "src/assets",
    "inlineCss": false, "compression": "small"
  }
}
```

`index.html` stays at the export root; page URLs and directory indexes keep their route structure. JS includes
workers and complete versioned SDK trees; packages retain relative asset relationships inside that tree. Images
places hashed image copies in that folder and generated social cards below its og/ subfolder. Other generated
assets include imported binaries and fonts. Raw public JS/CSS keep their authored directory so relative imports and
CSS URLs still work; they use revalidation rather than an unsafe relocated immutable copy. Original public/ files keep their authored paths for external links
and streaming playlists: place them in public/img/ or public/src/ when those stable URLs are desired.
Directories are relative URL-safe paths; `.` means the export root. Traversal, reserved paths and output collisions
fail before writing the artifact. The layout also applies to doctor and MCP builds. References, hashes, SRI,
preloads, compressed copies and the payload ledger follow the actual destinations.
`inlineCss` defaults true (small sheets may be inline); false keeps generated sheets in the CSS folder. Dynamic
style attributes remain dynamic. `compression` defaults small; fast reduces build-time encoding work while still
keeping raw files and useful gzip/Brotli/zstd variants. Budgets use actual resulting bytes under either profile.
`fonts` optionally separates hashed fonts from other assets (otherwise they follow `assets`).
`precompressed:false` omits generated .gz/.br/.zst siblings. `hostFiles:false` omits deployment configuration hints;
configure security headers and redirects on the host. Neither setting removes authored public files or required Wasm.
`cache.sw:false` keeps a small sw.js cleanup worker so previous visitors unregister an older worker and its caches;
new pages do not register a caching worker. Deleting that cleanup file could strand an older offline deployment.
Use existing settings alongside the layout: `base` for hosting below a URL prefix, `.brain_env`/--site for the
public origin, `seo` for canonical/trailing-slash policy and metadata, and `cache` for HTTP/SW/offline policies.

## Discovery for assistants and search

The standard export includes llms.txt (page index) and llms-full.txt (readable page text), robots.txt, sitemap.xml,
feeds and Markdown copies. They are generated from the real routes and rendered content, including custom layouts.
The LLM discovery files are enabled by default (`seo.llms` and `seo.llmsFull`); do not hand-maintain duplicate lists.
Use `seo.siteName` for the public display name independently of the package name. With `seo.markdown:false`,
llms.txt links to HTML pages instead of omitted Markdown copies. `seo.manifest`, `seo.feed` and `seo.sections`
control their generated files and matching links. Set the real public origin for absolute sitemap/canonical links. Review `seo.ai` when choosing crawler access.
For a deliberate link to the live site on the same origin, use an absolute HTTP(S) href with `rel="external"`.
Doctor lists that origin as unchecked; this does not exempt relative broken links, images or other assets.

## Minimal correct example
Create views/contact.brain, link it from the navigation, and rebuild:
```brain
<meta seo title="Contact" description="Contact our team for information about the project, its documentation and the next steps for getting started." canonical="/contact">
<main><h1>Contact</h1><p>Find our contact details on the <a href="/about">about page</a>.</p></main>
```
