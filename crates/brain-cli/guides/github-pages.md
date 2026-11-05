# guide: github-pages — publish a static site from its build folder

## When
Use GitHub Pages for prerendered public HTML, CSS, JavaScript and assets. It does not run the BRAINLLM server,
native APIs, sessions or arbitrary server headers/redirect rules. Read `brain guide static` first.

## Steps
1. Create a site with `brain new my-site --static` and keep output:static in brain.json.
2. Set staticOutput.host to github-pages. Keep index.html at the artifact root; do not create a repository-named
   folder inside dist. For a project URL https://example.github.io/project/, set base to /project/.
   For an account site or a custom domain mounted at its root, set base to /.
3. Set PUBLIC_SITE_URL to the HTTPS origin only, such as https://example.github.io. The separate base supplies
   /project/. Do not include it again in the origin. The build rejects paths, queries and fragments in this preset's origin.
4. Run `brain check`, `brain test`, `brain doctor --json`, then build with the real HTTPS origin.
5. Preview the contents of dist at the configured prefix using an ordinary static HTTP server. Check direct nested
   URLs, assets after hydration, the sitemap and a missing URL. Deploy dist contents, never the project source.

## Output
The preset emits dist/index.html, each route/index.html, 404.html and an empty .nojekyll marker at the root.
A custom 404 is preserved; a minimal noindex fallback is supplied if absent. The marker prevents a branch-based
Jekyll build from discarding underscore-prefixed asset folders such as _b. Existing custom asset folders still work.
No extra project/ wrapper, server executable, server.mjs or _server tree is needed or emitted.
Attribution stays on unless the user explicitly requests root compilerBranding:false; the workflow preview follows it too.
The preset defaults hostFiles to false: GitHub Pages does not apply _headers/_redirects. Compression and budget
defaults stay unchanged. Set precompressed:false only when raw-file budget measurements fit; uploaded .gz/.br
siblings do not themselves configure host negotiation. Explicit overrides do not add host capabilities.
Raw minified files and complete prerendered pages remain. A .nojekyll control marker stays empty; build metadata
carries compiler attribution without changing the marker's protocol.
For a configured custom domain, public/CNAME may contain that domain; configure it and DNS in GitHub separately.

## Publication workflow
`brain deploy config github-pages --json` prepares a revision-checked deploy/github-pages.yml proposal.
Review/apply it, then place the reviewed workflow in .github/workflows/pages.yml without overwriting a custom workflow.
Commit package-lock.json. In repository Settings > Pages, select GitHub Actions as the publishing source.
The workflow gets the actual origin/base_path from configure-pages, refuses a brain.json base mismatch,
installs the pinned compiler with npm ci, checks/builds the static site and uploads only dist with
upload-pages-artifact. A separate deploy job uses pages:write, id-token:write and the github-pages environment.
Protect that environment and review the configured main-branch/manual triggers before activating the workflow.
The preview does not write files, modify GitHub settings, push a repository or deploy anything.
For branch publication instead, commit the contents of dist, including .nojekyll, to the selected root or docs folder.
GitHub Pages controls error status, TLS and redirects; the compiler does not promise provider-wide custom headers.

## Verify
```sh
brain doctor --json
brain build --static --site https://example.github.io --report
brain deploy config github-pages --json
```
Doctor audits a fresh isolated static build. Review the actual deployment artifact too: .nojekyll, index.html,
404.html and assets must be present at its root, with /project/ used only in public URLs. Wait for the GitHub Actions
run and visit the public root, a nested route, a real asset and a missing page before claiming publication.

## Common mistakes
- Using / as base for a project site, or prefixing both the site origin and base with the repository path.
- Uploading dist as a nested folder in the Pages artifact instead of uploading its contents.
- Omitting .nojekyll from a branch deployment and losing _b files through Jekyll filtering.
- Expecting _headers, _redirects, server runtime code or authentication to work on GitHub Pages.
- Assuming a prepared workflow is an actual publication or changing an existing workflow without review.

## Minimal correct example
brain.json for https://example.github.io/project/:
```json
{"output":"static","base":"/project/","staticOutput":{"host":"github-pages","js":"js","css":"css","images":"img"}}
```
Build with `brain build --site https://example.github.io`. The dist root contains index.html and .nojekyll;
asset URLs start with /project/, while the artifact itself has no project/ wrapper.
See [GitHub's static generator guidance](https://docs.github.com/en/pages/getting-started-with-github-pages/creating-a-github-pages-site)
and [custom workflows](https://docs.github.com/en/pages/getting-started-with-github-pages/using-custom-workflows-with-github-pages).
