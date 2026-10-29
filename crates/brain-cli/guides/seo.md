# guide: seo — classic search and AI answers (GEO)

## When
Every view that should be found: landing pages, articles, docs, product and profile pages. Do this before writing the
page text, not after.

## 1. Pick the target query FIRST (one primary query per page)
1. Write the user's intent in one sentence: who arrives, what they want to know or do.
2. Write the words a person would actually type or ask an assistant ("how to split a bill offline", not "Bill
   Splitting Solutions"). Prefer the plain, specific phrase over the brand term.
3. Choose ONE primary query per page; two pages on the same query compete (B1817 cannibalization).
4. Declare it: `<meta seo keyword="split a bill offline">` on every indexable view (missing = B0502). The audit
   checks it in the title, the description, the H1, the first paragraph and the URL, at a natural density (B1816).
   Without a keyword, a title or description off the H1's topic is B1836.
5. Put it in: `title` (≤ 60 chars), `<h1>`, the first sentence, `description` (≤ 160 chars), and the route name
   (`brain add view split-a-bill-offline`). Lengths over the limits are B1602.

## 2. Write for answer engines
- **Answer first:** the first paragraph answers the query directly in ≤ 300 characters, then expands (B1830).
- **Questions as FAQ:** two or more question headings → group them; FAQPage JSON-LD is generated:
  `<section faq><details><summary>Is it free?</summary>Yes, for up to ten people.</details></section>` (B1831).
- **How-to:** numbered steps under question or "How to …" headings; one action per step.
- **Entities:** name things exactly (product, place, standard) and link 1-3 authoritative sources where you state
  facts or numbers (B1834).
- **Author and trust:** articles name an `author:` in the frontmatter and its profile in `brain.json`
  `"authors": { "Ada Lovelace": { "url": "https://…", "sameAs": ["https://www.linkedin.com/in/…"] } }` (B1833).
  The site's own profiles: `"organization": { "name", "logo", "sameAs": [...] }` and `"social": { "x": "@acme", … }`.
- **Freshness:** show the date in the text `<time datetime="2026-10-01">October 1, 2026</time>`; collections set
  `date:` and `updated:` in the frontmatter (→ `dateModified`, B1832); views set `<meta seo lastmod="2026-10-13">`.
- **Depth:** thin pages score lower (B1808); internal links in and out (B1812 no links out, B1813 orphan page).
  A link to a page the build does not produce fails `brain build` (B1819).

## 3. Declare the page
```html
<meta seo title="Split a bill offline" keyword="split a bill offline" canonical="/split-a-bill-offline" schema="HowTo"
  description="Split a restaurant bill between friends without network: …">
```
- `schema="Type"` writes JSON-LD with that `@type` plus name, description and url. A full object:
  `jsonld={{ '@type': 'Article', headline: props.post.title, datePublished: props.post.date }}` (B1603/B1818 without `@type`).
- Keep each attribute value on one line (newlines are kept in the output). Values may be expressions from `load` props: `title={props.user.name}`.
- Site defaults once in `app.brain`: `<meta seo titleTemplate="%s · Acme" og:image="/og.png" twitter="@acme">`.
- `robots="noindex"` for pages that must not be indexed (403, thank-you pages). Other head tags: a `<head>` block in the view.
- `[id]` views: `export async function paths()` (prerendered, in the sitemap) or, for live pages,
  `export async function sitemap({ env })` returning `[{ id, lastmod }]` (needs `brain.json` `site`). Never both (B0117).
- Collections: `brain add collection blog` turns `content/blog/*.md` (frontmatter `title description date updated?
  tags? cover? author? draft?`) into pages, tag pages, RSS and Atom, with Article JSON-LD.

## 4. What is automatic (never hand-write it)
`brain build` writes robots.txt, sitemap.xml, llms.txt, llms-full.txt, a Markdown twin of every indexable page
(`/pricing.md`, `seo.markdown`), the manifest, missing Open Graph/Twitter tags from `brain.json` `social`, BreadcrumbList,
`hreflang` for every locale, and Google tags from `brain.json` `"google"`. AI crawler policy:
`brain.json` `"seo": { "ai": "allow" | "search-only" | "block" }` (`search-only` = AI search and answers yes, training no).
Non-production builds force `noindex` (B1822).

## Common mistakes — do NOT
- Stuff the keyword or repeat it beyond a natural density (B1816 reports over 3 %); use synonyms.
- Hand-write `og:*` defaults, robots.txt, llms.txt, sitemap.xml or a Google tag snippet.
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
```
`brain explain B1830` (or any B18xx) prints why and a correct example. GEO hints (B1830-B1835) are info: they never
change the score but they decide whether an AI answer quotes the page.

## Minimal correct example
```html
<meta seo title="Split a bill offline" keyword="split a bill offline" schema="HowTo"
  description="Split a restaurant bill between friends with no network: add the items, tap who had what, done.">
<main class="page">
  <h1>Split a bill offline</h1>
  <p>To split a bill offline, add each item, tap who had it, and the app shows every share without network.</p>
  <h2>How to split a bill offline</h2>
  <ol><li>Add the items from the receipt.</li><li>Tap the people who shared each item.</li><li>Read each share.</li></ol>
  <section faq>
    <details><summary>Does it need an account?</summary>No, everything stays on the phone.</details>
    <details><summary>Can I add a tip?</summary>Yes, as a percentage or a fixed amount.</details>
  </section>
  <p>More tools to share costs: <a href="/">all calculators</a>.</p>
  <p><time datetime="2026-10-01">Updated October 1, 2026</time></p>
</main>
```
