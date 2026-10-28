# bench/a11y-seo — accessibility and SEO metric

Two layers, both reproducible:

| Layer | Tool | What it measures | When |
|---|---|---|---|
| Static (internal) | `brain check --json` → count of B04xx/B0501 per file. Until it exists: `node a11y.mjs <files>` (provisional lint, markup rules B0401–B0407 and B0501) | violations per component/view; in the LLM test, % of generated views without violations | now (provisional) |
| External | Lighthouse 13.5.0 via `npx` (`lighthouse.sh`), measurement only, no project dependency | accessibility and SEO score (0–100) and failing audits, per URL | once `brain dev`/prerender serves `examples/` |

Proposed target: 0 B04xx/B0501 violations in `examples/` and Lighthouse a11y = 100, SEO ≥ 95 on the views.

Validation of the provisional lint: it reproduces exactly the `code/line/col` of the 9 markup fixtures in
`examples/invalid/` and gives 0 diagnostics on the 5 valid examples (2026-10-13). B0201, B0301, B0408 and B0409
need the BrainScript/CSS parser and are left to `brain check`.
