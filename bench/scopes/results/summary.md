# Scopes bench — results

Binary sha256 `a7a460dff990` · 2026-10-13T21:05:00.775Z · total 10.2 s (build 1.5 s) · dev 12/12 · prod 12/12 · verdict **PASS**

| # | Rule | dev | prod | Evidence |
|---|---|---|---|---|
| 1 | Local state per instance | pass | pass | One: 3 × 2 = 6 · Two: 0 × 2 = 0 · Three: 0 × 2 = 0 |
| 2 | Context per subtree | pass | pass | after shift: red→red+, inner blue-inner, green green, outside none |
| 3 | Global store shared, field-precise | pass | pass | A=101 B=101 other=1; other write touched 1 node(s) in #far-other |
| 4 | Precise updates | pass | pass | counter click: 2 node(s); 200-row bump: 1 node(s) in row-57 |
| 5 | CSS same-name isolation | pass | pass | 17 probes + 430 component spans as expected |
| 6 | IDs per instance | pass | pass | inputs email-1 / email-2; dup ids 0 |
| 7 | Events reach only the direct parent | pass | pass | leaf a→mid only (view 0); b relayed as "b-relayed"; DOM ping events 0 |
| 8 | Slots keep the author's scope | pass | pass | slotted content styled and bound by its author |
| 9 | Error isolation | pass | pass | handler throw: #boom1 kept, clicks 1; drv throw → "Boom failed Retry"; setup throw → "Setup failed Retry"; neighbours "After: 2 × 2 = 4", "Before: 1 × 2 = 2", "After: 1 × 2 = 2" |
| 10 | CSS cannot escape its component | pass | pass | body 16px, main 16px, sibling p 31px |
| 11 | JS same names stay module-local | pass | pass | counts 1/0/1010; leaked globals 0 |
| 12 | Custom property per instance | pass | pass | plain rgb(0, 98, 196), custom rgb(10, 20, 30), view rgb(0, 98, 196) |

Raw per-mode data (every computed style, mutation record and console line): `results/raw/<mode>.json` (not committed).
