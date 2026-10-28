# Clean-install test of the npm distribution (task #102)

Run of 2026-10-28: BRAINLLM 0.1.0, Node v22.21.1, npm 10.9.4, darwin-arm64. Work dir `~/tmp/brainllm-install` (temp HOME, npm cache and prefix). Total 12.7 s. Overall: **PASS**.

| Step | Result | Time | Numbers |
|---|---|---|---|
| 1 SHA256SUMS | PASS | 0.0 s | 5 entry verified; 4/4 tarballs listed |
| 2 npm i launcher + platform → npx brain --version | PASS | 4.1 s | install 1016 ms · `npx brain --version` first run 2590 ms, then 229 ms |
| 3 brain new demo → npm i -D → doctor → build → start | PASS | 5.9 s | install 785 ms · doctor 3876 ms · build 266 ms · 10 pages (12 .html, 117 files, 394.0 KB) · SEO 91–95 (6 indexable: /about 95, / 91, /posts/hello-world 95, /posts 95, /posts/second-post 95, /login 100) · start→/_health 200 164 ms · stop 51 ms |
| 4 npm i create + launcher + platform → npx create-brainllm app2 --no-install | PASS | 1.7 s | install 689 ms · create 654 ms · check 314 ms |
| 5 runtime package exports | PASS | 0.2 s | 29/29 files resolve · 19 named exports from `.` |
| 6 offline install + no writes outside the work dir | PASS | 0.7 s | offline install 201 ms, 0 registry requests · online runs: 26 registry GETs (status 404), 0 writes · new in ~: 0 · new in ~/.npm: 0 |
| 7 package hygiene | PASS | 0.0 s | 4 packages |

## Failed checks

- none

## Packages

| Package | Files | .tgz | Unpacked | README | LICENSE |
|---|---|---|---|---|---|
| `@christianirack/brainllm` | 4 | 3.0 KB | 8.2 KB | yes | no |
| `@christianirack/brainllm-darwin-arm64` | 3 | 2196.1 KB | 4227.7 KB | yes | no |
| `@christianirack/brainllm-runtime` | 54 | 103.7 KB | 300.4 KB | yes | no |
| `@christianirack/create-brainllm` | 3 | 1.1 KB | 2.0 KB | yes | no |

## Findings

| Owner | Severity | Finding | Fix |
|---|---|---|---|
| Chris / TOOLCHAIN (npm/pack.mjs) | blocker for publishing | no LICENSE file in any package; license field is "UNLICENSED" (npm shows it as not licensed for use) | Chris picks the license; then pack.mjs copies LICENSE into every package (already a known pre-publish item in npm/README.md) |
| TOOLCHAIN (npm/pack.mjs) | low | no `repository`/`homepage` in any package.json (npm provenance needs `repository` to match the repo) | add once the public repository exists (npm/README.md already lists it) |
