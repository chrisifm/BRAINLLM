# Site-level SEO: focus terms and published URLs

Both are optional; without them nothing changes.

## Site focus
```json
{ "seo": { "focus": ["offline planner", "field reports"] } }
```
Up to three terms the site wants to be known for. `brain seo audit` shows, per term, where it appears, which page is its
main page (the term in the title and the H1), how many links reach that page and how many use the term as their text. Case
and accents do not count (Latin scripts). It reports a term missing from the home page (B1862), a term with no main page
(B1863) or several (B1864), pages that mention the term without linking to its main page (B1865, info) and a term in nearly
every title (B1866). More than three terms is B1861.

Be clear about what this is: an editorial consistency check, not a ranking factor. Google documents crawlable links,
descriptive anchor text and keyword stuffing as spam; it does not document this scheme, and no finding here promises a
position. B1866 only ever warns, and the thresholds are heuristics.

## Published URLs
`brain seo urls --write` creates `seo-urls.txt`, a committed list of the URLs you have published. From then on `brain build`
fails with B1879 when a listed URL disappears with no redirect in `redirects.json`, and warns about indexable pages not in
the list yet. Unlike B1826, which compares with the previous `dist/`, it also works from a clean clone. Without the file
nothing changes. To retire a page on purpose, remove its line.
## Live preview and doctor measurements

`brain seo preview /route --url https://example.com --json` previews one anonymous HTTP response:
search-result text, social metadata, JSON-LD and a live audit of that response. It works without local
`dist/`, including known per-request URLs. It requires HTTP 200 HTML and does not follow redirects,
execute browser JavaScript, fetch other pages or claim search-engine rendering. A response is bounded
to 8 MiB and thirty seconds. Use `seo audit --url` for a broader explicitly sampled site audit; preview
rejects audit-only sampling flags instead of silently ignoring them.

`brain doctor` measures the same production inputs and payload bytes as a normal `brain build`.
When the configured site is a placeholder, only SEO receives a separate private origin model to avoid
repeating B0736 as unrelated metadata penalties. JSON identifies `seo.outputScope` as
`placeholder-origin-model`, `matchesProductionBuildOutput: false` and `usedForPayloadBudget: false`.
The actual production artifact supplies the build signature, payload budget and static consistency
audit; the model never replaces deployment files or raises a budget.

## Sitemap comparison and sample coverage

`brain seo diff --from previous-sitemap.xml --json` compares a saved sitemap with the app's existing
`dist/sitemap.xml`. An explicit HTTP(S) sitemap URL is also accepted. No rebuild or file write occurs. Comparison
uses exact path+query membership, so a host migration alone does not lose URLs. Each inventory must be internally
consistent about its origin. Missing URLs are sorted; exits are 0 without losses, 1 with losses and 2 for invalid
input or incomplete observation. This is not proof of content, canonical equivalence, redirects or search indexing.

Inputs support ordinary unprefixed sitemap urlsets or one-level indexes. They reject DTDs, CDATA, nested indexes,
incomplete entries and symlinks. Limits are 8 MiB per file, 32 MiB total, 128 children and 500,000 combined entries.
Anonymous HTTP requests are sequential, never retry or follow redirects, and use three-second timeouts within a
thirty-second overall deadline. Remote children must share the index origin. Local indexes never fetch remote
children: their URLs map to adjacent local files using the public directory inferred from the first child. Preserve
this saved layout; encoded child paths and traversal outside that directory are refused.

Live `seo audit --url ... --sample N` sorts and deduplicates URLs, groups by first path segment and depth, then samples
groups round-robin. Every group is represented when N covers the group count; `--sample 0` keeps all distinct URLs.
These are visible URL families, not inferred route templates. Sitemaps do not reveal response sizes, so this does
not promise to select the largest pages.

Existing JobPosting B1818 warnings also review a title against its declared company/location using whole token
sequences, ignoring short or generic location values, and flag excessive repeated punctuation. These are review
heuristics without automatic edits; ordinary slashes and hyphens remain valid. Follow
[Google's JobPosting title guidance](https://developers.google.com/search/docs/appearance/structured-data/job-posting).

### Live crawl limits

Live audit shares a 300-second HTTP observation budget across sitemap traversal, response reads, probes and pacing, with at most 4096 requests and 128 MiB response bytes in total (8 MiB per response). Requests use no environment proxy, redirect retry or authentication; page redirects remain findings, while sitemap redirects are followed at most five times on their original origin. Child sitemaps stay under the audited origin/base, with at most 128 children and one index level. Inventory is capped at 100000 URLs.

`--sample` accepts 0–1000 (default 30); zero requests all inventoried pages only when there are at most 1000. Explicit `--path` batches have the same 1000-page cap. `--delay-ms` accepts 0–10000 (default 2000) and counts toward the shared budget. Exceeding a bound exits 2 with an incomplete observation error, rather than returning a passing truncated report. Split large crawls into explicit smaller path batches. Static analysis of the fetched bounded documents follows the HTTP phase; the timeout is not a browser-rendering or external DNS cancellation guarantee.
