# Clean-install test of the npm distribution (task #102, TOOLCHAIN #19)

Installs the tarballs of `node npm/pack.mjs` the way a user would, in an isolated temp environment, and checks the
packages themselves. Nothing is published and nothing is written to any registry: every `.tgz` is installed from disk.

## Method

```sh
node bench/install/run.mjs              # cargo build --release -p brain-cli + node npm/pack.mjs, then steps 1-7 (~75 s)
node bench/install/run.mjs --no-build   # use npm/dist as it is
BRAIN_INSTALL_WORK=<dir> node bench/install/run.mjs   # work dir (default ~/tmp/brainllm-install, wiped first)
```

- Isolation: `HOME`, `npm_config_cache`, `npm_config_prefix`, `npm_config_userconfig` and `npm_config_logs_dir` all
  point inside the work dir; inherited `npm_*` variables are dropped. The tarballs are copied into the work dir first,
  so a concurrent `pack.mjs` cannot change what is tested.
- Steps:
  1. `SHA256SUMS`: every listed sum matches the file inside its tarball, and every `.tgz` has an entry.
  2. `npm i <launcher.tgz> <platform.tgz>` → `npx brain --version` = `brain 0.1.0`; `--version --json` is JSON;
     only the host's platform package is installed.
  3. `npx brain new demo` → `npm i -D` the same two tarballs inside `demo` → `npx brain doctor` exit 0 →
     `npm run build` exit 0 → `dist/` (pages, files, bytes; the build's SEO range and `brain seo audit --json` per page)
     → `npm start -- --port P` (the scaffold's `"start": "brain start"`): `GET /_health` → 200 `{ok, build}`,
     `GET /` → 200 → `brain stop`; the port must be closed afterwards. Leftover pids (from `brain status --json`) are
     killed by pid only.
  4. `npm i <create.tgz> <launcher.tgz> <platform.tgz>` → `npx create-brainllm app2 --no-install` → the app exists,
     pins `@christianirack/brainllm 0.1.0`, has no `node_modules`, and `brain check --json` prints `[]`.
  5. `npm i <runtime.tgz>`: `import.meta.resolve` of `.` and of every shipped `.js`/`.mjs` through `./*`, and
     `import('@christianirack/brainllm-runtime')` evaluates in plain Node.
  6. Step 2 again with an empty cache and `--offline` (0 registry requests, read from npm's debug log); no run makes a
     registry write; read-only listings of the real `~` and `~/.npm` (and `~/.npm/_logs` files that mention the work
     dir) before and after show no new entries.
  7. Hygiene: every tarball is unpacked; file count, `.tgz` and unpacked size; flags tests/demos/fixtures, `.map`,
     `.env`, dot-directories, logs, absolute home paths (`/Users/<user>/`, also inside the binary) and secret-like
     strings (`sk-ant-`, `sk-…`, `ghp_`, `gho_`, `AKIA`, private keys, npm tokens); checks README/LICENSE, `bin`,
     `engines`, `os`/`cpu`, `type`/`exports` and the 7 `optionalDependencies`.
- Output: `results/summary.json` and `results/summary.md` (committed; the home dir is written as `~`). Logs, the build
  and start output and the unpacked tarballs stay in the work dir. Exit 1 on any failed check.

## Latest local verification

The local closure run passed all seven steps on Node 22.21.1 / npm 10.9.4 (darwin-arm64), in 12.7 s.
All four tarballs have verified checksums, runtime dot-directories are excluded, and no absolute home paths or
secret-like strings were found. The scaffold passed doctor, build, start and stop; offline install made zero
registry requests. See `results/summary.md` for the generated evidence. License and public repository metadata
remain publishing decisions. This run validates the host platform only.

## Historical results (2026-10-13)

Run of 2026-10-13: BRAINLLM 0.1.0, Node v26.8.1, npm 11.19.0, macOS darwin-arm64. Full numbers in
[`results/summary.md`](results/summary.md).

Overall **FAIL**: the install flow itself passes end to end (steps 2-6); steps 1 and 7 fail on packaging findings below.

| Step | Result | Time | Numbers |
|---|---|---|---|
| 1 SHA256SUMS | FAIL | 0.0 s | 1 entry verified; 0/4 tarballs listed |
| 2 npm i launcher + platform → npx brain --version | PASS | 2.4 s | install 980 ms · `npx brain --version` first run 886 ms, then 372 ms |
| 3 brain new demo → npm i -D → doctor → build → start | PASS | 9.8 s | install 898 ms · doctor 7040 ms · build 316 ms · 10 pages (12 .html, 113 files, 349.3 KB) · SEO 91–95 (6 indexable: /about 95, / 91, /posts/hello-world 95, /posts 95, /posts/second-post 95, /login 100) · start→/_health 200 266 ms · stop 52 ms |
| 4 npm i create + launcher + platform → npx create-brainllm app2 --no-install | PASS | 1.9 s | install 779 ms · create 691 ms · check 427 ms |
| 5 runtime package exports | PASS | 0.3 s | 23/23 files resolve · 19 named exports from `.` |
| 6 offline install + no writes outside the work dir | PASS | 1.6 s | offline install 535 ms, 0 registry requests · online runs: 26 registry GETs (status 404), 0 writes · new in ~: 0 · new in ~/.npm: 0 |
| 7 package hygiene | FAIL | 0.0 s | 4 packages |

Package sizes:

| Package | Files | .tgz | Unpacked | README | LICENSE |
|---|---|---|---|---|---|
| `@christianirack/brainllm` | 4 | 2.7 KB | 6.7 KB | yes | no |
| `@christianirack/brainllm-darwin-arm64` | 3 | 1916.3 KB | 3791.7 KB | yes | no |
| `@christianirack/brainllm-runtime` | 38 | 81.8 KB | 234.8 KB | yes | no |
| `@christianirack/create-brainllm` | 3 | 1.1 KB | 2.0 KB | yes | no |

The first `npx brain --version` after install includes macOS's first-launch assessment of the new binary (0.9 s in this run, 5.6 s in an earlier one); later runs take ~0.3 s through `npx`, 0.06 s calling `node_modules/.bin/brain` directly. The 26 registry GETs of the online installs are the 404s for the 7 not-yet-published platform packages (optional dependencies, skipped as intended).

## Findings

| Owner | Severity | Finding | Fix |
|---|---|---|---|
| TOOLCHAIN (npm/pack.mjs) | medium | SHA256SUMS covers 1 native binary and 0 of 4 .tgz files, so a downloaded tarball cannot be verified against it | append `sha256  <file>.tgz` for every packed tarball after `npm pack` (or ship a second TARBALLS.SHA256SUMS) |
| Chris / TOOLCHAIN (npm/pack.mjs) | blocker for publishing | no LICENSE file in any package; license field is "UNLICENSED" (npm shows it as not licensed for use) | Chris picks the license; then pack.mjs copies LICENSE into every package (already a known pre-publish item in npm/README.md) |
| TOOLCHAIN (npm/pack.mjs) | low | no `repository`/`homepage` in any package.json (npm provenance needs `repository` to match the repo) | add once the public repository exists (npm/README.md already lists it) |
| BUILD (release profile) | medium | @christianirack/brainllm-darwin-arm64 bin/brain: 2 absolute home path(s), e.g. ~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/fsevent-sys-4.1.0/src/core_foundation.rs, ~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/notify-8.2.0/src/fsevent.rs: build-machine paths (cargo registry sources and/or the workspace) are embedded in panic/debug locations and leak the builder's user name | build releases with RUSTFLAGS `--remap-path-prefix=$HOME/.cargo/registry/src=/cargo` (stable; Cargo `trim-paths` is still unstable); CI builds would embed /home/runner/… instead |
| TOOLCHAIN (npm/pack.mjs) | low | @christianirack/brainllm-runtime ships ui/.brain/stuck.json: a local tool cache picked up because pack.mjs lists untracked files (`git ls-files --others`) | skip dot-directories (`/(^\|\/)\./`) in the runtime copy loop, or pack only tracked files |
