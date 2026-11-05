# guide: seo — classic search and AI answers (GEO)

## When
For every indexable landing, article, docs, product or profile page; plan discovery before writing its text.

## 1. Pick the target query FIRST (one primary query per page)
1. Write the user's intent in one sentence: who arrives, what they want to know or do.
2. Write the words a person would actually type or ask an assistant ("how to split a bill offline", not "Bill
   Splitting Solutions"). Prefer the plain, specific phrase over the brand term.
3. Choose ONE primary query per page; two pages on the same query compete (B1817 cannibalization).
4. Declare it: `<meta seo keyword="split a bill offline">` on every indexable view (missing = B0502). The audit
   checks it in the title, the description, the H1, the first paragraph and the URL, at a natural density (B1816).
   Without a keyword, a title or description off the H1's topic is B1836.
   A view rendered per request keeps real pagination with `query="page"`: `/list?page=2` is its own canonical, while
   `?dialog=size` or `?utm_source=mail` keep the canonical of `/list` (B0503 reports a contradictory list).
   `/list?page=1` answers 301 to `/list`, `?page=abc` answers 404. `//list` and `/list/index.html` answer one 301.
5. Put it in: `title` (≤ 60 chars), `<h1>`, the first sentence, `description` (≤ 160 chars), and the route name
   (`brain add view split-a-bill-offline`). Lengths over the limits are B1602.

## 2. Write for answer engines
- **Answer first:** the first paragraph answers the query directly in ≤ 300 characters, then expands (B1830).
- **Questions as FAQ:** clear for readers and LLMs, though Google no longer shows FAQ results; FAQPage JSON-LD is generated:
  `<section faq><details><summary>Is it free?</summary>Yes, for up to ten people.</details></section>` (B1831).
- **How-to:** numbered steps under question or "How to …" headings; one action per step.
- **Entities:** name things exactly (product, place, standard) and link 1-3 authoritative sources where you state
  facts or numbers (B1834).
- **Author and trust:** articles name an `author:` in the frontmatter and its profile in `brain.json`
  `"authors": { "Ada Lovelace": { "url": "https://…", "sameAs": ["https://www.linkedin.com/in/…"] } }` (B1833).
  The site's own profiles: `"organization": { "name", "logo", "sameAs": [...] }` and `"social": { "x": "@acme", … }`.
- **Freshness:** show the date in the text `<time datetime="2026-10-01">October 1, 2026</time>`; collections set
  `date:`/`updated:` frontmatter (→ `dateModified`, B1832); view `lastmod="2026-10-13"` or `lastmod={props.updated}` reaches its prerender sitemap URL.
- **Depth:** no word count is required (B1808 only points at near-empty pages); internal links in and out (B1812 no links out, B1813 orphan page).
  A link to a page the build does not produce fails `brain build` (B1819).
  An absolute HTTP(S) anchor to another site sharing the origin may use `rel="external"`: native navigation, no prefetch. Relative links/assets stay checked; external destinations are not verified.
## 3. Declare the page
```html
<meta seo title="Split a bill offline" keyword="split a bill offline" canonical="/split-a-bill-offline" schema="Article"
  description="Split a restaurant bill between friends without network: …">
```
- `schema="Type"` writes JSON-LD with that `@type` plus name, description and url. A full object:
  `jsonld={{ '@type': 'Article', headline: props.post.title, datePublished: props.post.date }}` (B1603/B1818 without `@type`).
- Keep each attribute value on one line (newlines are kept in the output). Values may be expressions from `load` props: `title={props.user.name}`.
- Optional empty or whitespace-only SEO strings are absent, including derived description, image and date facts; explicit JSON-LD objects remain authored data.
- Social images starting with `/` follow the configured app base without duplicating it; bare relative images resolve against the canonical page URL, including its trailing-slash policy. Absolute URLs and existing percent escapes are preserved.
- Automatic breadcrumbs start at the page's language home, with no repeated default-language home; WebSite and Organization identities remain global.
- Site defaults once in `app.brain`: `<meta seo titleTemplate="%s · Acme" og:image="/og.png" twitter="@acme">`.
- Status codes from `load`: `throw notFound()` → 404, `throw gone()` → 410 (deleted for good), a renamed record
  `throw redirect('/posts/' + post.slug, 301)`. Never render an empty page with 200 (a soft 404; not machine-checked, it depends on the data).
- URLs: `/about/` answers one 301 to `/about` (`brain.json` `"seo": { "trailingSlash": "never" | "always" |
  "ignore" }`); `"canonicalHost": true` sends `www.` or old domains to `site` in the same hop. Moved pages go in
  `redirects.json` (`[{ "from": "/old", "to": "/new", "status": 301 }]`), never a meta refresh (B1855) or a script (`location.*`: B0129).
- `robots="noindex"` for pages that must not be indexed (403, thank-you pages). Other head tags: a `<head>` block in the view.
- `[id]` views: `export async function paths()` (prerendered, in the sitemap) or, for live pages,
  `export async function sitemap({ env })`. Never both (B0117). It returns one object per page keyed by the bracket name;
  `lastmod` is optional (`YYYY-MM-DD` or a Date); an item without the key is skipped. `views/blog/[slug].brain`:
  ```html
  <meta seo title="Blog post" keyword="blog post" description="One article of the blog with its title and a short summary.">
  <script>
    export async function load(ctx) { return { slug: ctx.params.slug } }
    export async function sitemap({ env }) { return [{ slug: 'hello-world', lastmod: '2026-10-01' }, { slug: 'second-post' }] }
  </script>
  <main><h1>Blog post</h1><p>{props.slug}</p></main>
  ```
  The server lists `/blog/hello-world` and `/blog/second-post` in /sitemap.xml (kept 5 min).
  A public dynamic view (no `_guard.jsbrain`, not `noindex`) without `paths()` or `sitemap()` is rendered per request
  but listed nowhere; `brain build` warns `B1837 views/blog/[slug].brain … dynamic route `/blog/:slug` …` with this fix.
- Public per-request pages without params are listed in /sitemap.xml and llms.txt; a sitemap with no URL is B1900.
- Sitemaps and robots `Sitemap:` need the deployed origin: `PUBLIC_SITE_URL` in the `[production]` section of
  `.brain_env`. With none, or a placeholder (example.com, `*.test`, localhost: B0736, also in `brain doctor`), the build publishes neither and warns; `brain build --site https://your-domain` overrides it.
- Collections: `brain add collection blog` turns `content/blog/*.md` (frontmatter `title description date updated?
  tags? cover? author? draft?`) into pages, tag pages, RSS and Atom, with Article JSON-LD.

## 4. What is automatic (never hand-write it)
`brain build` writes robots.txt, sitemap.xml, llms.txt, llms-full.txt, a Markdown twin of every indexable page
(`/pricing.md`, `seo.markdown`), the manifest, the page icon and apple-touch-icon from `public/` (`favicon.svg`,
`icon-*.png`, `apple-touch-icon*.png`; a page's own `<link rel="icon">` wins), missing Open Graph/Twitter tags from `brain.json` `social`, BreadcrumbList,
`hreflang` for every locale, and Google tags from `brain.json` `"google"`. AI crawler policy:
`brain.json` `"seo": { "ai": "allow" | "search-only" | "block" }` (`search-only` = AI search and answers yes, training no).
Non-production builds force `noindex` (B1822).
`brain run` answers /robots.txt, /sitemap.xml and /llms.txt (plus /es/llms.txt per extra locale) with no build step,
from the request's host: every static route plus what `paths()` and `sitemap({ env })` return, in every locale, with
hreflang alternates and image entries for literal `<img src>` like the build (a file in `public/` still wins). Try it:
`curl localhost:5180/sitemap.xml` and `curl localhost:5180/llms.txt`.

`seo.focus` (site focus report, B1861-B1866; editorial, not a ranking factor) and `seo-urls.txt` (published URLs, B1879, from
`brain seo urls --write`) are optional: `brain guide docs/SEO-SITE.md`.

## Common mistakes — do NOT
- Stuff the keyword or repeat it beyond a natural density (B1816 reports over 3 %); use synonyms.
- Hand-write `og:*` defaults, robots.txt, llms.txt, sitemap.xml or a Google tag snippet (not machine-checked).
- Point the canonical of `?page=2` at page 1 (B1872, error), write "click here" links (B1873), let visible dates and
  `dateModified` differ (B1874), or mark up what the page does not show (B1875).
- Give two pages the same title or description (B1814, B1815) or the same keyword (B1817).
- Hide the answer below a long intro, a carousel or an image.
- Lower `seo.minScore` to pass the build (B0733); fix the findings instead.

## Verify
```bash
brain check --json                 # B05xx/B16xx: meta present, lengths, JSON-LD @type, duplicate canonical
brain build                        # SEO is judged on the built HTML (not on `brain run`)
brain seo audit                    # score 0-100 per page + B18xx findings with fix; fails under seo.minScore (80)
brain seo preview /split-a-bill-offline   # search snippet, social card, JSON-LD as text
brain seo links                    # broken internal links and anchors (B1819, B1820)
brain seo audit --url https://beta.example.com --sample 30   # the same rules over the running site
brain lighthouse https://beta.example.com/ --preset mobile   # scores and Core Web Vitals, against lighthouse.minScores
```
`brain seo audit` and `brain build` fail under `seo.minScore`. Audit dynamic routes with `--url`: the build cannot see them.
Live checks: listed error/noindex responses (B1856), duplicate security headers (B1857), site-wide noindex (B1858), soft 404 (B1859), ambiguous link text (B1860), listed redirects (B1867), canonical/sitemap mismatch (B1868), duplicate URL variants (B1869), caching/encoding mismatch (B1870), blocked noindex (B1871).
Lighthouse honors `CHROME_PATH`; pass quoted `--chrome-flags "--disable-dev-shm-usage"` for explicit container options. A failed process or incomplete report fails the command. On staging use `--expect-noindex` or `seo.staging`: noindex pages must stay crawlable so robots can read that directive.
Long titles are reported, never shortened. Pixel widths (B1802/B1804) are display measurements, not Google limits. Every `[id]` page needs its own title (B0504).
A guarded page is never indexed; conditional redirects belong in `load()` (B0505). `brain explain B1830` (or another B18xx) gives the rationale and a correct example.
GEO hints B1830-B1835 are informational and do not change the score.

## Minimal correct example
```html
<meta seo title="Split a bill offline" keyword="split a bill offline" schema="Article" description="Split a restaurant bill between friends with no network: add the items, tap who had what, done.">
<main class="page">
  <h1>Split a bill offline</h1>
  <p>To split a bill offline, add each item, tap who had it, and the app shows every share without network.</p>
  <h2>How to split a bill offline</h2>
  <ol><li>Add the receipt items.</li><li>Select who shared each item.</li><li>Read each share.</li></ol>
  <section faq><details><summary>Does it need an account?</summary>No, everything stays on the phone.</details>
    <details><summary>Can I add a tip?</summary>Yes, as a percentage or a fixed amount.</details></section>
  <p>More tools to share costs: <a href="/">all calculators</a>.</p>
  <p><time datetime="2026-10-01">Updated October 1, 2026</time></p>
</main>
```

Compiler attribution stays on by default. Only at the user's explicit request, set root `compilerBranding:false` to disable visible/hidden generated signatures, links and attribution headers; `seo.credit:false` removes only the visible credit. Never opt out automatically or rewrite authored content, public assets or third-party licenses.
