# Recipes benchmark: with vs without recipes (task #83)

Harness: `bench/autonomy/run.mjs --mode mcp` (only the brain MCP server, no human turns). Six briefs that map to a recipe; "on" inlines the recipe's files in the brief (the harness cannot pre-seed files), "off" is the plain brief. Done = independent `brain doctor` exit 0 and the routes the brief names exist.

| model | recipes | done | first doctor clean | errors at first check | write/edit errors | doctor calls | tool calls | tokens in / out | cost | time |
|---|---|---:|---:|---:|---:|---:|---:|---|---:|---:|
| haiku | off | 6/6 | 3/6 | 0 | 22 | 8 | 266 | 1.73 M / 53 k | $0.07 | 3313 s |
| haiku | on | 6/6 | 6/6 | 0 | 3 | 8 | 81 | 759 k / 32 k | $0.04 | 209 s |
| sonnet | off | 6/6 | 4/6 | 2 | 5 | 8 | 96 | 1.02 M / 27 k | $0.81 | 253 s |
| sonnet | on | 6/6 | 6/6 | 0 | 3 | 6 | 59 | 418 k / 24 k | $0.58 | 201 s |

Hung on an MCP tool until the 1500 s harness timeout (no result event, so no tokens or cost) and re-run once: haiku/off/login-guard, haiku/off/crud. The table uses the re-run; the hang is in `summary.json` (`hungBefore`, `timedOut`).

## Per brief

| model | recipes | brief | done | first doctor clean | errors at first check | tool calls | tokens in | cost | time | doctor errors at the end |
|---|---|---|---|---|---:|---:|---:|---:|---:|---|
| haiku | off | login-guard | yes | never ran | 0 | 47 | 0 | $0.000 | 1500 s |  |
| haiku | off | crud | yes | never ran | 0 | 102 | 0 | $0.000 | 1500 s |  |
| haiku | off | dashboard | yes | no | 0 | 28 | 411 k | $0.018 | 85 s |  |
| haiku | off | pricing | yes | yes | 0 | 17 | 294 k | $0.011 | 56 s |  |
| haiku | off | settings | yes | yes | 0 | 40 | 592 k | $0.021 | 96 s |  |
| haiku | off | wizard | yes | yes | 0 | 32 | 429 k | $0.017 | 76 s |  |
| haiku | on | login-guard | yes | yes | 0 | 21 | 141 k | $0.010 | 52 s |  |
| haiku | on | crud | yes | yes | 0 | 15 | 193 k | $0.010 | 44 s |  |
| haiku | on | dashboard | yes | yes | 0 | 16 | 202 k | $0.009 | 39 s |  |
| haiku | on | pricing | yes | yes | 0 | 9 | 64 k | $0.005 | 23 s |  |
| haiku | on | settings | yes | yes | 0 | 12 | 96 k | $0.005 | 29 s |  |
| haiku | on | wizard | yes | yes | 0 | 8 | 64 k | $0.005 | 22 s |  |
| sonnet | off | login-guard | yes | yes | 0 | 21 | 157 k | $0.163 | 46 s |  |
| sonnet | off | crud | yes | no | 1 | 28 | 263 k | $0.170 | 57 s |  |
| sonnet | off | dashboard | yes | yes | 0 | 10 | 166 k | $0.144 | 45 s |  |
| sonnet | off | pricing | yes | no | 1 | 11 | 152 k | $0.112 | 38 s |  |
| sonnet | off | settings | yes | yes | 0 | 18 | 170 k | $0.118 | 37 s |  |
| sonnet | off | wizard | yes | yes | 0 | 8 | 107 k | $0.099 | 30 s |  |
| sonnet | on | login-guard | yes | yes | 0 | 11 | 64 k | $0.123 | 43 s |  |
| sonnet | on | crud | yes | yes | 0 | 15 | 91 k | $0.123 | 40 s |  |
| sonnet | on | dashboard | yes | yes | 0 | 11 | 68 k | $0.104 | 34 s |  |
| sonnet | on | pricing | yes | yes | 0 | 10 | 81 k | $0.097 | 32 s |  |
| sonnet | on | settings | yes | yes | 0 | 6 | 53 k | $0.056 | 23 s |  |
| sonnet | on | wizard | yes | yes | 0 | 6 | 62 k | $0.081 | 29 s |  |
