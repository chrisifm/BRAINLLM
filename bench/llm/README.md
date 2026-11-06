# bench/llm — LLM test (#11)

20 views (`briefs.json`) generated with only `brain spec` as context, validated with `brain check --json` inside a
freshly created app (`brain new`); up to 3 fix rounds applying each `fix`.

```bash
node bench/llm/run.mjs [--model claude-haiku-5-5|claude-sonnet-5-5|qwen] [--mode spec|mcp] [--stage <label>] [--fix 3]
node bench/llm/analyze.mjs bench/llm/results/<tag>.json
```

`--mode mcp` adds the app map (`brain map --json`; while it does not exist, a QA substitute marked as such).
Each run stores the sha of the spec and of the scaffold's `tokens.cssbrain` at start and at end: if they change, the
run is marked `stable:false` and is not compared. `analyze.mjs` classifies each diagnostic as `lang` (LLM error),
`a11y`, `token` (token missing from the scaffold) and `ahead` (SPEC documents it and the checker does not yet).
Scratch apps go to `$QA_LLM_WORK` (default: `<os tmpdir>/brainllm-llm`); results go to `bench/llm/results/`.

## Official Codex CLI provider

`--provider codex` requires both `--model MODEL` and `--effort EFFORT`; neither is inferred from an installed
profile, a repository setting or a framework default. Select an approved model and its supported effort before
starting a paid run. `BRAIN_CODEX_BIN` selects one executable path (default `codex`), without a shell command or
embedded arguments. A local wrapper executable may invoke an operator-selected official npm CLI installation;
the framework does not install packages or copy authentication/configuration. Authentication stays in the CLI's
existing private store. Codex defaults to one concurrent generation; explicit `--conc` is bounded to 1..8.

```bash
node --test bench/llm/codex.test.mjs
node bench/llm/run.mjs --provider codex --model "$APPROVED_MODEL" --effort "$APPROVED_EFFORT" --conc 1 --mode spec --stage fresh
node bench/llm/gate.mjs bench/llm/results/codex-<model>-<effort>-fresh.json
```

