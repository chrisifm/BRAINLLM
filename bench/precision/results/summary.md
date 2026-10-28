# Precision bench — results

Binary sha256 `a7a460dff990` · 2026-10-13T20:47:42.179Z · total 53.9 s · PASS 16 · FAIL 0 · BLOCKED 0 · verdict **PASS**

| # | Case | Expected | dev | prod | Owner | Evidence |
|---|---|---|---|---|---|---|
| P1 | local state: one counter among 1 000 components | 2 | pass (2) | pass (2) | RUNTIME runtime/brain.js (signals, keyed lists) | dev: 2 mutation(s) = expected 2 (118 ms) |
| P2 | global store: write one field (10 readers of it, 10 of another field) | 10 | pass (10) | pass (10) | RUNTIME runtime/brain.js (signals, keyed lists) · channels store | dev: 10 mutation(s) = expected 10 (115 ms) |
| P3 | global store: write an equal value (Object.is) → nothing | 0 | pass (0) | pass (0) | RUNTIME runtime/brain.js (signals, keyed lists) · channels store | dev: 0 mutation(s) = expected 0 (98 ms) |
| P4 | global store: write the other field | 10 | pass (10) | pass (10) | RUNTIME runtime/brain.js (signals, keyed lists) · channels store | dev: 10 mutation(s) = expected 10 (113 ms) |
| P5 | group state: provide(Cart, store()) per subtree; write x in subtree one | 5 | pass (5) | pass (5) | RUNTIME #169 (store in provide) · COMPILER #170 | dev: 5 mutation(s) = expected 5 (113 ms) |
| P6 | 1 000-row keyed list: update one row field | 1 | pass (1) | pass (1) | RUNTIME runtime/brain.js (signals, keyed lists) | dev: 1 mutation(s) = expected 1 (107 ms) |
| P7 | 1 000-row keyed list: swap rows 2 and 999 (moves only those two) | 4 | pass (4) | pass (4) | RUNTIME runtime/brain.js (signals, keyed lists) | dev: 4 mutation(s) = expected 4; order ok (111 ms) |
| P8 | 1 000-row keyed list: remove one row | 1 | pass (1) | pass (1) | RUNTIME runtime/brain.js (signals, keyed lists) | dev: 1 mutation(s) = expected 1 (114 ms) |

**First interaction (informational, not gated):** dev: P1 view 0 mutation(s) in 84 ms, P5 view 0 mutation(s) in 93 ms, P6 view 0 mutation(s) in 87 ms · prod: P1 view 0 mutation(s) in 93 ms, P5 view 0 mutation(s) in 86 ms, P6 view 4 mutation(s) in 99 ms (a click on the inert `<h1>` before the measured write; in production it renders the island again and swaps it in).

Numbers in parentheses: labelled mutations observed (expected count in the third column). Raw labels: `results/raw/<mode>.json` (not committed).
