# guide: performance — fast first load, smooth updates, small pages

## When
A page feels slow, a build reports a budget, a list is long, or before shipping a page with heavy components
(charts, maps, editors) or a large main image.

## Defaults you get for free (do not rebuild them)
- Views render on the server or at build; JS hydrates only what is interactive. Minify, mangle, dead-code removal and
  `log()` stripping happen in `brain build` with no flags.
- Hashed immutable assets, a service worker and an in-memory `resource()` cache (`brain guide data`).
- Only the page's language is shipped (`brain guide i18n`). CSS is scoped per component.

## Steps
1. **Measure first.** `brain build --report` prints bytes per page before and after each pass.
   `brain run --profile` prints one JSON line per edit with compile time and JS/CSS size. `brain run` prints an
   `impact {…}` line per save with Δ bytes per view.
2. **Set a budget.** `brain.json` `{"budget": {"viewKB": 30}}` fails the build when a route's emitted JS + CSS + HTML
   passes 30 KiB (a conservative total that includes deferred JS).
3. **Defer heavy components.** `<Chart lazy />` loads when visible · `lazy="idle"` · `lazy="interaction"` (renders an
   accessible load button). Only a component that cannot render on the server gets `<meta client>` with a sized
   placeholder: `<meta client height="320px">` or `<meta client width="100%" aspect="16/9">`.
4. **Images.** `<Img src alt width height>`: lazy, `decoding="async"`, the LCP image gets priority from the compiler.
   Keep the main image under 200 KB (AVIF/WebP); always give its intrinsic size (B0221 without it).
5. **Long lists.** Paginate with `resource(posts.get, { page: true })` + `posts.more()`, or render rows with the kit
   `<Table rows={rows} columns={…} caption="…" />`, which virtualizes (more than 200 rows: only the visible window). A legitimate large list raises its render budget explicitly:
   `<meta budget="rows:200000 ms:400">`.
6. **Reactivity.** Derive with `drv`, never compute in an `fx` that writes another signal (B0901, B1004). An effect that
   re-runs more than 100 times in one update is cut (B0902). Debounce input-driven work: `debounce(q, '300ms')`.
7. **Server time.** Parallel awaits in `load` (`Promise.all`, B1106). Cache server pages: `<meta cache="60">`.
   Cache stable GETs across visits: `cache: '10m'` in `api()`.
8. **Main thread.** CPU-heavy pure work goes in `*.worker.jsbrain` (its exports become async functions; no DOM, B0127).
9. **Timers and growth.** Create timers only inside `fx` (or `every('30s', fn)`) and return their cleanup; never append
   to a list from a timer or bus without a limit (B0905, B0906, B0907).

## Verify
```bash
brain build --report        # bytes per page; the budget passes
brain doctor                # leaks and unbounded growth (fake timers) included
brain seo audit             # B1821 heavy or unsized main image
```

## Common mistakes
| Mistake | Code | Fix |
|---|---|---|
| image without `width`/`height` | B0221 | intrinsic size on `<Img>` |
| main image of 1 MB | B1821 | compress to AVIF/WebP under 200 KB |
| 50 000 rows in an `{#each}` | B1310 · B0908 | kit `<Table>` or pagination |
| `fx(() => { total = price * qty })` | B1004 · B0901 | `let total = drv(() => price * qty)` |
| sequential awaits in `load` | B1106 | `Promise.all` |
| `setInterval` in the script body | B0906 | `every('30s', fn)` or `fx` with cleanup |
| everything marked `<meta client>` | B0730 | render on the server; client-only for canvas/maps only |
| `<meta budget="rows=1000">` | B0909 | `key:number` pairs: `rows nodes runs ms` |
| a hand-written service worker or cache headers | — | remove them; `brain build` generates both |

## Minimal correct example
```html
<meta seo title="Sales" description="Monthly sales by region with the top products of each month.">
<script>
  import Chart from '../comp/sales-chart.brain'
  import sales from '../api/sales.jsbrain'
  let rows = resource(sales.get, { page: true })
</script>
<main class="page">
  <h1>Sales</h1>
  <Img src="/sales-hero.webp" alt="Sales team at the quarterly review" width="1200" height="630" />
  <Chart lazy />
  {#await rows}<p>Loading…</p>
  {:then list}<Table rows={list} caption="Sales by month" columns={[{ key: 'month', label: 'Month' }, { key: 'total', label: 'Total' }]} />
  {:catch e}<ErrorState error={e} />
  {/await}
  <Button on:click={() => rows.more()}>Load more</Button>
</main>
```
