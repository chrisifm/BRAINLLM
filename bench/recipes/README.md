# Recipes v1 (INTERNALS §27, task #83)

Verified building blocks for the common 80 % of an app. Each recipe is a folder of real BRAINLLM sources laid out
exactly like an app (paths relative to the app root) plus a `recipe.json` manifest. Copied into a fresh `brain new`
app, every recipe passes `brain check` → `[]`, `brain doctor` → exit 0 and its own `*.test.brain` files.

`brain gen` does not exist yet (`brain gen --list --json` → `unknown command gen`). The manifests are written so
TOOLCHAIN (#80) can serve these folders as data: name, title, description, a params schema (with the literal each
param replaces), the file list, the scaffold files it replaces, the kit components it uses and notes for the agent.

## Recipes

| Recipe | Files | What it gives |
|---|---|---|
| `app-shell` | `app.brain`, `views/_layout.brain`, `comp/app-nav.brain` + test | static top bar and skip link in `app.brain`; sidebar navigation (collapses behind a Menu button, Escape closes) and the session line in `_layout` |
| `navbar` | `app.brain`, `views/_layout.brain`, `comp/navbar.brain` + test | top navigation: brand, links, responsive Menu button, Account disclosure list fed by the session store |
| `auth` | `server/auth.jsbrain`, `views/signup.brain` + test, `views/admin/_guard.jsbrain`, `views/admin/index.brain`, `comp/session-status.brain` + test | sign-up with `form()` (confirmation, server errors), a role guard (anonymous → login with `?next=`, missing role → 403 naming it), session status component |
| `crud` | `types.jsbrain`, `api/items.jsbrain`, `api/item.jsbrain`, `views/items/index.brain`, `views/items/[id].brain`, `comp/item-list.brain` + test, `comp/item-form.brain` + test | one entity from typed `api()` endpoints: list with search + status filter + inline create, detail with edit and delete, loading/error/empty |
| `dashboard` | `lib/dashboard-data.jsbrain`, `ui/kpi.brain` + test, `comp/data-table.brain` + test, `views/dashboard.brain` | period `<Tabs>`, KPI grid with accessible change direction, sortable table with `aria-sort` |
| `settings` | `views/account/settings.brain` + test | behind the account guard: `<Tabs>` Profile/Notifications in one `form()`, server save that validates again, saved message |
| `pricing` | `comp/pricing-table.brain` + test, `views/pricing.brain` | plans from one list, Monthly/Yearly (`aria-pressed`, discount), featured plan, choose → sign-up with plan and period, billing FAQ |
| `landing` | `views/home.brain`, `ui/hero.brain` + test, `ui/feature-grid.brain`, `ui/cta-band.brain`, `comp/faq.brain` + test | hero, features, FAQ accordion, call-to-action band; enough copy and internal links for the SEO audit |
| `wizard` | `ui/steps.brain`, `comp/onboarding-wizard.brain` + test, `views/onboarding.brain` | three steps with progress (`aria-current="step"`), per-step validation with focus on the first error, review, server save with busy and error states |
| `modal` | `comp/confirm-dialog.brain` + test, `comp/rename-dialog.brain` + test, `views/team.brain` | confirm and edit-in-a-dialog flows on the kit `<Dialog>` |
| `states` | `ui/loading-state.brain`, `ui/empty-state.brain` + test, `comp/order-list.brain` + test, `api/orders.jsbrain`, `views/orders.brain` | loading skeleton, empty state with an action, `{#await}` with `<ErrorState>` and Retry |

## Use

```bash
brain new my-app
cp -R bench/recipes/crud/{types.jsbrain,api,views,comp} my-app/    # or the paths listed in recipe.json "files"
cd my-app && brain check --json && brain doctor
```

Then rename (`params` in `recipe.json` lists what each parameter replaces) and link new views from the navigation.
Recipes are independent: two recipes that ship the same path (`app-shell` and `navbar` both replace `app.brain` and
`views/_layout.brain`; `auth` replaces `server/auth.jsbrain`) are alternatives, not layers.

## Verify

```bash
cargo build --release -p brain-cli
node bench/recipes/verify.mjs            # all recipes; --only crud,auth for some
```

For each recipe: a fresh `brain new` app in `~/tmp/brainllm-recipes/<recipe>`, the recipe copied over it, then
`brain check --json` must be `[]`, `brain test --json` must run the recipe's own tests and all must pass (no a11y,
error or leak findings), `brain doctor --json` must exit 0. Every file in the folder must be listed in `recipe.json`.
It also probes `brain gen --list --json` and, once it exists, requires each recipe to be listed. Exit 1 on any failure.
Result: `results/verify.json`.

## With vs without recipes

`bench/recipes/bench/`: `briefs-src.json` (six briefs, each mapped to a recipe, with the routes it must create),
`make-briefs.mjs` (writes `briefs-83.json` = plain brief and `briefs-83-on.json` = brief + the recipe's files inlined)
and `summarize.mjs` (reads the harness output and transcripts, writes `results/summary.{json,md}`).

```bash
cargo build -p brain-cli
node bench/recipes/bench/make-briefs.mjs
node bench/autonomy/run.mjs --model claude-haiku-5-5 --mode mcp --briefs ../recipes/bench/briefs-83.json --tag recipes-off-haiku
node bench/autonomy/run.mjs --model claude-haiku-5-5 --mode mcp --briefs ../recipes/bench/briefs-83-on.json --tag recipes-on-haiku
# same two with claude-sonnet-5-5 (tags recipes-off-sonnet, recipes-on-sonnet)
node bench/recipes/bench/summarize.mjs
```

Limitation: the harness cannot pre-seed files into the app, so the "on" variant hands the recipe to the agent inside
the brief and the agent writes it with `brain_write`. Those writes and the longer prompt count against "on"; with
`brain gen` (one tool call) the cost of "on" would be lower than measured here.

RESULTS_PLACEHOLDER
