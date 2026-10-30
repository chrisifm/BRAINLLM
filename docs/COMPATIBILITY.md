# Compatibility contracts and release policy

`scripts/compatibility-check.mjs` compares explicitly versioned contract manifests.
It does not change package versions, install dependencies, execute migrations or
modify the existing release/LLM gates. There is no historical release baseline
bundled by this change. A baseline must be supplied explicitly and reviewed for
its actual release provenance.

```sh
node scripts/compatibility-check.mjs snapshot --output /tmp/brain-current-contract.json
node scripts/compatibility-check.mjs compare --baseline /path/to/released-contract.json --current /tmp/brain-current-contract.json --json
node --test scripts/test/compatibility-check.test.mjs
```

`snapshot` reads the current workspace version, top-level CLI help syntax,
registered author primitive names, literal diagnostic IDs in Rust source and
field names in the diagnostic JSON serializer. It writes a deterministic source
inventory with explicit coverage/limitations and `provenance.baseline: false`.
It refuses to overwrite an existing output. No network or compiled binary is
required. An inventory is partial evidence: it cannot infer primitive arity,
behavior, diagnostic severity or complete output schemas. Inline Rust tests may
contribute diagnostic literals. Names alone do not prove backward compatibility.

`compare` requires `--baseline`. Its current side is either an explicit
`--current` manifest or a fresh inventory from `--root` (default: this repository).
Exit codes are 0 for policy passed, 1 for policy violation, and 2 for malformed
inputs/options or an unsupported manifest. `compatible` concerns the declared
surfaces only; a breaking change with a sufficient major bump can pass the
version policy while remaining incompatible. The JSON report preserves inventory
limitations so a successful result cannot silently become a full compatibility
claim.

## Authored manifest

A reviewed release manifest can extend the source inventory with explicit input
and output schemas, diagnostic severity, primitive signatures and enabled
security invariants. All four surface objects must be present, even if empty.

```json
{
  "manifestVersion": 1,
  "frameworkVersion": "1.2.0",
  "security": { "failClosedAuthorization": true },
  "surfaces": {
    "commands": {
      "check": {
        "signature": "brain check [path]",
        "inputSchema": {
          "type": "object",
          "properties": { "path": { "type": "string" } }
        }
      }
    },
    "diagnostics": { "B0119": { "severity": "error" } },
    "primitives": { "sig": { "signature": "sig(initialValue)" } },
    "outputs": {
      "version": {
        "schemaVersion": 1,
        "outputSchema": {
          "type": "object",
          "properties": { "version": { "type": "string" } },
          "required": ["version"]
        }
      }
    }
  }
}
```

Entry metadata may include `description`, `evidence`, `deprecated`, `replacement`,
`renamedFrom` and `mechanicalFix`. A mechanical fix is a proposed instruction,
never executable code run by this guard. Unknown explicit contract values are
compared conservatively; changed values require a breaking release review.
Descriptions/evidence and schema prose do not change the API contract.

The schema comparator handles properties, required fields, enum sets and nested
array items directionally. New required inputs and previously guaranteed outputs
that become optional break compatibility. Input enum widening is additive;
output enum widening can break exhaustive consumers. Removed fields and changed
wire versions are breaking. Unknown changed schema constraints are treated as
breaking rather than assumed safe. This is a conservative subset, not a general
JSON Schema theorem prover or a substitute for behavioral regression tests.

## Version and upgrade policy

- For stable releases, removed/renamed contracts, changed signatures, diagnostic
  severity changes and incompatible output/schema changes require a major bump.
  For `0.x`, a minor bump is required; a patch cannot silently break the current
  minor line. Downgrades, including stable-to-prerelease downgrades, fail.
- Additions and deprecations require a minor bump and produce warnings. Preserve
  old names/aliases and behavior during migration. `renamedFrom` documents a
  rename; removing the old name remains breaking. Keeping its callable alias and
  deprecating it allows a minor release.
- Each breaking contract produces an upgrade-note warning. Add a checked
  mechanical fix where it is deterministic. Ambiguous semantic changes require
  a human decision; do not invent a migration or infer that changing the package
  pin updates source behavior.
- A sufficient version bump does not certify an upgrade or execute its fix.
  Run the normal compiler, runtime, fixture, English and applicable LLM gates,
  plus real app migration slices, before publishing or merging.
- Enabled top-level `security` flags are invariant requirements. Removing or
  disabling one fails even with a major bump. Explicit current manifests must
  retain the baseline's enabled invariants. Security behavior still needs its
  existing tests/review; an inventory does not claim to audit it. Do not weaken
  checks, authorization, sanitization or diagnostics to make compatibility pass.

Inputs are bounded to 4 MiB, 10,000 entries and 32 metadata nesting levels. Source
inventory scanning is bounded to 2,048 Rust files, 32 MiB and eight directory
levels; symlinks are refused. Contract schemas have their own 24-level limit.
No environment files, credentials or generated dependency directories are read.

## Complementary real engine matrix

`BRAIN_PLAYWRIGHT_MODULE=/absolute/path/to/playwright/index.mjs node --test --test-concurrency=1 scripts/test/browser-matrix.test.mjs`
runs the real compiler's counter/form fixtures plus native URL/dialog/history helpers in Chromium, Firefox and
WebKit, using isolated owned browser contexts and closing them in finally. It does not install browsers automatically
or replace installed Viewer E2E execution. Without the explicit module path the optional lane reports skipped.
The 2026-10-30 execution passed all three engines with one document request each. This verifies the covered engine
interactions, not physical iOS/Android devices or every API described by the compatibility dataset.
