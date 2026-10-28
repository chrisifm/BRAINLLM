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
