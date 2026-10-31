# Native capabilities without dependency setup

`brain lib list` and MCP `brain_lib` return the current function catalog. Existing `api/http/resource`, schemas/form,
store/bus, environment selection and Intl formatting remain the established implementations. New pure date and hash
functions use platform APIs and are auto-imported by the compiler; app authors do not install a date or crypto package.
They run in browser modules and Node server modules. Web Crypto requires a secure browser context or Node 22+.

| Function | Result and constraints |
|---|---|
| `dateParse(value)` | Epoch milliseconds from a valid Date, finite epoch or four-digit-year ISO date. Datetime strings require Z or an explicit offset; ambiguous and invalid calendar dates are rejected. Date-only strings mean UTC. |
| `dateAdd(value, amount, unit='day')` | New epoch. Elapsed units: ms/second/minute/hour/day/week. Integer month/year arithmetic uses UTC and clamps the original day to the destination month. Inputs are never mutated. |
| `dateDiff(left, right, unit='day')` | Signed fractional elapsed difference, left minus right. Calendar month/year differences are intentionally unsupported. A day means 24 hours, not a local DST calendar day. |
| `dateRange(start, end, options)` | Inclusive epochs with explicit step/unit and limit (default 1000, maximum 10000). Rejects overflow, stalled steps, direction errors and oversized results. Calendar steps apply repeatedly from the previous element. |
| `dateZone(value, timeZone)` | Read-only projection through Intl into an explicit IANA timezone, including its actual offset at that instant. It does not reinterpret a local wall-clock input as an instant. Formatter cache holds at most 16 zones; failed lookups do not evict valid entries. Numeric offset identifiers are rejected. |
| `hashText(text, algorithm='SHA-256')` | Async lowercase hex SHA-256, SHA-384 or SHA-512 using Web Crypto. UTF-8 input is limited to 8 MiB; no cryptographic algorithm is reimplemented and no Unicode normalization is applied. |
| `shortHash(text, length=32)` | Async SHA-256 prefix, 8–64 hex characters. Truncation increases collision risk; this is a content identifier, not password hashing or a signature. Backend passwords continue using Argon2id. |
| `httpCall(url, options)` | Async outbound HTTP for server code: per-attempt timeout, bounded retries only for idempotent requests, `Idempotency-Key`, response size cap, private-target refusal and redacted `HttpCallError`. See `docs/backend/outbound.md`. |

`fmt.relative` already exists for relative display. These helpers do not select the user's timezone, add date
annotations or fetch browser data during a build. Unsupported timezone identifiers and missing Web Crypto fail clearly.

## Inclusion and measurement

Functions are named exports in `brain-time.js` and `brain-hash.js`; the existing compiler imports only referenced
primitives and the bundler removes unused exports and their dependencies. There is no runtime library namespace or
new dependency in the core. Shared chunks still follow the existing bundler policy: per-component duplication must be
justified by measured compressed bytes rather than promised as universally smaller. `brain build --json` exposes the
emitted byte ledger. Compare a fixture before/after each helper; source-module gzip is not a network measurement.

## Authoring and contributing

`brain lib new normalize-title` atomically creates `lib/normalize-title.jsbrain` with one pure named export. It respects
active file leases and refuses overwrites, traversal, symlinks and built-in-name collisions. Import that export normally:

```js
import { normalizeTitle } from '../lib/normalize-title.jsbrain'
```

Keep inputs/outputs explicit and avoid module-level side effects. Run `brain check`, add behavior tests, then inspect
the ledger. A project helper does not become a compiler built-in automatically. To propose a reusable native module,
add its runtime exports, scoped primitive mappings, source documentation, boundary tests and byte fixtures in an
isolated framework contribution. Follow CONTRIBUTING.md and docs/EVOLUTION.md for review and release gates. No CLI
command opens or publishes a PR, changes a package version, or installs dependencies automatically.


## Date boundaries and verification

UTC calendar changes set the destination date directly, including day clamping;
valid dates at either epoch boundary do not overflow through an intermediate day
or month. Timezone offsets use Date/Intl over equivalent Gregorian 400-year cycles,
so projecting a valid instant beyond a local wall-clock boundary still produces a
finite offset. Historical offsets may contain fractional minutes. A repeated hour
at a DST fall-back keeps the two distinct instants and their respective offsets.

Range options are validated even for a one-element result. Limits count emitted
elements, including both endpoints when the stepping sequence reaches the end.
Fractional elapsed steps that round to the same millisecond are rejected when the
range needs to advance. Calendar month/year steps must always be integers.

Run `node --test runtime/test/native-libraries.test.js` for edge epochs, leap years,
DST folds, cache eviction, UTF-8 size boundaries, secure-context failures and hash
vectors. Tests resolve helper modules through the shared `BRAIN_RUNTIME` harness,
so a configured minified runtime tests those sibling modules too. A normal source
run does not itself prove minified output or browser-engine behavior; emitted
chunk integration remains part of the compiler/pack gates.
