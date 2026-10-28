# npm packages

| Package | What it is |
|---|---|
| `@christianirack/brainllm` | `bin/brain.js` launcher + `brain.schema.json`; one optional dependency per platform |
| `@christianirack/brainllm-<os>-<arch>` | the native `brain` binary: darwin-arm64, darwin-x64, linux-x64, linux-arm64, linux-x64-musl, win32-x64, win32-arm64 |
| `@christianirack/create-brainllm` | `npm create @christianirack/brainllm <app>` |
| `@christianirack/brainllm-runtime` | the runtime ES modules (the binary embeds the same version) |

All share the workspace version of `Cargo.toml` (lockstep). The launcher runs the binary of the current platform; a
global `brain` delegates to the version the nearest project pins in `node_modules`; `BRAIN_BINARY=<path>` overrides
the binary and `BRAIN_NO_DELEGATE=1` skips the delegation.

## Pack (never publishes)

```sh
node npm/pack.mjs --dry-run        # builds the host binary into target/npm with local paths remapped out        # list what each package would contain
node npm/pack.mjs                  # npm/dist/*.tgz + npm/dist/SHA256SUMS (platforms without a binary are skipped)
```

Clean-install check: in a temporary directory and `HOME`, `npm i` the launcher and platform tarballs, then
`npx brain new demo`, `npm i -D` the same tarballs inside `demo`, `npx brain doctor` (exit 0) and `npm run build`.

## Release

`.github/workflows/release.yml` builds the 7 binaries, packs with `--require-all` and uploads the tarballs. It publishes
with `npm publish --provenance` only for a `v*` tag, after a manual approval in the `npm-publish` environment, using the
`NPM_TOKEN` secret. Publishing a version is Chris's decision, every time.

Before the first publish: choose the license (the workspace says `UNLICENSED`, so npm marks the packages as not
licensed for use) and add `repository`/`homepage` in `npm/pack.mjs` once the public repository exists.
