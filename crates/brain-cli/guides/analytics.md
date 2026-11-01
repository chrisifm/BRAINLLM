# guide: analytics — what visitors actually do, stored in the project's own Postgres

## When
Deciding what to fix or remove: which routes people reach, which elements they use, where they give up. It needs a native
backend (`brain guide backend`). Without one, `brain doctor` reports the info `B0729` and nothing is collected.

## Steps
1. Nothing to install: with `DATABASE_URL` and `BRAIN_BACKEND_URL` set, served pages load `/_brain/analytics.js`, which is
   added at serve time and costs no bytes in the route budget. Events go only to the project's own database.
2. Decide consent before launch: `"analytics": { "consent": "required" }` in `brain.json` records nothing until the app
   calls `consent('analytics')`. `consent('analytics', false)` stops it again.
3. Mark the outcomes you care about: `<button data-track="add_to_cart">`. Elements are already identified by component
   and position, so mark outcomes, not every control.
4. `brain build` writes `.brain/analytics/elements.json`, which resolves recorded elements to `file:line`; refresh it with
   `brain analytics map`.
5. Read before changing anything: `brain analytics report --since 7d --json`. Rage and dead clicks point at broken UI;
   `neverUsed` lists controls nobody touched.
6. Follow a path: `brain analytics funnel /shop /cart /checkout --since 14d` (2 to 8 steps, `/prefix/*` matches a section).
7. Find a group: `brain analytics segment --viewed /pricing --min-views 2 --not-viewed /signup --within 7`. Segments stay
   inside the project's database.
8. Retention and erasure: `retentionDays` (default 180) deletes old events by itself; `brain analytics purge --days 180`
   and `brain analytics erase --user <uuid>` act on demand.
9. Turn it off per process with `BRAIN_ANALYTICS=off`, or per project with `"analytics": false`.

## Privacy
Field values, typed text and clipboard contents are never sent. No IP address is stored. Visitors are random ids, and
Global Privacy Control, Do Not Track and automated browsers disable the collector. Your jurisdiction may still require
consent, so choosing `consent` is your decision. The full list: `brain guide docs/backend/analytics.md`.

## Verify
```bash
brain doctor                            # B0729 gone once the backend is attached
brain analytics report --since 1d       # your own visit appears with file:line sources
brain analytics map                     # after renaming or moving components
```

## Common mistakes
| Mistake | Code | Fix |
|---|---|---|
| no events and no backend | B0729 | set `DATABASE_URL` and `BRAIN_BACKEND_URL`, migrate, serve |
| no events with `consent: "required"` | — | nothing records until `consent('analytics')` runs |
| testing in an automated browser | — | `navigator.webdriver` disables the collector; use a normal window |
| elements reported without a source | — | run `brain build` or `brain analytics map` |
| putting personal data in `data-track` or event props | — | names and counts only; the rule is yours to keep, not machine-checked |
| adding a third-party tag for the same questions | — | read the report first; it answers without leaving your database |

## Minimal correct example
```html
<button data-track="checkout_started" on:click={start}>Checkout</button>
```
```bash
brain analytics report --since 7d
brain analytics funnel '/products/*' /cart /checkout --since 30d
```