The adapter invokes official `codex exec` with explicit model/reasoning effort, `--ignore-user-config`,
`--ignore-rules`, `--sandbox read-only`, `--skip-git-repo-check`, `--ephemeral`, `--json`, and an owned
`--output-last-message` file. Prompt input travels over stdin in an isolated empty temporary working directory.
`project_doc_max_bytes=0` omits project instruction documents, `features.shell_tool=false` disables shell tools
and `web_search="disabled"` disables search. These keys are described in the official
[Codex configuration schema](https://developers.openai.com/codex/config-schema.json); exec flags were checked
against official CLI 0.166.0 help. The prompt forbids tools/delegation; any observed tool item fails the generation.
This is a CLI-backed generation lane: built-in Codex instructions still apply, so it is not a raw API system-prompt
equivalence claim with the Claude or Ollama adapters. Read-only sandboxing alone is not a universal tool prohibition.

The CLI may emit a non-terminal `error` diagnostic item before a successful turn. Only the message-only item shape
(`id`, `type`, `message`) is discarded; its private text is never logged or retained. Actual tool items, terminal
errors, missing completion/usage and unsafe output still fail. On an infrastructure failure, queued Codex briefs
are marked unattempted and no new generation starts; already-running generations drain within their deadlines.
The run writes its incomplete evidence and exits 1, so it cannot be mistaken for a valid release-gate run.

Each generation has an absolute 180-second deadline, 2 MiB stdin/stdout/final-message limits and a 64 KiB stderr
limit. Deadline, interruption and limit failures terminate only the owned process group, escalate after 150 ms,
reap the direct child and remove owned scratch. SIGINT/SIGTERM also prevent later Codex generations in that run.
As with other process-group owners, deliberately detached descendants and an uncatchable parent SIGKILL cannot
be guaranteed cleanup by JavaScript. No raw JSONL transcript, reasoning, thread ID or stderr is persisted in results.
Only the bounded regular last-message file becomes generated source; symlinks are rejected. Completed-turn token
usage is validated; cached input is recorded separately and is already included in total input. CLI usage does
not provide a verified USD charge, so each Codex cost and the aggregate `costUSD` are `null`, never a free-run claim.

Codex artifacts include provider and effort in their filename and summary. Spec, binary and scaffold token hashes
retain the existing frozen-run checks. The release gate and matched comparison share validation: at least twenty
complete, uniquely identified generations, matching measured first-pass counts and valid frozen binary/spec/token
hashes are required. A single `--only` smoke is not release evidence. Codex baseline keys include provider, model,
explicit effort and mode; historical non-Codex model/mode keys remain unchanged. Different efforts never share an
accepted Codex baseline.
A cohort with no accepted reference reports `baseline none` and fails; review a complete reference run before
initializing it explicitly with `--accept`. Do not accept an unreviewed candidate as its own reference. The fake-CLI tests exercise transport,
cleanup and actual run/gate composition without contacting a model or changing a baseline.
Transport tests launch their synthetic CLI through the existing Node executable and a bounded structured argument
prefix, rather than asking the OS to execute newly created temporary shebang files. The run/gate integration test
uses a local preload that transports only its synthetic CLI/checker scripts through Node; the scripts' real bytes
are still copied and hashed by the unchanged frozen-run path. This avoids interpreter startup variance without
extending deadlines or accepting a timeout as the expected malformed-output failure.

| Stage | Spec | Toolchain state | Haiku 5.5 1st try | Final | Iterations |
|---|---|---|---:|---:|---:|
| base | `15b1bd15182a` | scaffold `--sp-N`, SPEC without token list | 7/20 | 19/20 | 2.00 |
| a | `3fca38d5a226` | #52 closed (tokens `--space-N`, app.brain), #53 in progress (primitives without import, bind to field) | **17/20** | 20/20 | 1.15 |
| a · mcp | `3fca38d5a226` | same + app map | 16/20 | 20/20 | 1.30 |
| a · Sonnet 5.5 | `80632fb71c50` | full SPEC with visible `planned` blocks | **19/20** (Sonnet) | 20/20 | 1.05 |

| served · Haiku 5.5 | `f60e94771d1d` (6,086 B) | served spec without `planned` blocks, frozen binary | **19/20** | 20/20 | 1.05 |
| served · Sonnet 5.5 | `b351664c21d8` (6,329 B) | same (TOOLCHAIN rebuilt between runs: different sha) | **20/20** | 20/20 | 1.00 |

Haiku's only failure on the served spec: a view with two `<h1>` (B0405), class "h1/main hierarchy" → §21 fix
level 3 (the scaffold ships `<main><h1>` and the model fills it in).

`results/void-*`: a Sonnet run with the spec changed mid-run (no guard yet); discarded.

## Release gate

`node bench/llm/gate.mjs results/<run>.json [--accept]` exits 1 if the run is not stable or if the first-try rate
drops by more than 1 view (noise with n=20) against `baseline.json` (per model and mode). `--accept` raises the
baseline only if the run passes and does not get worse. Current baseline (`baseline.json`): Haiku spec 19/20 (spec `f60e94771d1d`), Haiku mcp 16/20 (spec `3fca38d5a226`), Sonnet spec 20/20 (spec `b351664c21d8`).

## 2026-10-28 toolchain completion check

A deterministic replay of the 20 first-attempt sources in `claude-haiku-5-5-gate277a` against the updated compiler
returned **19/20**, matching that historical run, with zero infrastructure failures. The remaining users-CRUD source
references a missing API module and omits the empty state (B0119/B1101). The historical gate also passes its accepted
one-case tolerance. This reuses recorded source; it is **not a fresh model evaluation of the expanded specification**.
No model was called, no baseline was changed and no new first-pass-generation claim follows from this replay.
A fresh frozen model/spec/compiler run remains required before treating the newly documented features as LLM-benchmarked.

## 2026-10-29 gate for 0.3.0 (fresh model runs, `--mode spec`, n=20 each)

| Run | Served spec | Haiku 5.5 1st try | Sonnet 5.5 1st try | First-try failures |
|---|---|---:|---:|---|
| om020 (0.2.0 binary, `origin/main`) | `88d92fac2659` (22,155 B) | 17/20 | — | 3× B0119 (missing `server/`, `channels/`, `lib/` module) |
| gate030 a · b (0.3.0 before the fix) | `a402f4d9d1f3` (22,763 B) | 17/20 · 17/20 | 20/20 | B0119 missing module, B0111 undeclared helper |
| gate030 c · d (0.3.0, fixed spec) | `68ff3d58aea9` (21,765 B) | 18/20 · 19/20 | — | B0119 `../channels/cart.jsbrain` (both), B0111 `fmtLap` (c) |
| gate030e a · b (feeds, prefetch, `ai`) | `96a9d18ffbd2` (22,249 B) | 20/20 · 20/20 | 20/20 · 20/20 | none |

The drop came with 0.2.0, which grew the served spec by about 4 KB (sessions, form messages, live sitemaps) without a
fresh run; it was not caused by the 0.3.0 rules (B0411–B0415, B0502 never fired on a first try). Fix, in §21 order:
those passages moved to `brain guide forms|server|seo` behind a one-line pointer, and the existing import rule now says
that a store, helper or server module the app does not have yet is declared in the file or created with `brain add`.
Result: one of two Haiku runs passes the gate, the same profile as the accepted gate234 and gate274 pairs.
Open: the cart brief still imports `../channels/cart.jsbrain` in both runs (once in six runs before 0.2.0); candidate
fix is a B0119 edit that scaffolds the missing channel (`brain add channel`), which would make the fix mechanical.

After the review fixes (site feed always written, prefetch forgets on writes, Speculation Rules removed, the `ai`
primitive and its one-line spec pointer), all four runs pass on the first try and the gate passes for both models.

## 2026-10-29 experimental backend/agent slice — generation gate pending

The served specification is 23,884 bytes versus 22,249 bytes at the base revision (+1,635). Backend details
stay in live contracts, cards and the server guide rather than expanding the main spec with endpoint boilerplate.
This is a byte-size measurement, not a token count or a model evaluation. No model was selected/called and no baseline
was changed. `gate.mjs` without a candidate run exits 2 with usage; a fresh frozen spec/compiler/model run is still
required before claiming first-pass compatibility for this slice. Historical accepted rates above do not cover it.

### Frozen native-platform comparison (2026-10-30)

The before run uses the official integrity-verified 0.3.1 npm native binary and its accepted source specification,
with an explicitly selected Codex model/effort and tools disabled. The completed run produced **20/20 first-pass**
views, zero infrastructure failures and matching frozen spec/token hashes. Binary SHA prefix: `581fb5a79b05`;
spec SHA prefix: `0f209d72cbed`. This is a separate provider/model cohort from the historical Claude results.
The frozen 0.4.0 release binary also produced **20/20 first-pass** views, zero infrastructure failures and stable spec/token hashes.
The release gate and the matched-cohort comparison both passed without changing historical baselines.
After binary SHA prefix: `c5a264db5930`; spec SHA prefix: `d1c310181dcb`; served specification: 31939 bytes.
Both runs used `gpt-6.1-sol`, `medium`, specification-only context, concurrency 1 and zero fix iterations.
Artifacts: `results/codex-gpt-6.1-sol-medium-before-031-native.json` and
`results/codex-gpt-6.1-sol-medium-after-040-url-fix.json` (local benchmark outputs, not packaged).
The final run includes the Windows file-URL correction; the earlier 0.4.0 snapshot also passed 20/20.

### Backlog syntax checkpoint (2026-11-04)

Two actual frozen specification-only runs used `gpt-6.1-sol`, explicit `medium` effort, concurrency one and zero
fix iterations. Both generated the same twenty brief IDs without infrastructure failures:

| Compiler snapshot | Binary SHA prefix | Served spec SHA prefix | First pass |
|---|---|---|---|
| Published npm 0.8.12 | `92b1599bc6de` | `4df8ceaca9cc` | 20/20 |
| Unreleased local syntax snapshot (debug binary) | `cf70ec03813d` | `fd1dc76b1663` | 20/20 |
| Unreleased static-export contract snapshot (debug binary) | `d26e427a5e79` | `06b5e2a8d839` | 20/20 |

After cold review, the published run initialized only `codex|gpt-6.1-sol|medium|spec`; historical baselines were
preserved. The hardened release gate and matched-cohort comparison passed. Artifacts are the local
`results/codex-gpt-6.1-sol-medium-audit-published-retry-20261104.json` and
`results/codex-gpt-6.1-sol-medium-audit-candidate-20261104.json` outputs. The initial transport-failed attempt is not
acceptance evidence. These results measure source generation; they do not establish backend/browser workflow
coverage, production performance or correctness of later health-routing changes. The backlog remains in progress.
`compare.mjs before.json after.json` checks both complete runs, matching cohort settings, consistent measured
counts and the existing one-view tolerance without replacing accepted historical baseline entries.
Compiler crashes, empty/malformed diagnostics and failures in unrelated scaffold files are infrastructure failures;
they cannot count as passing generations. App directory names are safe deterministic hashes, independently from
model identifiers containing dots or slashes. Monetary cost remains unknown when the provider supplies no cost.

The static-export contract snapshot used the same twenty IDs, provider/model/effort/mode, concurrency one and
zero fixes: all twenty passed first try with stable spec/token hashes and zero infrastructure failures. Its gate
and matched comparison against the published reference passed without changing baseline.json. Local artifact:
`results/codex-gpt-6.1-sol-medium-static-export-candidate-20261104.json`. This snapshot predates the later internal
impact-measurement filename correction; it verifies source authoring, not that correction or release readiness.
Generic static HTTP/Chrome and stdio MCP acceptance are separate tests in scripts/test/static-export.test.mjs.

## 0.9.1 field-fix candidate (2026-11-05)

The fresh `codex/gpt-6.1-sol/medium/spec` cohort generated the same twenty briefs with concurrency one and zero
repair rounds: **20/20 first pass**, zero infrastructure failures and stable SPEC `3e0c0b7e46b2`, scaffold tokens
`97cdbb891450` and binary `95c114bac63a`. The release gate passed against the accepted 20/20 reference without
changing `baseline.json`. Local artifact: `results/codex-gpt-6.1-sol-medium-typed-input-adoption-final-20261105.json`.
The earlier owned-canceled partial candidate is not acceptance evidence. This cohort measures source authoring;
handler execution, shutdown, migration integrity, browser route reachability and public npm consumption require
their separate regression and release gates. The CLI provider does not supply a verified monetary charge.


## 0.10.0 frozen field-completion candidate (2026-11-06)

A fresh `codex/gpt-6.1-sol/medium/spec` run generated the same twenty briefs as the accepted published 0.8.12
reference. Both scored **20/20 first pass** with zero infrastructure failures. The candidate used concurrency one
and allowed at most three repair rounds; every brief passed its initial generation, so no repair was used.
The explicit release gate passed against the accepted reference without changing `baseline.json`.

| Snapshot | Binary SHA prefix | Served spec SHA prefix | First pass |
|---|---|---|---|
| Published npm 0.8.12 reference | `92b1599bc6de` | `4df8ceaca9cc` | 20/20 |
| Frozen 0.10.0 debug candidate | `69fb688f187d` | `f9b9717ceed3` | 20/20 |

The candidate served 78,089 UTF-8 specification bytes with stable spec and scaffold token hashes (`f03e6c811f69`).
Local artifact: `results/codex-gpt-6.1-sol-medium-backlog-010-final-20261106.json`. The provider supplied no verified
monetary charge. These results measure specification-only source generation; optional integrations, real HTTP,
browser behavior, database concurrency, bounded shutdown and anonymous package installation have separate gates.
The following test-only fixture and gate-registration corrections did not change the frozen compiler or specification.
