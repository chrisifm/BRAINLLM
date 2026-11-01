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
