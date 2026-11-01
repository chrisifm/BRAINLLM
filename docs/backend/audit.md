# Audit log

`audit: true` in a resource's `policy.jsbrain` records every insert, update and delete of its rows, whichever path wrote
them. A database trigger writes the entry inside the same transaction, and serving refuses a table whose trigger is missing.
Organization and membership changes are always recorded.

```js
<meta server>
export default { read: ['admin'], create: ['admin'], update: ['admin'], delete: ['admin'], audit: true }
```

An entry holds `id`, `at`, `actor`, `actor_kind` (`user`, `system_key`, or `server` for a write outside a request), `org`,
`resource`, `record`, `operation`, `fields` and `request`. `fields` lists changed field **names**. Values are never stored, so
a private or write-only field cannot leak through the log. An update that changes nothing is not recorded.

The table refuses `UPDATE`, `DELETE` and `TRUNCATE`, also from the application's own database role.

| Route | Who | Reads |
|---|---|---|
| `GET /_brain/orgs/{id}/audit` | that organization's managers | Its entries |
| `GET /_brain/audit` | global `admin` | Entries outside any organization |

Both accept `resource`, `before` (an entry id, from `next`) and `limit` (1 to 100), newest first; any other parameter is 422.
Retention is yours to decide: nothing expires entries.
