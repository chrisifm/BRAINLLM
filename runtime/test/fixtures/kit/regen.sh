#!/bin/sh
# Regenerates the kit fixtures from the REAL compiler output: serves form.view.brain with `brain run` and saves the
# compiled view and the runtime/ui kit it imports. Imports are rewritten for Node: `/@brain/ui/x.brain` →
# `./ui/x.brain.js`, `/@brain/` → `__RT__/`; the dev CSS <link> injection is dropped (no stylesheets in the shim).
set -e
here="$(cd "$(dirname "$0")" && pwd)"
repo="$here/../../../.."
brain="${BRAIN:-$repo/target/debug/brain}"
app="$(mktemp -d)"
mkdir -p "$app/views" "$app/channels"
printf '<html lang="en"><head><meta charset="utf-8"><title>t</title></head><body><slot /></body></html>\n' > "$app/app.brain"
echo "export default bus('toast')" > "$app/channels/toast.jsbrain"
cp "$repo/crates/brain-cli/src/templates/tokens.cssbrain" "$app/tokens.cssbrain" # the scaffold theme: the kit only uses its tokens
cp "$here/form.view.brain" "$app/views/form.brain"
port=$(node --input-type=module -e 'import net from "node:net"; const s=net.createServer(); s.listen(0,"127.0.0.1",()=>{console.log(s.address().port);s.close()})')
(cd "$app" && exec "$brain" run --port $port) >"$app/run.log" 2>&1 </dev/null &
pid=$!
trap 'kill $pid 2>/dev/null || true; wait $pid 2>/dev/null || true; rm -rf "$app"' EXIT
i=0
until curl -sf "http://127.0.0.1:$port/views/form.brain" >/dev/null; do i=$((i + 1)); [ $i -gt 50 ] && { echo "brain run did not start:" >&2; cat "$app/run.log" >&2; exit 1; }; sleep 0.2; done
fix() { sed -e '/^if(!document.querySelector(`link\[data-brain/d' -e 's|"/@brain/ui/\([a-z]*\)\.brain"|"./ui/\1.brain.js"|g' -e 's|"\./\([a-z]*\)\.brain"|"./\1.brain.js"|g' -e 's|"/@brain/|"__RT__/|g' -e 's|"\.\./channels/toast\.jsbrain"|"./toast.jsbrain.js"|g'; }
mkdir -p "$here/out/ui"
curl -sf "http://127.0.0.1:$port/views/form.brain" | fix > "$here/out/form.brain.js"
curl -sf "http://127.0.0.1:$port/channels/toast.jsbrain" | fix > "$here/out/toast.jsbrain.js"
for f in "$repo"/runtime/ui/*.brain; do
  n=$(basename "$f" .brain)
  curl -sf "http://127.0.0.1:$port/@brain/ui/$n.brain" | sed -e 's|"/@brain/ui/\([a-z]*\)\.brain"|"./\1.brain.js"|g' | fix > "$here/out/ui/$n.brain.js"
done
echo "kit fixtures regenerated in $here/out"
