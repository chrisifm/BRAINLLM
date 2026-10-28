#!/usr/bin/env bash
# Local and CI verification of the current source tree. Never publishes.
set -euo pipefail
cd "$(dirname "$0")/.."

# Fixtures invoke target/debug/brain; cargo test alone does not refresh that binary.
cargo build --locked -p brain-cli
cargo test --locked --workspace
npm test --prefix runtime
node --test scripts/test/*.test.mjs
node bench/fixtures-check.mjs
node scripts/privacy-audit.mjs
scripts/check-english.sh
