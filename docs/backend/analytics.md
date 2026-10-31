# Behavior analytics (experimental backend)

Every app with a native backend records how visitors use it: which elements they click, where they are and how long
they stay. Events land in the project's own Postgres (`_brain_analytics_*` tables, created by `brain backend migrate`).
Nothing is sent to a third party, and the compiled build output does not change: route budgets stay the same.

## How it turns on

Analytics is on by default once a native backend is attached:

1. `brain backend serve` mounts `POST /_brain/analytics` (ingestion) and `GET /_brain/analytics.js` (the collector).
2. The frontend server (`brain run`, the production server and the Node runtime server) inserts one tag before
   `</head>` on HTML pages: `<script type="module" async src="/_brain/analytics.js"></script>`. It is added when the
   page is served, not compiled into the build, so it costs zero bytes in the route budget.
3. The collector batches events and sends them with `sendBeacon` (fallback: `fetch` with `keepalive`).

Without `DATABASE_URL` or `BRAIN_BACKEND_URL`, `brain doctor` reports info `B0729` with the steps to enable it.

Switch it off:

- per frontend server process: `BRAIN_ANALYTICS=off` (no tag is injected, so nothing is collected);
- per project: `"analytics": false` in `brain.json`.

## brain.json

```json
{
  "analytics": {
    "enabled": true,
    "consent": "required",
    "retentionDays": 90,
    "countryHeader": "cf-ipcountry",
    "eventsPerMinute": 600
  }
}
```

| Key | Default | Meaning |
|---|---|---|
| `enabled` | `true` | `false` turns analytics off (same as `"analytics": false`). |
| `consent` | `"none"` | `"required"`: the collector records nothing until the visitor grants consent. |
| `retentionDays` | `180` | Events older than this are deleted by a background loop (1–3650). |
| `countryHeader` | none | Lowercase header set by your proxy with an ISO country code. |
| `eventsPerMinute` | `600` | Per-visitor ingestion rate limit (1–6000). |

Unknown keys are errors. `crates/brain-cli/schema/brain.schema.json` describes the same shape.

## Privacy

- **No PII by design.** The collector never sends field values, typed text or clipboard contents. For inputs it sends
  only counts and durations. Password, card and other sensitive fields are reported as type `secret`.
- **No IP address is stored.** The country comes only from `countryHeader`, if set.
- **Anonymous ids.** The visitor id is a random UUID in `localStorage` (`brain:vid`). A session (`brain:sid`) ends after
  30 idle minutes. If the visitor signs in, the session is linked to the user id the backend already knows.
- **Signals honored.** Global Privacy Control, Do Not Track and `navigator.webdriver` disable the collector. The server
  also drops known bot user agents.
- **Consent mode.** With `consent: "required"`, the collector records nothing until the app calls
  `consent('analytics')`, the same runtime helper that unlocks Google tags. `consent('analytics', false)` stops it
  again. The choice is stored in `localStorage` under `brain:consent`, and a visitor who has already consented is
  recorded on the next page load.

- **Retention and erasure.** Events past `retentionDays` are deleted automatically. `brain analytics purge` and
  `brain analytics erase --user <uuid>` delete data on demand.
- **Validation.** The server validates every batch: at most 64 events, a 32 KB body, bounded strings and routes, and
  parameterized SQL only.

## What is recorded

| Kind | When |
|---|---|
| `page`, `leave` | A route is shown, or left. `leave` carries visible dwell time and maximum scroll depth. |
| `click`, `key` | A click, or Enter/Space on a non-text control. |
| `rage`, `dead` | Three clicks on one element within 1 s, or a click that changes nothing within 1 s. |
| `focus`, `input`, `change`, `toggle`, `submit` | Form use: time focused, edit counts, toggles, sent or invalid submits. |
| `copy`, `cut`, `paste`, `drag`, `drop`, `context`, `hover`, `media` | Other interactions, including 500 ms hover intent and media play/pause/seek/end. |
| `track` | A custom event. |

The first page view also records the referrer host, UTM parameters, language and a coarse viewport width.

## Custom events

Declarative:

```html
<button data-track="add_to_cart">Add to cart</button>
```

Imperative:

```js
dispatchEvent(new CustomEvent('brain:track', { detail: { name: 'checkout_started', props: { items: 3 } } }))
```

## Element ids cost zero bytes

The compiler adds nothing for analytics. Every component root already carries `data-b`, the component hash. The
collector describes each element by that hash plus its tag, its position among same-tag elements, its accessible name,
`data-track`, role, and field or link target.

`brain build` writes `.brain/analytics/elements.json`, a map from those descriptors to `file:line`. You can also
generate it with `brain analytics map`. The report resolves each recorded element back to its source line.

## Usage report

```sh
brain analytics report --since 7d           # per route and per element, resolved to file:line
brain analytics report --since 30d --json   # same, as JSON (includes neverUsed and notes)
brain analytics funnel /shop /cart /checkout --since 14d
brain analytics segment --viewed /pricing --min-views 2 --not-viewed /signup --within 7
brain analytics export --since 30d --csv > events.csv
brain analytics map                         # rewrite .brain/analytics/elements.json
brain analytics purge --days 180
brain analytics erase --user 0b7c…          # delete one account's analytics
```

Every command accepts `--env <name>` (default `development`). The report includes:

- visitors, sessions, views and signed-in users;
- per route: dwell, scroll depth, time to first interaction and exits;
- per element: interactions, visitors, rage and dead clicks, with `source` `file:line`;
- interactive elements that nobody used (`neverUsed`);
- entry routes, referrers, UTM campaigns, devices, countries and custom events.

AI agents can call the same report through the MCP tool `brain_analytics_report` (arguments `since`, `env`).

### Example: a shop

```sh
brain analytics funnel '/products/*' /cart /checkout --since 30d
```

```json
{"schemaVersion":1,"steps":[{"route":"/products/*","visitors":1240},{"route":"/cart","visitors":310},{"route":"/checkout","visitors":96}]}
```

Each step counts visitors who reached it after the previous step (2–8 steps; `/prefix/*` matches a section).
Rage clicks on `components/cart.brain:14` (a disabled "Apply coupon" button) or a high `neverUsed` count on a filter
panel show which UI to fix or remove.

### Example: a blog

```sh
brain analytics report --since 7d --json | jq '.routes[] | select(.route|startswith("/posts/")) | {route, medianDwellMs, avgScrollPct}'
brain analytics segment --viewed '/posts/*' --min-views 3 --not-viewed /newsletter
```

The first command lists reading time and scroll depth per post. The second finds frequent readers who have never
opened the newsletter page.

## Segments from Rust

`brain_backend::analytics::segment(&pool, &Segment { viewed: "/pricing".into(), min_views: Some(2), .. })` returns
the same members as the CLI: visitor id, user id when linked, number of views and last visit. You can use it in
first-party personalization or notifications. Segments are for internal use only; they never leave the project's
database.

## Budget

The collector loads asynchronously from the backend, about 4.8 KB gzip, after the page renders. It is not part of
any compiled route, so the default 30 KiB route budget is unaffected. As measured, `/admin/posts/:id/edit` was the
same size before and after.
