#!/bin/sh
# Regenerates the fixtures from the REAL compiler output (crates/brain-codegen/tests/e2e.rs).
# The runtime's file:// paths are replaced with __RT__/ so the test points at the tested build (BRAIN_RUNTIME).
set -e
cd "$(dirname "$0")/../../../.."
cargo test -q -p brain-codegen --test e2e >/dev/null
out=target/tmp/e2e
rt="file://$PWD/runtime/"
dst=runtime/test/fixtures/compiled
for f in comp/counter.brain.js counter.dev.js counter.dev2.js comp/todo.brain.js; do
  sed "s|$rt|__RT__/|g" "$out/$f" > "$dst/$(basename "$f")"
done
echo "fixtures regenerated in $dst"
