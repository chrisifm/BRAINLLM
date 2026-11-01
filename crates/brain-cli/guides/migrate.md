# guide: migrate — move an existing app to the installed compiler, step by step

## When
After installing a newer `@christianirack/brainllm`, when `brain doctor` warns that the app's recorded version is older
than the compiler, or when a task needs a capability the app does not use yet. Moving a site from another framework is
a different task: `brain guide docs/MIGRATION.md`.

## Steps
1. Commit or stash first: the codemods edit source files and a clean diff is your review.
2. `brain guide whats-new` lists what the newest version added. `brain upgrade` (a dry run, writes nothing) is the same
   list measured against this app: from the version recorded in `brain.json` `brainVersion` (else the package pin, else
   the lockfile) to the installed compiler.
3. Read the report in this order: `codemods`, manual steps with `file:line`, capabilities the app does not use yet with
   the snippet that starts each one, then the gates.
4. `brain upgrade --apply` runs the safe codemods and the gates, and records the target version. If the package pin
   changed, run `npm install`.
5. Work through every manual step. A breaking or deprecated entry names the files it found; each stays in the report
   until the probe stops matching.
6. Adopt what helps the task; a capability is never required. Each one names its guide: read it before using the snippet.
7. Repeat `brain upgrade --json` until it exits 0, then `brain doctor`.

## Skipping versions
`brain upgrade --from 0.6.0` starts from an explicit version when the recording is missing or wrong; `--to` stops earlier
than the installed compiler. The report covers every release between them, in order, so do not upgrade one version at a
time unless a step asks for it.

## Pin only
`brain upgrade 0.8.0` (an exact version) previews the package pin change and the decisions left to a person. It writes
nothing and does not migrate source.

## Over MCP
`brain_upgrade` returns the same report; `apply: true` needs `confirm: true`.

## Verify
```bash
brain upgrade --json   # exit 0: no manual step or failing gate left
brain check --json     # []
brain doctor           # exit 0
brain build            # production build succeeds
```

## Common mistakes
| Mistake | Code | Fix |
|---|---|---|
| hand-editing the pin, then treating the app as migrated | — | `brain upgrade --apply` records the version and runs the gates |
| an old doc says a syntax is fine, the checker disagrees | — | the checker wins: `brain spec` describes the installed version |
| a new diagnostic appears after the upgrade | — | `brain explain <code>`, then `brain fix` for the exact edits |
| keeping a workaround for a fixed defect | — | `fix` entries in the report say which workarounds can go |
| `location.assign('/x')` flagged after an upgrade | B0129 | `navigate('/x')`; after a sign-out that must reload everything, see the fix text |
| running `--apply` with unreviewed local edits | — | commit first; `--apply` keeps revision checks but you still review the diff |

## Minimal correct example
```bash
git add -A && git commit -m "chore: before the compiler upgrade"
npm install -D @christianirack/brainllm@latest
npx brain guide whats-new
npx brain upgrade            # dry run: read it
npx brain upgrade --apply    # codemods + gates
npm install                  # only if the pin changed
npx brain doctor             # exit 0
```
