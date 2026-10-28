# .brain_env gate (#140)

brain `/private/tmp/claude-501/-Users-chris-Projects-WEB-BRAINLLM/d83c019e-0f2f-49dc-b700-76a0cf0fa2b1/scratchpad/target/release/brain` sha256 `0b51a8261ff3` · 2026-10-13T21:07:30.493Z · **29 passed, 0 failed, 0 leaks**

| Check | Result | Evidence | Owner if failing |
|---|---|---|---|
| 1a brain new: the §29 tree (36 paths) | PASS | all 36 paths present (exit 0) |  |
| 1b .brain_env: shared block + [development] [staging] [production], example in sync, gitignored | PASS | sections: development, staging, production · shared keys: PUBLIC_SITE_NAME, PUBLIC_SITE_URL · example values: 0 · example keys match: true · .gitignore has .brain_env: true |  |
| 1c untouched app: brain doctor exits 0 | PASS | exit 0: errors  none captured while the tests ran \| build   ok · build j9jw6ox6 \| seo     6 page(s) scored 91–100 (minimum 80) \| leaks   none after the tests unmounted (GC forced) \| ✓ no errors |  |
| 1d untouched app: brain build (static) succeeds | PASS | exit 0: summary 10 pages · 164.3 KB gz total · SEO 91–95 · 0 written, 115 unchanged · 335 ms |  |
| 1e SEO audit: every indexable page ≥ 80 | PASS | /about 95, / 91, /posts/hello-world 95, /posts 95, /posts/second-post 95, /login 100 · noindex (not scored): /403, *, /_offline, /account |  |
| 2-staging build --env staging: its PUBLIC_API_URL in the client bundle, the other env's absent, no private value in dist | PASS | exit 0 · own value in 4 file(s) (_b/kod9sikt.js, _b/kod9sikt.js.gz, about/index.html) · other env value in 0 · private values in dist: none |  |
| 2-production build --env production: its PUBLIC_API_URL in the client bundle, the other env's absent, no private value in dist | PASS | exit 0 · own value in 8 file(s) (_b/fdr2vl3b.js, _b/fdr2vl3b.js.gz, about/index.html) · other env value in 0 · private values in dist: none |  |
| 2-diff staging and production bundles carry different PUBLIC_* values | PASS | staging → stg value only: true · production → prod value only: true |  |
| 2-site-url PUBLIC_SITE_URL follows --env (canonical, sitemap) | PASS | staging: stg url in 20 file(s), prod url in 0 · production: prod url in 38, stg url in 0 |  |
| 3 process env PUBLIC_API_URL overrides [production] | PASS | exit 0 · override in 8 client file(s) · file value in 0 |  |
| 4 empty [production] key: build --env production fails with B0720 (host env fills it) | PASS | exit 2 · B0720 in output: true · names DATABASE_URL: true · values leaked: 0 · same build with DATABASE_URL in the process env: exit 0 · "error: B0720 .brain_env:20:1 variable `DATABASE_URL` has no value in `production`; fix: set `DATABASE_URL=…` under `[production]` in .brain_env or in the host's" |  |
| 5-check brain check --json: no .brain_env value | PASS | exit 0 · 3 bytes captured |  |
| 5-doctor brain doctor --json: no .brain_env value | PASS | exit 0 · 2990 bytes captured |  |
| 5-map brain map --json: no .brain_env value | PASS | exit 0 · 3851 bytes captured · env keys listed: {"development":["API_TIMEOUT_MS","BRAIN_SECRET","DATABASE_URL","DEMO_PASSWORD","PUBLIC_API_URL","PUBLIC_SITE_NAME","PUBLIC_SITE_URL"],"staging":["API_TIMEOUT_MS","DATABASE_URL","PUBLIC_API_URL","PUBLIC_SITE_NAME","PUBLIC_SITE_URL"],"production":["API_TIMEOUT_MS","DATABASE_URL","INTERNAL_TOKEN","PUBLIC_API_URL","PUBLIC_SITE_NAME","PUBLIC_SITE_URL"]} |  |
| 5-mcp-tools_list MCP tools/list: no .brain_env value | PASS | answered · 24542 bytes |  |
| 5-mcp-brain_check MCP brain_check: no .brain_env value | PASS | answered · 160 bytes |  |
| 5-mcp-brain_doctor MCP brain_doctor: no .brain_env value | PASS | answered · 6146 bytes |  |
| 5-mcp-brain_map MCP brain_map: no .brain_env value | PASS | answered · 8456 bytes |  |
| 5-mcp-brain_info MCP brain_info: no .brain_env value | PASS | answered · 2550 bytes |  |
| 5-mcp-brain_read_brain_env MCP brain_read .brain_env: no .brain_env value | PASS | answered · 137 bytes · {"jsonrpc":"2.0","id":7,"result":{"content":[{"type":"text","text":"`.brain_env` is outside the app or a secrets file"}],"isError":true}} |  |
| 5-mcp-brain_read_brain_env_example MCP brain_read .brain_env.example: no .brain_env value | PASS | answered · 145 bytes · {"jsonrpc":"2.0","id":8,"result":{"content":[{"type":"text","text":"`.brain_env.example` is outside the app or a secrets file"}],"isError":true}} |  |
| 5-mcp-stderr MCP stderr: no .brain_env value | PASS | answered · 0 bytes |  |
| 5-run brain run log (startup + requests): no .brain_env value | PASS | port 51017 · /about 200, /views/about.brain 200, / 200, /login 200, /nope-404 404 · 517 bytes of log · pid 27121 stopped |  |
| 5-run-env brain run: [development] PUBLIC_* substituted in the dev module, no private value in responses | PASS | /views/about.brain has dev PUBLIC_API_URL: true · raw process.env.PUBLIC_ left: false · prod value: false · private values in responses: none |  |
| 5-selftest leak scanner finds every value in .brain_env itself | PASS | 14/14 |  |
| 5-total leaks found across every captured output | PASS | 0 leaks · 14 values searched literally |  |
| 6 fixtures-pending/env: B0720–B0724 + no-leak (5/5 OK) | PASS | exit 0 · OK B0724 via `brain check --json .` \| OK B0723 via `brain check --json .` \| OK B0720 via `brain build --json --env production` \| OK B0721 via `brain check --json .` \| OK B0722 via `brain doctor --json` |  |
| 7-env140b-haiku-5-5 LLM gate env140b-haiku-5-5: URL per section in .brain_env as PUBLIC_*, token server-only, no .env, no hardcoded URL, doctor 0 | PASS | {"brainEnv":true,"urlKey":"PUBLIC_API_URL","urlPerSection":true,"urlPublic":true,"tokenKeys":["STATUS_API_TOKEN"],"tokenServerOnly":true,"tokenProdEmptyOrAbsent":true,"dotEnv":[],"hardcoded":[],"tokenInClient":[],"doctor":0,"harnessDone":true} |  |
| 7-env140b-sonnet-5-5 LLM gate env140b-sonnet-5-5: URL per section in .brain_env as PUBLIC_*, token server-only, no .env, no hardcoded URL, doctor 0 | PASS | {"brainEnv":true,"urlKey":"PUBLIC_API_URL","urlPerSection":true,"urlPublic":true,"tokenKeys":["STATUS_API_TOKEN"],"tokenServerOnly":true,"tokenProdEmptyOrAbsent":true,"dotEnv":[],"hardcoded":[],"tokenInClient":[],"doctor":0,"harnessDone":true} |  |

## LLM gate

| Model run | Done (harness) | Views | Tool calls | Cost | Gate |
|---|---|---|---|---|---|
| env140b-haiku-5-5 | true | 13/2 | 39 | $0.02 | PASS |
| env140b-sonnet-5-5 | true | 13/2 | 13 | $0.14 | PASS |
