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

Each generation has an absolute 180-second deadline, 2 MiB stdin/stdout/final-message limits and a 64 KiB stderr
limit. Deadline, interruption and limit failures terminate only the owned process group, escalate after 150 ms,
reap the direct child and remove owned scratch. SIGINT/SIGTERM also prevent later Codex generations in that run.
As with other process-group owners, deliberately detached descendants and an uncatchable parent SIGKILL cannot
be guaranteed cleanup by JavaScript. No raw JSONL transcript, reasoning, thread ID or stderr is persisted in results.
Only the bounded regular last-message file becomes generated source; symlinks are rejected. Completed-turn token
usage is validated; cached input is recorded separately and is already included in total input. CLI usage does
not provide a verified USD charge, so each Codex cost and the aggregate `costUSD` are `null`, never a free-run claim.

Codex artifacts include provider and effort in their filename and summary. Spec, binary and scaffold token hashes
retain the existing frozen-run checks. `gate.mjs` remains unchanged and keys baselines by model plus mode; review
provider/effort/context provenance before comparing runs, and do not mix different efforts into an accepted baseline.
A new model with no accepted baseline reports `baseline none`; passing that structural gate establishes no historical
regression comparison. Do not use `--accept` without a reviewed fresh run. The fake-CLI tests exercise transport,
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
After binary SHA prefix: `f38f973b8e34`; spec SHA prefix: `d1c310181dcb`; served specification: 31939 bytes.
Both runs used `gpt-6.1-sol`, `medium`, specification-only context, concurrency 1 and zero fix iterations.
Artifacts: `results/codex-gpt-6.1-sol-medium-before-031-native.json` and
`results/codex-gpt-6.1-sol-medium-after-040-native.json` (local benchmark outputs, not packaged).
`compare.mjs before.json after.json` checks both complete runs, matching cohort settings, consistent measured
counts and the existing one-view tolerance without replacing accepted historical baseline entries.
Compiler crashes, empty/malformed diagnostics and failures in unrelated scaffold files are infrastructure failures;
they cannot count as passing generations. App directory names are safe deterministic hashes, independently from
model identifiers containing dots or slashes. Monetary cost remains unknown when the provider supplies no cost.
