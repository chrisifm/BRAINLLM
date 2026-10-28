#!/usr/bin/env bash
# Reproduces the production-build benchmark: BRAINLLM vs Svelte 5 + Vite vs React 19 + Vite on the same app.
# The BRAINLLM app is the frozen fixture bench/build/app/ (the users dashboard the ref/ ports implement), never
# examples/, so a change to the examples cannot make the comparison unequal.
# Usage: bench/build/run.sh   (env: RUNS=20 WARMUP=2 BENCH_WORK=$HOME/tmp/brainllm-build BRAIN_BIN=<prebuilt brain>)
# BRAIN is accepted as an alias of BRAIN_BIN.
# Requires: cargo, node/npm, hyperfine, gzip, brotli. Writes results/hf-*.json, versions.json, summary.{json,md}.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
WORK="${BENCH_WORK:-$HOME/tmp/brainllm-build}"
RUNS="${RUNS:-20}"
WARMUP="${WARMUP:-2}"
OUT="$HERE/results"

mkdir -p "$WORK/bin" "$OUT"
rm -f "$OUT"/hf-*.json

echo "== 1. release binary (frozen copy so a concurrent rebuild cannot change it mid-run)"
# BRAIN_BIN=/path/to/brain skips the cargo build (useful while the tree does not compile).
# Copies remove the destination first: macOS kills a binary copied over a previous copy of itself.
BRAIN_BIN="${BRAIN_BIN:-${BRAIN:-}}"
if [ -n "$BRAIN_BIN" ]; then
  if ! [ "$BRAIN_BIN" -ef "$WORK/bin/brain" ]; then rm -f "$WORK/bin/brain"; cp "$BRAIN_BIN" "$WORK/bin/brain"; fi
else
  (cd "$ROOT" && cargo build --release -p brain-cli --quiet)
  rm -f "$WORK/bin/brain"
  cp "$ROOT/target/release/brain" "$WORK/bin/brain"
fi
BRAIN="$WORK/bin/brain"

echo "== 2. app copies"
rm -rf "$WORK/app" && mkdir -p "$WORK/app"
rsync -a --exclude dist --exclude .brain "$HERE/app/" "$WORK/app/"
for ref in svelte react; do
  mkdir -p "$WORK/$ref"
  rsync -a --delete --exclude node_modules --exclude dist "$HERE/ref/$ref/" "$WORK/$ref/"
  (cd "$WORK/$ref" && npm ci --no-audit --no-fund --loglevel=error)
done

echo "== 3. sanity builds"
# A failing build stops the run with its own output (set -e alone would hide it behind >/dev/null).
if ! (cd "$WORK/app" && rm -rf dist && "$BRAIN" build > "$WORK/brain-build.log" 2>&1); then
  echo "brain build failed:" >&2; cat "$WORK/brain-build.log" >&2; exit 1
fi
for ref in svelte react; do (cd "$WORK/$ref" && rm -rf dist && npx vite build --logLevel error); done

echo "== 4. build time"
LOAD_START="$(sysctl -n vm.loadavg 2>/dev/null | tr -d '{}' | xargs)"
hf() { # name, cwd, prepare, command — run without a shell (-N) from the app directory
  (cd "$2" && hyperfine -N --style basic --warmup "$WARMUP" --runs "$RUNS" --prepare "$3" --export-json "$OUT/hf-$1.json" "$4")
}
hf brain-cold "$WORK/app" "rm -rf dist .brain/cache" "$BRAIN build"
hf brain-warm "$WORK/app" "rm -rf dist"              "$BRAIN build"
for ref in svelte react; do
  hf "$ref-cold" "$WORK/$ref" "rm -rf dist node_modules/.vite" "npx vite build --logLevel silent"
  hf "$ref-warm" "$WORK/$ref" "rm -rf dist"                    "npx vite build --logLevel silent"
done
# Leave a fresh dist of each app for the byte count.
(cd "$WORK/app" && rm -rf dist && "$BRAIN" build >/dev/null 2>&1)
for ref in svelte react; do (cd "$WORK/$ref" && rm -rf dist && npx vite build --logLevel silent); done

echo "== 5. tool cold start (informational)"
hf brain-start "$WORK/app"    "true" "$BRAIN help"
hf vite-start  "$WORK/svelte" "true" "npx vite --version"

LOAD_END="$(sysctl -n vm.loadavg 2>/dev/null | tr -d '{}' | xargs)"

echo "== 6. versions"
pkg() { node -p "require('$WORK/$1/node_modules/$2/package.json').version"; }
cat > "$OUT/versions.json" <<JSON
{
  "brain binary sha256 (12)": "$(shasum -a 256 "$BRAIN" | cut -c1-12)",
  "brain binary size (bytes)": "$(wc -c < "$BRAIN" | tr -d ' ')",
  "rustc": "$(rustc --version)",
  "node": "$(node --version)",
  "npm": "$(npm --version)",
  "vite": "$(pkg svelte vite)",
  "svelte": "$(pkg svelte svelte)",
  "@sveltejs/vite-plugin-svelte": "$(pkg svelte @sveltejs/vite-plugin-svelte)",
  "react / react-dom": "$(pkg react react) / $(pkg react react-dom)",
  "@vitejs/plugin-react": "$(pkg react @vitejs/plugin-react)",
  "hyperfine": "$(hyperfine --version | cut -d' ' -f2)",
  "cpu": "$(sysctl -n machdep.cpu.brand_string 2>/dev/null || uname -m), $(sysctl -n hw.ncpu 2>/dev/null) cores",
  "memory (GiB)": "$(( $(sysctl -n hw.memsize 2>/dev/null || echo 0) / 1073741824 ))",
  "load average 1/5/15 min (start → end of timing)": "$LOAD_START → $LOAD_END",
  "os": "$(sw_vers -productName 2>/dev/null) $(sw_vers -productVersion 2>/dev/null) ($(uname -r))"
}
JSON

echo "== 7. summary"
node "$HERE/summarize.mjs" "$WORK" "$OUT"
