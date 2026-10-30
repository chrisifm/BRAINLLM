# Upgrade previews

An upgrade preview prepares an exact `@christianirack/brainllm` package pin change.
It does not install dependencies, contact a registry, execute a shell command or
claim that source migration is complete. The destination is an explicit semantic
version; tags, ranges, leading `v`, malformed identifiers and downgrades are
refused. Prereleases require review.

The CLI/MCP layer calls:

```rust
upgrade::preview(app_root, "0.4.0") -> Result<serde_json::Value, String>
```

The returned JSON includes `dryRun: true`, current/requested declarations,
`requiresHumanDecisions`, required gates and `operations` compatible with the
leased changes protocol. Each operation contains `path: "package.json"`, the
exact current SHA-256 `expectedRevision` and the proposed UTF-8 `content`.
An already matching pin returns an empty operation list. Preview never writes
package.json or creates a changes plan on its own.

The caller defaults to preview. To apply an explicitly requested pin change,
acquire a `package.json` lease, pass the operations to `changes::plan`, review the
result and then use `changes::apply`. Keep its optimistic revision guard; a
stale preview must be regenerated. The changes protocol provides validation and
conditional recovery for the source edit. It does not perform dependency
installation or certify the behavior of the requested framework version.

## What is preserved

The preview updates installable declarations in `dependencies`,
`devDependencies` and `optionalDependencies`. If the framework appears in multiple
sections, all matching declarations update consistently and the preview emits a
warning. It preserves peer dependency ranges and requests a separate review of
their compatibility. It refuses to invent a framework declaration if none exists.

All other effective JSON values are preserved. A changed file is serialized with
two-space indentation, so formatting/key order can change. Duplicate JSON keys
follow standard last-value parsing semantics and are normalized on serialization;
review such a file before applying. Input and output are bounded to 2 MiB; invalid
JSON, symlinks and detected secret literals are refused.

An existing tag/range can be replaced by the requested exact pin, but the preview
cannot infer the actually installed version from that range. Its decision list
requires resolving that version before evaluating source compatibility. A plain
package pin is not evidence of what a lockfile or installation currently uses.

## Source migrations and installation remain explicit

This implementation has no selected versioned source migration catalog. It
returns an empty `sourceMigrations` list and an explicit unverified migration
status. It never invents renames, rewrites app source or claims every version pair
is supported. A major upgrade, or minor upgrade while on `0.x`, calls out the
potential breaking change. Other releases still require reviewed release notes
and a real baseline; version arithmetic cannot establish semantic compatibility.

After applying a reviewed pin:

1. Review actual release notes and compare an explicit released compatibility
   manifest using `scripts/compatibility-check.mjs`. The current source inventory
   is not a historical release baseline.
2. Choose the app's existing package manager, install the exact requested version
   and review its lockfile changes. No installation or lockfile update is part
   of the preview or changes transaction.
3. Run `brain check`, app tests, `brain test`, `brain build` and migration slices
   under the installed target version. Resolve ambiguous changes with a human
   decision; keep existing security gates enabled.
4. Framework contributors additionally run workspace/runtime/fixture/English
   gates and the LLM gate when author-facing contracts change.

Only these later steps can establish whether the app upgrade is complete.

## Adoption of an actual previous release

The optional `scripts/test/upgrade-release.test.mjs` uses two explicitly supplied native compilers. It does not download binaries, run npm lifecycle scripts or install the destination package. Its previous compiler must report exactly 0.3.1. Obtain that compiler from a reviewed release artifact and verify its archive integrity before running the test; a fixture recreated using today's templates is not a historical baseline.

```sh
BRAIN_PREVIOUS_BINARY=/verified-release/brain \
BRAIN_CURRENT_BINARY=/current-checkout/target/debug/brain \
node --test scripts/test/upgrade-release.test.mjs
```

The test generates an app using the previous compiler, adds token/font, unknown configuration and opaque data/asset sentinels, and checks the app with both compilers. It adopts the design foundation through leased CAS operations, retains project-owned unknown baseline fields, rejects a stale package preview, and applies a regenerated exact package-pin operation. It then runs check/build with the explicitly supplied current compiler and compares every pre-existing source/data file by SHA-256. Only package.json may change; its other effective JSON values must survive. Private environment bytes are included in the in-memory preservation check and never printed.

The target 0.4.0 is only a package declaration in this regression. No registry availability check, installation or execution of that destination package occurs. A current checkout's successful check/build does not certify an unpublished release or every legacy app. The test has bounded command output and a five-minute work deadline, removes only its own temporary fixture, and skips explicitly when binaries are not supplied. A default offline suite containing that skip is not evidence that the released-app regression ran.

### Preserving existing performance budgets

A successful source check is not enough for adoption: the released-app regression also keeps the generated app's original `budget.viewKB` value. Optimization must reduce emitted or transferred bytes rather than enlarge that limit. In particular, server-rendered view styles are emitted as content-hashed stylesheet assets, with SRI and their actual compressed transfer size included in the route ledger. Personalized HTML and props remain separate from those compression inputs. Static views retain the existing small critical-style inlining policy.

Literal flat form schemas use compiler-only tracked helpers for exact unbounded `str`, `email`, `bool` and `num` rules, including required variants. Dynamic schemas, bounds, lists and nested fields retain the general parser. Text-only forms do not import the numeric validator. Initialization, validation messages, numeric conversion, boolean values, async checks and server submit identity use the same form lifecycle; this optimization does not reinterpret booleans or numbers as text.


### Released-app evidence (2026-10-30)

The preservation regression ran against the official npm artifact `@christianirack/brainllm-darwin-arm64@0.3.1`, downloaded from the npm registry and verified against its published SHA-512 integrity before extracting only the regular native compiler executable. No install hooks ran. The artifact reported `brain 0.3.1`; the target executable was built from the current checkout and reported `brain 0.4.0`.

The real regression passed without skips: 40 pre-existing non-package files retained their SHA-256 hashes through adoption, stale-CAS rejection, package-pin application and current check/build. Token/font bytes, opaque local data, unknown effective configuration values and unknown design fields survived. The original `budget.viewKB: 30` remained in effect. The opaque font sentinel verifies byte preservation, not font rendering. The exact package pin was applied locally; the target package was neither installed nor published by this test.

This is evidence for the tested generated 0.3.1 app and these guarded operations. It is not a universal source-migration catalog, a guarantee for arbitrary legacy apps or evidence of registry availability for 0.4.0.

Runtime export pruning uses stable module order and bounded fixed-point passes. A removed function's former imports cannot permanently retain another runtime module's parser or core exports. Explicit server roots, dynamic namespace access, import evaluation and observable effects stay conservative. The regression `scripts/test/build-determinism.test.mjs` builds the real default scaffold in separate processes after clearing only its own fixture caches, compares every emitted file and build signature, and enforces the unchanged 30 KiB budget. Supplying an existing `BRAIN_BIN` runs the real compiler; absence of a compiler is an explicit skip rather than proof.
