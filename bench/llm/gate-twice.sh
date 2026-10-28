#!/usr/bin/env bash
# Runs the LLM gate twice per model on the served spec and prints one line per run (first-try rate + gate verdict).
# Usage: bench/llm/gate-twice.sh <stage-label>   (e.g. gate-166)
set -euo pipefail
cd "$(dirname "$0")/../.."
label="${1:?stage label required}"
for m in claude-haiku-5-5 claude-sonnet-5-5; do
  for r in a b; do
    node bench/llm/run.mjs --model "$m" --mode spec --stage "$label$r" > "bench/llm/$label$r-$m.log" 2>&1
    node bench/llm/gate.mjs "bench/llm/results/$m-$label$r.json" || true
  done
done
