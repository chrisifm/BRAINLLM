#!/usr/bin/env bash
# Local and CI verification of the current source tree. Never publishes.
set -euo pipefail
cd "$(dirname "$0")/.."

# Fixtures invoke target/debug/brain; cargo test alone does not refresh that binary.
cargo build --locked -p brain-cli
# This real-browser lifecycle check runs in its own test process so unrelated unit subprocesses cannot compete
# with Chrome startup. The workspace pass excludes only the check already required immediately above it.
cargo test --locked -p brain-cli --bin brain browser::tests::comparison_closes_only_its_temporary_tab_on_success_and_evaluation_failure -- --exact
cargo test --locked --workspace -- --skip browser::tests::comparison_closes_only_its_temporary_tab_on_success_and_evaluation_failure
npm test --prefix runtime
node --test --test-concurrency=2 scripts/test/*.test.mjs
node --test bench/fixtures-check.test.mjs bench/agents/runner.test.mjs bench/llm/codex.test.mjs bench/llm/check.test.mjs bench/llm/compare.test.mjs
node --test --test-concurrency=2 npm/test/*.test.mjs
node bench/fixtures-check.mjs
node scripts/source-inputs-check.mjs
node scripts/privacy-audit.mjs
scripts/check-english.sh
