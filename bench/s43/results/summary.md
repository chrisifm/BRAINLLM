# §43 bench: agent loop before vs after, and a large app by components (task #202)

Harness: `bench/autonomy/run.mjs --mode mcp` (only the brain MCP server, no human turns, release binary frozen per run). "before" = the same binary with `brain_card`, `brain_impact`, `brain_why` and `brain_split` denied (`--deny`, they are removed from the agent's tool list); "after" = all tools. Done = independent `brain doctor` exit 0 and `views/` holds at least the routes the brief asks for. A read is big when `brain_read` returned a file over 200 lines (or a paged one, over 60 KB).

## (a) Agent loop, before vs after (bench/autonomy/briefs.json, 5 briefs)

| model | verdict | median time before → after | same, timed-out runs left out | median tool calls before → after | done before / after |
|---|---|---:|---:|---:|---|
| haiku | **FAIL** | 111 s → 175 s | 111 s → 151.5 s (FAIL) | 57 → 84 (after > before) | 5/5 / 5/5 |
| sonnet | **PASS** | 56 s → 48 s | 56 s → 48 s (PASS) | 30 → 31 (after > before) | 5/5 / 5/5 |

Noise reference (A/A): a second "after" run of the same briefs (arm `aa`) took a median of 60 s (sonnet) against 48 s; a before/after gap smaller than the A/A gap is within run-to-run variance.

| model | arm | done | first doctor clean | doctor calls | tool calls | reads (big) | card / map / impact / why / split | tokens in / out | cost | total time |
|---|---|---:|---:|---:|---:|---:|---|---|---:|---:|
| haiku | before | 5/5 | 5/5 | 11 | 401 | 41 (0) | 0 / 4 / 0 / 0 / 0 | 7.52 M / 122 k | $0.18 | 752 s |
| haiku | after | 5/5 | 3/5 | 12 | 381 | 39 (0) | 15 / 4 / 0 / 2 / 0 | 5.55 M / 93 k | $0.14 | 2452 s |
| sonnet | before | 5/5 | 2/5 | 8 | 156 | 28 (0) | 0 / 0 / 0 / 0 / 0 | 1.13 M / 35 k | $0.94 | 277 s |
| sonnet | after | 5/5 | 4/5 | 6 | 151 | 27 (0) | 4 / 0 / 0 / 0 / 0 | 938 k / 34 k | $0.89 | 247 s |
| sonnet | aa | 5/5 | 3/5 | 8 | 184 | 29 (0) | 10 / 0 / 0 / 0 / 0 | 1.59 M / 36 k | $0.99 | 294 s |

### Per brief

| model | arm | brief | done | time | tool calls | reads (big) | doctor calls | first doctor clean | tokens in | cost | doctor errors at the end |
|---|---|---|---|---:|---:|---:|---:|---|---:|---:|---|
| haiku | before | habits | yes | 172 s | 97 | 6 (0) | 3 | yes | 1.66 M | $0.041 |  |
| haiku | before | inventory (re-run) | yes | 263 s | 154 | 9 (0) | 2 | yes | 3.57 M | $0.072 |  |
| haiku | before | events | yes | 111 s | 55 | 9 (0) | 2 | yes | 767 k | $0.022 |  |
| haiku | before | recipes | yes | 98 s | 57 | 10 (0) | 2 | yes | 852 k | $0.024 |  |
| haiku | before | support | yes | 108 s | 38 | 7 (0) | 2 | yes | 675 k | $0.021 |  |
| haiku | after | habits | yes | 175 s | 101 | 4 (0) | 2 | yes | 2.33 M | $0.048 |  |
| haiku | after | inventory (re-run) | yes | 1800 s (timeout) | 87 | 7 (0) | 2 | no | 0 | $0.000 |  |
| haiku | after | events | yes | 77 s | 46 | 7 (0) | 2 | yes | 520 k | $0.018 |  |
| haiku | after | recipes | yes | 128 s | 63 | 9 (0) | 3 | no | 596 k | $0.025 |  |
| haiku | after | support | yes | 272 s | 84 | 12 (0) | 3 | yes | 2.10 M | $0.048 |  |
| sonnet | before | habits | yes | 57 s | 35 | 4 (0) | 1 | yes | 282 k | $0.212 |  |
| sonnet | before | inventory | yes | 61 s | 30 | 5 (0) | 1 | yes | 174 k | $0.193 |  |
| sonnet | before | events | yes | 55 s | 27 | 7 (0) | 2 | no | 204 k | $0.163 |  |
| sonnet | before | recipes | yes | 56 s | 36 | 7 (0) | 2 | no | 245 k | $0.200 |  |
| sonnet | before | support | yes | 48 s | 28 | 5 (0) | 2 | no | 227 k | $0.168 |  |
| sonnet | after | habits | yes | 48 s | 31 | 4 (0) | 2 | no | 241 k | $0.205 |  |
| sonnet | after | inventory | yes | 60 s | 36 | 7 (0) | 1 | yes | 256 k | $0.210 |  |
| sonnet | after | events | yes | 43 s | 26 | 7 (0) | 1 | yes | 151 k | $0.143 |  |
| sonnet | after | recipes | yes | 54 s | 33 | 5 (0) | 1 | yes | 163 k | $0.185 |  |
| sonnet | after | support | yes | 42 s | 25 | 4 (0) | 1 | yes | 127 k | $0.147 |  |
| sonnet | aa | habits | yes | 51 s | 31 | 4 (0) | 1 | yes | 177 k | $0.187 |  |
| sonnet | aa | inventory | yes | 51 s | 36 | 4 (0) | 1 | yes | 172 k | $0.172 |  |
| sonnet | aa | events | yes | 67 s | 41 | 8 (0) | 2 | yes | 523 k | $0.216 |  |
| sonnet | aa | recipes | yes | 60 s | 35 | 7 (0) | 2 | no | 279 k | $0.196 |  |
| sonnet | aa | support | yes | 65 s | 41 | 6 (0) | 2 | no | 435 k | $0.220 |  |

Hung until the harness timeout and re-run once (the table uses the re-run; the hang is in `summary.json`, `hungBefore`): haiku/before/inventory (1800 s, 55 tool calls), haiku/after/inventory (1800 s, 146 tool calls).

## (b) Large app by components (bench/s43/briefs-202.json, 11 pages, 60+ components)

| model | verdict | doctor 0 | components (map total) | non-view components | routes | reads (big) | max lines read | card / map / split calls | tool calls | tokens in / out | cost | time | B1010 at the end | doctor again ×3 (exit 0) |
|---|---|---|---:|---:|---:|---:|---:|---|---:|---|---:|---:|---:|---:|
| sonnet | **PASS** | yes | 100 | 83 | 13 | 11 (0) | 59 | 8 / 1 / 0 | 194 | 2.86 M / 69 k | $1.41 | 412 s | 0 | 3/3 |
| haiku (re-run; first run: 3601 s, timeout) | **PASS** | yes | 103 | 86 | 13 | 13 (0) | 42 | 14 / 2 / 0 | 396 | 28.08 M / 205 k | $1.94 | 902 s | 0 | 3/3 |

The fresh `brain new` scaffold already holds 14 map components (2 outside views/) and 8 routes; the counts above include them.

## Notes

- §43 tools the agents actually called over all runs: brain_card 51, brain_map 11, brain_why 2, brain_impact 0, brain_split 0. SPEC.md does not mention them; agents learn of them only from the tool descriptions.
- Every timeout in this bench is a tool that never answered, not an agent that kept working: `brain_eval` with an awaited timer after clicking a submit whose handler calls `navigate()` (the CDP call has no deadline and blocks the MCP server), and once `brain_navigate` (aborted by Claude Code after 1800 s of silence). These tools exist in both arms; a hang inflates whichever arm it lands in.
- Big reads (over 200 lines) were 0 in every run, both arms: with the default scaffold and these briefs no agent ever opened a file that large, so "0 big reads" does not separate the arms.
