# Public website

The landing lives in `docs/site/`. Its public URL is https://chrisifm.github.io/BRAINLLM/.
It is plain HTML, CSS and JavaScript with local artwork: no bundler, external font service or analytics.
The README and npm marketing copy reuse this identity. Asset prompts and usage are in [BRAND-ASSETS.md](BRAND-ASSETS.md).

## Preview and verify

Serve `docs/site/` with a local static server. Check desktop and narrow mobile widths, keyboard navigation,
reduced-motion settings, both clipboard success and fallback, image loading and every call to action.
The manual install commands are secondary; the agent prompt is the primary onboarding path.
Run `scripts/check-english.sh` and `node scripts/privacy-audit.mjs` before publishing.

## Publish

Only publish after the maintainer authorizes the website change. The `gh-pages` branch contains the contents of
`docs/site/` at its root, plus `.nojekyll`; it must never contain local environment files, build caches or private
app data. GitHub Pages is configured to serve `/` from that branch. Deployments do not publish an npm package or
merge compiler changes into `main`.

Use an isolated checkout of `gh-pages`, copy only the reviewed site files, commit and push without force. For the
first deployment, create that branch in an isolated repository. Keep the framework checkout intact. Record the
source commit in the deployment commit message. Confirm the Pages build succeeded and fetch the public URL before
reporting the landing as live. To revert, make a new deployment commit restoring the last verified site.

Update the repository homepage to the public URL. The principal-author review rule still governs framework
contributions. The website contributor section uses public GitHub profiles and a checked-in fallback snapshot;
see [CONTRIBUTORS.md](CONTRIBUTORS.md) for attribution updates.

## Keep claims reproducible

The visible build times describe the frozen dashboard benchmark, not every app or today's compiler revision.
Keep units, workload, version and source next to each measurement. A cache-key microbenchmark is not a build-time
comparison. Never substitute estimated token spending, savings or hypothetical speedups for recorded results.

When revising onboarding, sync the landing prompt and README. `npm/pack.mjs` generates the npm README from the
root README with absolute public links; published npm versions only update through a new release.
