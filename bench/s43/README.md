# §43 bench: the agent loop before vs after, and a large app by components (task #202)

INTERNALS §43.5: time the agent loop (brief → app with `brain doctor` exit 0) before and after the §43 tools on the
same briefs, and have a model build a 60+ component app using only cards and the map, with 0 reads of files over
200 lines. Results: `results/summary.md` and `results/summary.json`.

## What is measured

Harness: `bench/autonomy/run.mjs --mode mcp` (only the brain MCP server, no Bash, no web, no human turns). Done = an
independent `brain doctor --json` exits 0 after the agent stops and `views/` holds at least the routes the brief asks for.

Per run, on top of the harness metrics (tool calls per tool, tokens, cost, wall time, stuck loops):

| Metric | Source |
|---|---|
| `reads` | `brain_read` calls in the transcript |
| `bigReads` | reads whose returned file has more than 200 lines, or is paged (`nextOffset`, over 60 KB) |
| `maxReadLines`, `readList` | lines of every file each read returned |
| `doctorCalls`, `firstDoctorOk` | `brain_doctor` calls, and whether the first one was already clean |
| `tools` | the tool list the agent really saw (the `init` event): proves the denied tools were absent |

### (a) Agent loop, before vs after

Same binary, same five briefs (`bench/autonomy/briefs.json`), Haiku and Sonnet. The "before" arm is an ablation:
`--deny brain_card,brain_impact,brain_why,brain_split` passes them to `claude --disallowedTools`, which removes them
from the agent's tool list. Verdict per model: after slower than before in median wall time → **FAIL**.

Honest limits of the ablation:
- `brain_map`'s §43 additions (the `reverse` graph and the `view`/`comp`/`reads` filters, pagination) cannot be
  removed by denying a tool: the "before" arm still gets them, so "before" is slightly better than the real pre-§43
  toolchain and the measured gain of "after" is a lower bound.
- The same holds for B1010 (the size warning with its `brain split` edit), which `brain_check`/`brain_doctor`
  report in both arms, and for `brain_map`'s description, which names `brain_card` in both arms.
- Five briefs per arm, one run each: model variance per brief is large compared with the difference measured;
  read the per-brief table before drawing conclusions.
- The arms ran in parallel on one machine with the large-app runs, but not all at the same moment: the first launch
  of the "before" arms passed `--deny` glued to its value (an unquoted zsh variable is not word-split), the harness
  ignored it and both "before" runs were in fact "after" runs. They were stopped (Haiku) or kept (Sonnet, complete)
  as an A/A replica (`s43-aa-sonnet`, arm `aa` in the summary): its gap against "after" is the run-to-run noise.
  The real "before" arms were launched again later, under a different background load. `run.mjs` now rejects any
  argument that is not a known `--flag value` pair, so that mistake fails loudly.

### (b) Large app by components

`briefs-202.json`: one brief for a store back office with 11 pages that asks for at least 60 small components,
composed with `brain_map`/`brain_card` and split with `brain_split`, never reading whole large files. "After" arm only,
Sonnet and Haiku, timeout 3600 s, 400 turns. Verdict per model: `brain map --json` total components ≥ 60, bigReads = 0
and doctor exit 0 → **PASS**. The fresh `brain new` scaffold already has 14 map components (views included) and 8
routes; `summary.md` also shows the non-view components and the routes. `B1010 at the end` = B1010 warnings in a
final `brain check --json`.

## Reproduce

```bash
cargo build --release -p brain-cli
# freeze one copy for every arm (remove first: macOS kills a binary copied over a previous copy)
mkdir -p ~/tmp/brainllm-s43 && rm -f ~/tmp/brainllm-s43/brain && cp target/release/brain ~/tmp/brainllm-s43/brain
B=~/tmp/brainllm-s43/brain
D=brain_card,brain_impact,brain_why,brain_split

# (a) agent loop: 5 briefs × {haiku, sonnet} × {before, after}
node bench/autonomy/run.mjs --model claude-haiku-5-5  --mode mcp --brain $B --tag s43-before-haiku  --timeout 1800 --deny $D
node bench/autonomy/run.mjs --model claude-haiku-5-5  --mode mcp --brain $B --tag s43-after-haiku   --timeout 1800
node bench/autonomy/run.mjs --model claude-sonnet-5-5 --mode mcp --brain $B --tag s43-before-sonnet --timeout 1800 --deny $D
node bench/autonomy/run.mjs --model claude-sonnet-5-5 --mode mcp --brain $B --tag s43-after-sonnet  --timeout 1800

# (b) large app
node bench/autonomy/run.mjs --model claude-sonnet-5-5 --mode mcp --brain $B --briefs ../s43/briefs-202.json --tag s43-large-sonnet --timeout 3600 --max-turns 400
node bench/autonomy/run.mjs --model claude-haiku-5-5  --mode mcp --brain $B --briefs ../s43/briefs-202.json --tag s43-large-haiku  --timeout 3600 --max-turns 400

# a run that hung until the timeout is re-run once under <tag>-r2 (only that brief); in this bench:
node bench/autonomy/run.mjs --model claude-haiku-5-5 --mode mcp --brain $B --tag s43-before-haiku-r2 --only inventory --timeout 1800 --deny $D
node bench/autonomy/run.mjs --model claude-haiku-5-5 --mode mcp --brain $B --tag s43-after-haiku-r2  --only inventory --timeout 1800
node bench/autonomy/run.mjs --model claude-haiku-5-5 --mode mcp --brain $B --briefs ../s43/briefs-202.json --tag s43-large-haiku-r2 --timeout 3600 --max-turns 400
# A/A noise reference (an "after" replica):
node bench/autonomy/run.mjs --model claude-sonnet-5-5 --mode mcp --brain $B --tag s43-aa-sonnet --timeout 1800
node bench/s43/summarize.mjs
```

Output of the harness (gitignored): `bench/autonomy/results/<tag>/<id>/` (the app), `<tag>/<id>.transcript.jsonl` and
`<tag>.json`. `summarize.mjs` reads those and writes `results/summary.{md,json}` with the home directory as `~`.

## Run log (2026-10-13, brain 3051072d05ce, spec 60155901d653)

- Hung until the timeout and re-run once: haiku/before/inventory and haiku/after/inventory (`brain_eval`), the Haiku
  large app (`brain_navigate`). haiku/after/inventory hung again in its re-run (same `brain_eval` pattern); the
  summary uses that re-run, timed out, and also shows the medians with timed-out runs left out.
- The first launch of the Haiku large-app re-run lost its harness process after about 17 minutes (no result, no
  transcript; not a timeout, most likely an external kill on the shared machine). It was launched again under the
  same tag; the summary uses that second launch.
