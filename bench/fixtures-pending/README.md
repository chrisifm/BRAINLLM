# bench/fixtures-pending/ — fixtures for rules the checker does not implement yet

Same contract as `examples/invalid/README.md` (exact code/file/line/col; non-empty msg and fix; a single Diag per
case). They live here and not in `examples/invalid/` so they do not break `cargo test -p brain-check` before the rule exists.
**COMPILER:** when implementing a rule, move its `.brain` + `.expected.json` pair to `examples/invalid/comp/` (change
`file` to `invalid/comp/…`).

`level` is QA's proposal per INTERNALS §20 (an error blocks the build; a warning does not). The comparer ignores it until the
Diag exposes it.

Validation done: with the current checker, `brain check --json bench/fixtures-pending` gives `[]`, i.e. no case
triggers another existing rule (zero collateral hits).

| Code | Case | Rule (INTERNALS §20) | Level |
|---|---|---|---|
| B1201 | `comp/b1201-unsupported-css.brain` | `field-sizing` outside the `baseline-widely` target | error |
| B1202 | `comp/b1202-experimental-no-guard.brain` | `moveBefore` without feature detection | warning |
| B1203 | `comp/b1203-deprecated.brain` | deprecated `substr` | warning |
| B1301 | `comp/b1301-await-in-for-of.brain` | independent `await` in `for-of` | warning |
| B1302 | `comp/b1302-fetch-no-catch.brain` | `fetch` without error handling | warning |
| B1303 | `comp/b1303-nested-loop.brain` | `find` inside `map` over the same list | warning |
| B1304 | `comp/b1304-fx-no-deps.brain` | `fx` without reactive dependencies | warning |
| B1305 | `comp/b1305-heavy-in-each.brain` | `sort` inside `{#each}` | warning |
| B1306 | `comp/b1306-timer-no-cleanup.brain` | `setInterval` in `fx` without cleanup | warning |
| B1307 | `comp/b1307-magic-number.brain` | literal `3000` in `setTimeout` | warning |
| B1308 | `comp/b1308-console.brain` | `console.log` | warning |

Check once the rule exists: `node bench/fixtures-check.mjs` after moving them to `examples/invalid/`.

## B09xx, B10xx and B1101 (#41; INTERNALS §15 and §18)

| Code | Case | Proposed level |
|---|---|---|
| B0901 | `comp/b0901-fx-self-cycle.brain` | error |
| B0903 | `comp/b0903-recursion-no-base.brain` | error |
| B1001 | `comp/b1001-drv-passthrough.brain` | warning |
| B1004 | `comp/b1004-fx-writes-only.brain` | warning |
| B1005 | `comp/b1005-unused-prop.brain` | warning |
| B1009 | `comp/b1009-dead-function.brain` | warning |
| B1101 | `comp/b1101-await-no-catch.brain` (+ `api/users.jsbrain`) | error |
| B1002, B1003, B1006, B1007, B1008 | `doctor-app/` (they need to see the whole project) → `doctor-app/doctor.expected.json` | info (`brain doctor`) |

B0902 (cut-off at 100 re-runs) and B0904 (unbounded list) are runtime rules: they are tested in the RUNTIME tests
(#39), not with static fixtures. `doctor-app` with the current checker gives `[]` (zero collateral hits); its
`doctor.expected.json` is what `brain doctor --json` must return (or `brain check` if the rules are moved into check).
