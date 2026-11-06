# Explicit Google event bridge

Google Analytics or Tag Manager is optional. Legacy `google.analytics`/`google.tagManager` keeps its
existing behavior. Opt in to the consent-gated bridge with `google.mode: "basic"`:

```json
{
  "google": {
    "mode": "basic",
    "environments": {
      "production": { "analytics": "G-EXAMPLE123" },
      "development": {}
    },
    "events": { "save": "save_item", "submit": "generate_lead" },
    "funnels": { "acquisition": ["save_item", "generate_lead"] },
    "banner": true
  }
}
```

Choose one GA4 `analytics` ID or one GTM `tagManager` ID per destination. Both together are rejected
to avoid accidental duplicate delivery. Basic-mode objects reject duplicate keys rather than selecting
one of two ambiguous destinations. An explicit environments map is authoritative: a missing or
empty environment has no Google destination and never inherits production or top-level IDs. IDs are
public configuration, while service-account credentials and API tokens never belong in this object.

Only declared `events` mappings cross the bridge. `<button data-track="save">` maps a click to
`save_item`; a form's `data-track` maps its submit attempt. A submit event does not establish that the
server accepted a conversion. Code may dispatch `brain:track` with its declared name; extra event
properties, form values, DOM labels, user IDs, query strings and full page URLs are discarded.
`data-private` ancestors suppress DOM tracking. The first-party collector remains a separate feature.

The generated module is served from the same origin in static and SSR builds. In basic mode it loads
no external Google script and queues no Google events until analytics consent is literally `true`.
Use the normal `consent('analytics', true|false)` API or the optional accessible `banner`. Denial,
revocation, DNT and Global Privacy Control stop bridge delivery. Storage failure is handled with
page-local consent state; no remembered consent means denied. Already executed third-party code
cannot be unloaded by removing a script element. Review the selected Google product's consent
configuration and any other scripts your app includes.

GA4 uses `send_page_view: false`, disabled advertising signals and redacted title/referrer. Its page
location is the origin root, without paths or queries. Review and disable Enhanced Measurement
features that would collect additional browser data. A GTM container may execute arbitrary tags:
configure consent checks and triggers inside that container. The framework does not certify the
privacy behavior of externally configured tags, create a legal consent policy or verify delivery to
a real Google property during a local build.

Funnels describe 2..20 declared event names. The matching event carries the first matching funnel
name and step; the bridge never fabricates a conversion or sends an extra funnel event. Configure
remote reports yourself after reviewing the event schema.

Run `brain google setup --env production --json` for a read-only configuration and review plan.
It creates no Google resources, writes no files, installs no SDK and accepts no `--apply` shortcut.
The generated CSP permits the official Google loader/collection origins only when this environment
has a destination. Inspect the final asset ledger for the bridge's compressed cost; apps without
this opt-in do not receive the Google runtime module.
