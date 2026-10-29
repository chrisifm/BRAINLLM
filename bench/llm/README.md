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

The drop came with 0.2.0, which grew the served spec by about 4 KB (sessions, form messages, live sitemaps) without a
fresh run; it was not caused by the 0.3.0 rules (B0411–B0415, B0502 never fired on a first try). Fix, in §21 order:
those passages moved to `brain guide forms|server|seo` behind a one-line pointer, and the existing import rule now says
that a store, helper or server module the app does not have yet is declared in the file or created with `brain add`.
Result: one of two Haiku runs passes the gate, the same profile as the accepted gate234 and gate274 pairs.
Open: the cart brief still imports `../channels/cart.jsbrain` in both runs (once in six runs before 0.2.0); candidate
fix is a B0119 edit that scaffolds the missing channel (`brain add channel`), which would make the fix mechanical.
