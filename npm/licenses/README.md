# Pinned public license supplements

`cargo-notices.mjs` copies LICENSE/NOTICE/COPYING/COPYRIGHT/AUTHORS texts from the locked Cargo source graph, including nested vendored notices. These few supplements cover exact crate releases whose published archives omit their upstream license files. The catalog binds each exception to the crate version, declared license, published Cargo VCS commit and checked-in text SHA-256. Generation never downloads replacement license texts.

The objc2 overview is preserved in full, including its Apple SDK qualification. Canonical SPDX license templates accompany its declared alternatives without inventing copyright holders or deciding SDK obligations. This inventory is redistribution evidence, not an automatic license-compatibility assessment or a linker-level proof of which code executes.

After a native build, run `cargo fetch --locked` to cache the metadata sources (including unbuilt workspace manifests), install the matching toolchain's `rust-docs` component and run:

```sh
node npm/cargo-notices.mjs --binary bins/<platform>/brain --target <rust-target>
```

Use brain.exe on Windows. The four sidecars bind public notices and the official standard-library and complete toolchain copyright documents to the binary SHA-256, Cargo.lock, target and framework version. Platform npm packages retain those sidecars and include Cargo text in THIRD_PARTY_NOTICES.txt. Packing rejects missing or mismatched bundles before replacing its staging output. Regenerate after a binary, lock, target or dependency change; do not copy a bundle from another platform/build.

The complete toolchain document also preserves vendor/builtin attributions not listed in the narrower standard-library document. It intentionally over-includes notices for compiler tools that the application does not redistribute as executables.
