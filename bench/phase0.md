# Phase 0 — token cost and error rate by naming style

**Question.** Should `.brain` source be written with cryptic names (`t01`, `f02`) to save tokens, or with
semantic names (`items`, `toggleItem`)? (plan §1)

**Verdict.** **Semantic** names in the source; compaction is left to the minifier (`brain-min`) in the
output. Cryptic names cost **more** tokens, not fewer: +6.3 % for the component alone and +42.5 % with the dictionary
the LLM needs to use them. Abbreviated names do not save input tokens (−0.4 %). The error rate does not tell
the styles apart in either task (see §3), so nothing offsets the extra cost of cryptic names.

## 1. Method (reproducible)

| Aspect | Value |
|---|---|
| Model | `claude-haiku-5-5` (the LEAD's decision: short prompts, bounded cost) |
| Measurement | `claude -p --output-format json` (Claude Code 2.1.296), no API key: this is **not** `count_tokens` |
| Input tokens | `usage.input_tokens + cache_creation_input_tokens + cache_read_input_tokens` |
| Isolation | empty cwd (`~/tmp/brainllm-phase0` at measurement time; now `PHASE0_CWD` or `$TMPDIR/brainllm-phase0`), fixed `--system-prompt`, `--tools ""`, `--setting-sources ""`, `--strict-mcp-config`, `--no-session-persistence` → no CLAUDE.md, hooks or MCP in the prompt |
| Cost of a variant | `input(base + component) − input(base)`, with the same base prompt for the 3 variants; 3 repetitions |
| Generations | 20 per variant and per task, concurrency 5; prompt = frozen excerpt of SPEC §1–3 (`phase0/spec-excerpt.md`, SPEC sha256 `0c71bfb7cdac…`) + reference example in the variant's style (+ dictionary for the cryptic one) + task |
| Validation | `phase0/check.mjs` (provisional checker until `brain check` existed): block order, forbidden syntax from §2, other frameworks' syntax, balanced blocks, keyed `each`, undeclared identifiers, `bind:` on a `sig`, assignment to a `drv`. Tested with 9 negative cases (all detected) and 0 false positives on `examples/` |
| Accessibility | `a11y-seo/a11y.mjs` (provisional lint of the markup rules in §8) |

Reproduce: `cd bench/phase0 && node run.mjs --only tokens --reps 3 && node run.mjs --only gen --task 1 && node run.mjs --only gen --task 2`.
Raw data: `phase0/results/tokens.json`, `generations*.json`, `gen*-<variant>.jsonl` and every generated file in
`phase0/results/gen*/<variant>/NN.brain`. Once `brain check` exists, those same files are revalidated.

The 3 variants (`phase0/variants/`) are the same component (a todo list with a filter) and only the names change:
`semantic` (`items`, `toggleItem`), `abbreviated` (`its`, `tgl`), `cryptic` (`t01`, `f02`, fields `f0..f2`;
`cryptic.dict.md` is its dictionary).

## 2. Input token cost

> **Note.** The tables below were measured 2026-10-13 with the Spanish-language variants (raw outputs archived outside the repo). Re-measured the same day with the English variants (`node bench/phase0/run.mjs --only tokens`, Haiku 5.5, 3 reps, deterministic): semantic 543 · abbreviated 541 · API-short 538 · cryptic 578 (+6.4 %) · cryptic+dict 743 (+36.8 %). Same conclusion: cryptic names save nothing.

Base = 565 tokens. The 3 repetitions gave the same value in every row (deterministic).

| Variant | Characters | Tokens Δ | vs semantic |
|---|---:|---:|---:|
| semantic | 1 290 | **560** | — |
| abbreviated | 1 139 | 558 | −0.4 % |
| cryptic | 1 130 | 595 | +6.3 % |
| cryptic + dictionary | 1 548 | 798 | +42.5 % |

Semantic names are whole words in the BPE vocabulary (≈1 token). `t01` splits into a letter + digits, so
writing with fewer characters **raises** the token count. The character savings of the abbreviated variant do not
translate into tokens.

## 3. Generation error rate (20 per variant and task)

### Task 1 — shopping list (structure close to the example)

| Variant | Pass the checker | Error rate | Prompt tokens | Output tokens (mean · median) | Style adherence | Cost USD |
|---|---:|---:|---:|---:|---|---:|
| semantic | 20/20 | 0 % | 2 549 | 724 · 706 | mean name length 6.8 | 0.0188 |
| abbreviated | 20/20 | 0 % | 2 547 | 678 · 675 | mean name length 3.0 | 0.0159 |
| cryptic | 20/20 | 0 % | 2 787 | 716 · 719 | 100 % `tNN/fNN` names (min. 83 %) | 0.0213 |

Ceiling effect: the task is too similar to the example and no style fails. That is why I ran task 2.

### Task 2 — cart (new state, derived values and event; departs from the example)

| Variant | Pass the provisional checker | Error rate | Prompt tokens | Output tokens (mean · median) | Style adherence | Cost USD |
|---|---:|---:|---:|---:|---|---:|
| semantic | 20/20 | 0 % | 2 656 | 1 035 · 1 041 | mean name length 8.2 | 0.0340 |
| abbreviated | 20/20 | 0 % | 2 654 | 921 · 934 | mean name length 3.2 | 0.0319 |
| cryptic | 20/20 | 0 % | 2 894 | 991 · 1 000 | 97 % `tNN/fNN` names | 0.0513 |

Source: `phase0/results/gen-t2-<variant>.jsonl` (aggregated with `jq -s`; `generations-claude-t2.json` only keeps the
last run, `apishort`). The provisional checker shows no difference either; revalidation with the real checker
(§3.1) does separate the variants.

### 3.1 Revalidation with the real checker (`brain check`, 2026-10-13)

The same 120 Haiku files (plus 51 from local qwen) run through `brain check --json <file>`.
Reproduce: `node bench/phase0/recheck.mjs` → `phase0/results/recheck.json`.
"No language errors" excludes B04xx/B05xx (accessibility/SEO, see §4).

| Model · task | Variant | No language errors | Codes |
|---|---|---:|---|
| Haiku · T1 | semantic | **20/20** | B0403×60 |
| Haiku · T1 | abbreviated | 20/20 | B0403×60 |
| Haiku · T1 | cryptic | 20/20 | B0403×60 |
| Haiku · T2 | semantic | **20/20** | B0403×20 |
| Haiku · T2 | abbreviated | 15/20 | B0004×4 (`products:list, ttl:…`: props separated by commas; the spec uses spaces and the frozen excerpt did not say so), B0111×1 |
| Haiku · T2 | cryptic | 20/20 | B0403×20 |
| local qwen · T1 | semantic | **16/20** | B0110×3, B0113×1, B0204×2 |
| local qwen · T1 | abbreviated | 11/20 | B0204×16, B0201×4, B0115×1 |
| local qwen · T1 | cryptic | 5/11 (incomplete run) | B0111×2, B0204×4, B0003, B0110 |
| Haiku · T1/T2 | `apishort` (API with short names, not implemented) | 0/20 · 0/20 | B0111, B0204: the real checker does not know that API; the experiment was only meaningful for tokens |

**Reading.** With the real checker, semantic is never worse: 40/40 on Haiku and the best on qwen (16/20 versus 11/20
abbreviated). Abbreviated introduces syntax errors that the provisional checker did not see. Cryptic ties on
Haiku, but costs +6.3 % tokens and its logic error (using `f1` where `f2` belonged) is still undetectable.

## 4. Side finding: the model copies the example's accessibility omissions

In task 1, all 60 generations (the 3 variants) have 3 inputs without a `<label>` or `aria-label` (B0403, 60/60
files), because the reference example did not have them either: `brain check` on `phase0/variants/*.brain` gives
B0403×2 for each one. The LLM replicates the example it sees, defects included.
**Implication for `brain spec` and the LLM test:** every example shown to the model must comply with §8 out of
the box, and the `fix` for B0403 must be concrete enough to apply without reasoning.

## 5. Limits

- A single model (Haiku 5.5) and two tasks. With more capable models the error rate should be equal or lower; the
  token difference is a property of the tokenizer and does not change.
- The provisional checker (§3) detected shape and naming errors; `brain check` (§3.1) is now the real one. Neither
  detects logic errors (for example, using `f1` where `f2` belonged in the cryptic variant passes if `f1` exists).
  This **favors** the cryptic variant: its real error rate is ≥ the measured one.
- `claude -p` method instead of `count_tokens`. If Chris adds `ANTHROPIC_API_KEY`, `count_tokens` will be used as a
  cross-check, not as a replacement.
