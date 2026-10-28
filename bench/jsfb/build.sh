#!/usr/bin/env bash
# Build the BRAINLLM keyed implementation and install it into a js-framework-benchmark checkout.
# Usage: [BRAIN=/path/to/brain] [JSFB=<checkout>] bench/jsfb/build.sh [JSFB_DIR]
# The checkout is the first argument, else $JSFB, else ~/tmp/jsfb.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
JSFB="${1:-${JSFB:-$HOME/tmp/jsfb}}"
SRC="$HERE/brainllm"
DEST="$JSFB/frameworks/keyed/brainllm"
PREFIX="/frameworks/keyed/brainllm/_b/"

[ -d "$JSFB/frameworks/keyed" ] || { echo "not a js-framework-benchmark checkout: $JSFB" >&2; exit 1; }

# BRAIN=/path/to/frozen/brain pins the compiler binary (record its sha256); default: build the repo's CLI.
if [ -z "${BRAIN:-}" ]; then
  cargo build -q -p brain-cli --manifest-path "$REPO/Cargo.toml"
  BRAIN="$REPO/target/debug/brain"
fi
echo "brain binary: $BRAIN sha256=$(shasum -a 256 "$BRAIN" | cut -d' ' -f1)"

# Production build of the app (prerendered index.html + hashed modules under dist/_b/).
# brainllm/brain.json turns the service worker and the offline page off ("cache": {"sw": false, "offline": false}):
# the runner measures DOM operations on one page, a worker would only add a background fetch and a registration
# script that the other frameworks do not have.
(cd "$SRC" && rm -rf dist && "$BRAIN" check --json >/dev/null && "$BRAIN" build)

rm -rf "$DEST"
mkdir -p "$DEST"
cp -R "$SRC/dist/." "$DEST/"
# The benchmark server compresses responses itself (brotli); precompressed copies are not needed.
find "$DEST" -name '*.gz' -delete
# Files the benchmark page never requests (deploy configs, crawler and LLM files).
rm -rf "$DEST/robots.txt" "$DEST/_brain" "$DEST/_headers" "$DEST"/llms*.txt "$DEST"/*.md
# With "sw": false the build still writes a kill-switch sw.js (it unregisters a worker left by an earlier deploy) but the
# page never registers it; the benchmark origin never had one, so it is not installed.
if grep -q "serviceWorker" "$DEST/index.html"; then echo "index.html still registers a service worker: check brainllm/brain.json" >&2; exit 1; fi
rm -f "$DEST/sw.js"

# brain build emits root-absolute asset URLs (/_b/...); the benchmark serves each framework from
# /frameworks/keyed/<name>/, so rebase them. This only rewrites URLs; no code is changed.
for f in "$DEST/index.html" "$DEST"/_b/*.js "$DEST"/_b/*.css; do
  [ -f "$f" ] || continue
  sed -i '' -e "s#\"/_b/#\"${PREFIX}#g" -e "s#'/_b/#'${PREFIX}#g" -e "s#(/_b/#(${PREFIX}#g" "$f"
done
# The web app manifest link is root-absolute too: rebase it to the framework directory (the file is kept).
sed -i '' -e "s#href=\"/manifest.webmanifest\"#href=\"${PREFIX%_b/}manifest.webmanifest\"#" "$DEST/index.html"
if grep -q 'href="/manifest.webmanifest"' "$DEST/index.html"; then echo "unrewritten manifest link in $DEST/index.html" >&2; exit 1; fi
if grep -rq '"/_b/' "$DEST"; then echo "unrewritten /_b/ path left in $DEST" >&2; exit 1; fi
# The rebase changed the bytes of rewritten files, so the build's SRI hashes (integrity="sha384-…") no longer match.
# Recompute each one from the installed file it points to (no attribute is dropped).
node -e '
const fs = require("fs"), path = require("path"), crypto = require("crypto")
const [dest, prefix] = process.argv.slice(1), f = path.join(dest, "index.html")
let n = 0
const html = fs.readFileSync(f, "utf8").replace(/(href|src)="([^"]+)"([^>]*?)integrity="sha384-[^"]+"/g, (m, attr, url, mid) => {
  const file = path.join(dest, url.startsWith(prefix) ? "_b/" + url.slice(prefix.length) : url)
  if (!fs.existsSync(file)) { console.error("integrity on a missing file: " + url); process.exit(1) }
  n++
  return `${attr}="${url}"${mid}integrity="sha384-${crypto.createHash("sha384").update(fs.readFileSync(file)).digest("base64")}"`
})
fs.writeFileSync(f, html)
console.log("SRI recomputed: " + n)
' "$DEST" "$PREFIX"
if grep -q 'integrity=' "$DEST/index.html" && grep -o '<[^>]*integrity=[^>]*>' "$DEST/index.html" | grep -vq "$PREFIX"; then echo "an integrity attribute points outside $PREFIX" >&2; exit 1; fi

cat > "$DEST/package.json" <<JSON
{
  "name": "js-framework-benchmark-brainllm",
  "version": "1.0.0",
  "description": "BRAINLLM keyed implementation (built from BRAINLLM/bench/jsfb/brainllm by build.sh)",
  "js-framework-benchmark": {
    "frameworkVersion": "0.1.0",
    "frameworkHomeURL": "https://github.com/chriscorporation/BRAINLLM",
    "language": "JavaScript"
  },
  "scripts": {
    "dev": "exit 0",
    "build-prod": "exit 0"
  },
  "license": "Apache-2.0"
}
JSON

# The benchmark only lists framework directories that have a package-lock.json (no dependencies here).
cat > "$DEST/package-lock.json" <<JSON
{
  "name": "js-framework-benchmark-brainllm",
  "version": "1.0.0",
  "lockfileVersion": 3,
  "requires": true,
  "packages": { "": { "name": "js-framework-benchmark-brainllm", "version": "1.0.0", "license": "Apache-2.0" } }
}
JSON

echo "installed: $DEST"
ls -la "$DEST" "$DEST/_b"
