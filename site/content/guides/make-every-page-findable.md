---
title: Make every page findable
description: How BRAINLLM turns two attributes per view into sitemaps, social cards, structured data and the files language models read.
date: 2026-10-13
tags: [seo, ai visibility]
author: BRAINLLM team
---
In BRAINLLM a page needs two things from you to be found: a title and a description. The build writes everything else and scores the result.

## Write the metadata

```html
<meta seo title="Pricing" description="Plans for teams of every size, with a free tier for up to ten people.">
```

Keep the title short enough for a search result and the description between 70 and 155 characters. The audit measures titles by their width in pixels, the way search results cut them, and suggests a fix when they are too long or too short.

## Let the build do the rest

`brain build` writes robots.txt, a sitemap that follows the [sitemaps protocol](https://www.sitemaps.org/protocol.html), llms.txt and llms-full.txt from the [llms.txt proposal](https://llmstxt.org/), a Markdown copy of each page for language models, and the structured data search engines read: breadcrumbs, the site and the organization, questions grouped as an FAQ, articles and videos.

Add your social profiles to `brain.json` and every page gets complete [Open Graph](https://ogp.me/) and X tags, so a link shared on Facebook, LinkedIn, WhatsApp or Slack shows a proper card.

## Read the audit

```sh
brain seo audit
brain seo preview /pricing
```

Every page gets a score from 0 to 100 with a fix for each finding. Broken internal links fail the build, and pages under the minimum score fail `brain doctor`. The preview shows how a page looks in a search result and as a social card before you publish it.

## Decide who may read it

One setting in `brain.json` chooses the policy for AI crawlers. `"seo": { "ai": "allow" }` lets every crawler in. `"search-only"` asks the crawlers that collect training data to stay out, while search and answer engines can still read and cite your pages. `"block"` also asks AI search engines and the fetchers that act for a user in a chat to stay away. The build writes the matching robots.txt groups and a `Content-Signal` line, so you never edit robots.txt by hand.

## Be quotable

Answer engines quote pages that answer early, show a date, name an author and cite sources. The audit gives hints for each of those without lowering the score: start with a short answer, group question headings as an FAQ, show the date, and link the sources of your numbers.
