#!/usr/bin/env bash
# External a11y/SEO audit with Lighthouse — measurement only, never a project dependency (SPEC §8, LEAD 2026-10-13).
# Runs a pinned Lighthouse via npx (nothing is added to any package.json) against URLs served by `brain dev`/prerender.
# Usage: bench/a11y-seo/lighthouse.sh http://127.0.0.1:<port>/users [more URLs...]
set -euo pipefail
LH_VERSION=13.5.0   # latest stable on 2026-10-13 (npm view lighthouse version)
OUT="$(cd "$(dirname "$0")" && pwd)/results"
mkdir -p "$OUT"
for url in "$@"; do
  slug=$(printf '%s' "$url" | sed -E 's#^https?://##; s#[^A-Za-z0-9]+#_#g')
  npx --yes "lighthouse@${LH_VERSION}" "$url" \
    --only-categories=accessibility,seo --output=json --output-path="$OUT/$slug.json" \
    --chrome-flags="--headless=new --use-mock-keychain --password-store=basic" --quiet
  node -e 'const r=require(process.argv[1]);const c=r.categories;
    const fails=Object.values(r.audits).filter(a=>a.score===0).map(a=>a.id);
    console.log(JSON.stringify({url:r.finalDisplayedUrl,lighthouse:r.lighthouseVersion,a11y:Math.round(c.accessibility.score*100),seo:Math.round(c.seo.score*100),failing:fails}))' "$OUT/$slug.json" \
    | tee -a "$OUT/summary.jsonl"
done
