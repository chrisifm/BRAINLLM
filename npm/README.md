# Public npm distribution

BRAINLLM is MIT licensed. Scoped packages are configured with `publishConfig.access: public`; installing a published
release does not require an npm account, paid plan or private-registry credentials. Publishing still requires an
authorized maintainer of the `@christianirack` scope.

**Status: prepared for the first public npm release, not published by this change.** Use the root README's source
installation until registry verification confirms the launcher, platform packages and both initializers exist.

| Package | Role |
|---|---|
| `@christianirack/brainllm` | Native compiler launcher and configuration schema |
| `@christianirack/brainllm-<os>-<arch>` | Native binary selected by OS, CPU and libc |
| `@christianirack/create-brainllm` | Canonical `npm create @christianirack/brainllm@latest my-app` |
| `@christianirack/create-brainll` | Shorthand `npm create @christianirack/brainll@latest my-app` |
| `@christianirack/brainllm-runtime` | Runtime ES modules, already embedded in the native compiler |

Both initializers use the same source and depend on the exact launcher version. All packages share the workspace
version in `Cargo.toml`, the MIT license text, principal-author attribution and public repository/homepage links.
Node 22+ is required by the launcher and initializers.

## Build and inspect without publishing

```sh
node npm/pack.mjs --dry-run
node npm/pack.mjs --bins /path/to/reviewed-binaries --require-all --dry-run
node npm/pack.mjs --bins /path/to/reviewed-binaries --require-all
```

The first command builds the host release binary with private paths remapped out. The `--bins` form reads only that
explicit artifact directory; it never substitutes stale local binaries for missing artifacts. Seven binary folders
are required for a complete release: `darwin-arm64`, `darwin-x64`, `linux-x64`, `linux-arm64`, `linux-x64-musl`,
`win32-x64`, `win32-arm64`. Each contains `brain` or `brain.exe` on Windows.

Packing writes `npm/dist/stage/`, tarballs (unless dry-run) and `SHA256SUMS`. It never authenticates or publishes.
The launcher and initializer README files reuse the current root marketing README with absolute image and document
URLs so they render on npm. Keep the root README's release status accurate when an authorized release is verified.

A global launcher delegates to the project's pinned local version. `BRAIN_BINARY` overrides the native executable;
`BRAIN_NO_DELEGATE=1` disables delegation for controlled testing.

Run the packaging contract test with `node --test npm/test/pack.test.mjs`. It checks metadata and actual npm pack
manifests without compiling or requiring registry authentication. It does not prove native-platform functionality.

See [the release checklist](../docs/NPM-RELEASE.md) for publication and clean-install verification. The existing
release workflow must publish both initializer tarballs and retain the required manual environment approval.
