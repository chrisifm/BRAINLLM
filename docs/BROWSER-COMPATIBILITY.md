# Browser compatibility evidence

The compatibility table is generated from the official `web-features` dataset.
Its declarations are platform evidence, not proof that arbitrary application
objects implement a browser interface. The application checker is deliberately
conservative. There is no automatic polyfill injection or syntax lowering in this
implementation.

## Dataset provenance and offline builds

The pinned package is `web-features` **3.44.0**, verified as the latest stable npm
release during this integration. npm records its publication as
`2026-10-22T16:30:26.001Z`. That publication date is source metadata, not a local
review timestamp. Future releases require an explicit reviewed pin update.

- Source: <https://github.com/web-platform-dx/web-features>.
- Official package: <https://registry.npmjs.org/web-features/3.44.0>.
- License: Apache-2.0; the upstream license is preserved in the linked notice below.
- Data SHA-256: `72577e8df9c8abe8a8af86a03bc161ac06be21488d76857f157c7f83f017f706`.
- The generator also pins and verifies the archive's npm SHA-512 integrity before
  extracting only `package.json` and `data.json`, without unpacking archive paths.

Builds and normal checks use the checked-in Rust table and never download data.
Maintenance generation is explicit:

```sh
node scripts/gen-baseline.mjs --fetch
node scripts/gen-baseline.mjs --fetch --check
node scripts/gen-baseline.mjs --data /path/to/official/data.json --check
node --test scripts/test/baseline-generator.test.mjs
```

`--data` accepts only the exact pinned official bytes, verified before parsing.
Importing the generator performs no network requests or writes. Downloads,
extraction, input size and feature/key counts are bounded. Source extraction and
download share a 30-second deadline; temporary files belong to that operation and
are removed in `finally`. Writes use a unique temporary file and atomic rename.
Output is deterministic, with source package version, publication date, archive
integrity, data hash and the sorted known-global names' hash embedded in the table.

## Table interpretation

Each row contains the key, Baseline year, widely-available status, browser support
mask, source feature indexes, match kind or receiver owner, and exact availability
dates. A zero year means limited availability. Upstream dates beginning with `≤`
are preserved as upper bounds rather than converted into invalid numeric years.
Browser version strings are represented only by presence in the support mask;
this table does not implement arbitrary minimum browser-version queries.

Static paths distinguish `Array.from` from `Array.prototype.at`,
`Set.prototype.union` and `DataView.prototype.getFloat16`. Legacy normalized BCD
keys are explicitly mapped to actual source properties such as `__defineGetter__`.
Instance member records retain their owner; a method name alone never proves its
receiver type. Unknown bindings and aliases remain unclassified.

All available raw compatibility keys remain in `COMPAT_KEYS`, including CSS
values/functions, syntax subfeatures and event metadata. `UNCLASSIFIED` identifies
keys without a source matcher, including features for which upstream supplies no
compatibility-key mapping. Raw evidence is not advertised as implemented checking.
No old widely-available rows are dropped by an arbitrary year cutoff. Conflicting
mappings combine support conservatively and retain all contributing feature IDs.

## Initial application checker

`browser_compat::check(component, context)` returns warnings with source locations
and actionable alternatives. It recognizes `baseline-widely` (the default),
`baseline-newly`, and internal `baseline-YYYY` targets from 2015 through the pinned
publication year. Existing public configuration uses `brain.json` `targets`;
configuration exposure and the diagnostic catalog are maintained by CLI wiring.

B2101 checks statically recognized global paths. B2102 checks component CSS
properties, pseudo selectors, at-rules, static HTML tags/attributes, literal inline
styles and style directives. B2103 reports an invalid target. Server modules,
server functions and exported server `load` bodies are excluded from browser API
checking. No unknown receiver is guessed from a distinctive method name.

Positive feature tests protect their own true branch, not the surrounding file.
Recognized proofs include literal `in`, supported `typeof` comparisons, truthy
member checks, logical AND, conditional expressions and optional calls. An OR
condition does not prove either operand inside the branch. Negative availability
probes are safe reads, without making their positive body supported. A single
positive CSS property or `selector(...)` test in `@supports` suppresses only that
matching feature in its block; negative and combined conditions are deliberately
not treated as proof.

Shadowed known-global names are conservatively excluded across the component,
including function parameters and local declarations. This can miss an unrelated
unshadowed use elsewhere in the file; it avoids attributing an application's
custom object to the platform. This first slice does not infer instance receiver
types, recognize arbitrary application polyfills, lower syntax, prove fallback
behavior, match every CSS value/function or run real browser engines. A clean
check is not a universal browser-support guarantee. Raw data and unclassified keys
make these limits explicit for future extensions. At most 100 compatibility
findings are produced per component.

## Verification boundaries

Generator regression tests use synthetic local fixtures and no network. They
cover static/prototype separation, normalized aliases, owner-qualified members,
raw CSS evidence, upper-bound dates, metadata overrides, conservative collisions,
deterministic provenance, import side effects and rejection of unverified data.

Checker regression tests exercise actual parser ASTs for unsupported globals,
source locations, shadowing, server exclusion, branch-scoped feature guards,
optional calls, CSS guards, selector strings, unsupported targets and the newly
available target. Workspace verification must run after checker wiring. Real
Chromium, Firefox and WebKit compatibility fixtures and syntax lowering remain
separate unfinished work; no engine coverage is claimed here.

## Upstream license

The upstream license applies to the source `web-features` dataset and generated
data derived from it. Keep this notice with redistributed generated data.

See [web-features-LICENSE.txt](licenses/web-features-LICENSE.txt) for the full
upstream license reproduced from the integrity-verified official package.
