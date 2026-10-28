# Incremental migration from React, Angular, Vue and related frameworks

This mode is an inventory and an LLM-guided workflow, not a JSX/TypeScript/template transpiler. It preserves
frontend behavior by tests and small cutovers. React, Angular, Vue, Svelte, Next.js and Nuxt are detected from
package metadata; dynamic routes and project conventions still require source inspection.

```sh
node /path/to/BRAINLLM/scripts/migrate-plan.mjs /path/to/original-app > migration-plan.json
brain new migrated-app
```

Keep the plan outside the original project. The read-only planner lists source files, route candidates and existing
tests without executing package scripts, reading environment files or following symlinks. It stops after 20000
directory entries or depth 20; inventory a monorepo one application at a time. Store working plans privately when
file names reveal private business information. Never upload the original source to a public framework PR.

## Preserve behavior before translating syntax

For one route or leaf component, record inputs/outputs, state transitions, events, validation, requests, auth,
permissions, cancellation, subscriptions, cleanup, URL/query behavior, storage, accessibility and visible states.
Run the original tests and capture a small reference flow with synthetic users. A failed original test is a baseline
finding, not permission to omit its behavior. Preserve the existing design and API contracts.

| Source concept | BRAINLLM direction; confirm exact syntax with `brain spec` |
|---|---|
| React state, Vue refs, Angular component state | `sig`, with immutable collection updates |
| Memo/computed values | `drv`; preserve dependency and update semantics |
| Effects/watch/subscriptions | `fx` with explicit teardown; verify cancellation and unmount behavior |
| Props, events and slots/children | The SPEC's props, emits and slots; test two-way behavior explicitly |
| Shared store/context/injection | Channels and provide/use contracts; preserve ownership and isolation |
| Fetch clients, forms and async views | `api`, `resource`, form and server contracts only where the served SPEC supports them |
| Router, guards and loaders | Map URL-by-URL to views/layouts/guards; preserve back/forward and query behavior |
| TypeScript, decorators, JSX, framework plugins | Re-express logic in supported BrainScript; keep unsupported libraries behind an explicit boundary |

Do not mechanically turn all hooks/watchers into effects: preserve initialization, dependency tracking and cleanup.
Do not ship a Next/Nuxt server secret to browser code when migrating SSR. Third-party components may need a separate
route boundary or a replacement with tested equivalent behavior; there is no general plugin compatibility claim.

## Repeatable slice and tests

1. Select a small leaf with few dependencies; preserve pure business logic with input/output tests first.
2. Port that slice into the separate BRAINLLM app using `brain add`, `brain spec` and `brain map --json`.
3. Test successful and failed API responses, loading/empty states, permissions, input validation and cancellation.
   Compare requests and side effects, not only screenshots. Cover keyboard and screen-reader behavior.
4. Run `brain check --json`, `brain doctor`, `brain build` and the slice's behavioral tests. Run the original
   tests again; use browser evidence for UI/interaction parity. Record actual outputs, never assumed passes.
5. Cut over only that verified route through the application's existing router/proxy configuration. Use separate
   DOM roots and route ownership. Preserve login, cookie scope, CSRF, origin/base paths and back/forward navigation.
   The planner does not install a proxy or modify deployment automatically.
6. Keep the original route available for rollback until acceptance. Repeat for dependencies, shared layout and
   increasingly complex routes. Remove the old framework only after all routes and integration tests pass.

Use this per-slice ledger, with a link to real evidence:

```json
{
  "source": "src/pages/profile.tsx",
  "target": "views/profile.brain",
  "behavior": ["load profile", "save validation", "401 redirect", "cancel on navigation"],
  "tests": [],
  "evidence": [],
  "status": "pending",
  "rollback": "route /profile back to the original application"
}
```

Valid progress states are `pending`, `porting`, `verifying`, `accepted` and `blocked`. A compiler gap goes to
[EVOLUTION.md](EVOLUTION.md), with a minimal synthetic reproducer and regression test. The LLM may improve the
compiler through that workflow; it must not fake feature support or silently change frontend logic.

## Prompt for the migrating LLM

> Read the target's served brain spec and docs/MIGRATION.md. Inventory the original project with migrate-plan.mjs.
> Establish its test baseline. Propose the smallest independent route/component slice and record its behavior,
> APIs, state transitions, permissions and failure states in the ledger. Port only that slice using implemented
> BRAINLLM primitives. Preserve the original project, styling and backend contracts. Verify behavioral and visual
> parity, cleanup, navigation, accessibility, doctor and production build before accepting the slice. Keep rollback.
> Record unsupported behavior explicitly; use EVOLUTION.md for compiler corrections. Continue slice by slice until
> every required behavior is covered. Never equate successful compilation with successful migration.
