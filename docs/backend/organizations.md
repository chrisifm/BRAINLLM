# Organizations and memberships

Tenant data belongs to an organization and is enforced by the server. A resource becomes tenant data by declaring a
field of kind `org`:

```js
// server/resources/projects/schema.jsbrain
<meta server>
export default { org: 'org!', title: 'str!' }
```

```js
// server/resources/projects/policy.jsbrain
<meta server>
export default { read: ['org'], create: ['org'], update: ['org.admin', 'org.owner', 'org.own'], delete: ['org.owner'] }
```

| Grant | Who |
|---|---|
| `org` | any member of the row's organization |
| `org.<role>` | members holding exactly that role |
| `org.own` | a member, on rows they created |

Global roles, `owner` and `owner.<ref>` are refused on tenant resources: a global administrator has no reach into an
organization. A `public` read needs a row condition or `expose_all_rows`. Unique fields are unique per organization,
and a row can only reference rows of its own organization, enforced by a database trigger. The organization of a row never changes.

Membership roles default to `owner`, `admin` and `member`, most senior first. To rename them:

```js
// server/auth/organizations.jsbrain
<meta server>
export default { roles: ['owner', 'admin', 'member'], managers: ['owner', 'admin'] }
```

## Endpoints

| Route | Who | Does |
|---|---|---|
| `POST /_brain/orgs` | signed in | Creates an organization with the caller as its first role |
| `GET /_brain/orgs` | signed in | Lists the caller's organizations |
| `GET /_brain/orgs/{id}/members` | member | Lists members |
| `PATCH`, `DELETE /_brain/orgs/{id}/members/{user}` | manager | Changes a role or removes a member; never below one owner (409 `ORG_LAST_OWNER`) |
| `POST /_brain/orgs/{id}/invites` | manager | Returns a single-use token, once |
| `POST /_brain/orgs/invites/accept` | signed in | Joins with a token (`ORG_INVITE_INVALID` otherwise) |

A non-member receives the same 404 as for an organization that does not exist.

## Not available yet

Deleting an organization. Actions, transitions, batches and operations on tenant resources: the contract refuses them,
so tenant writes go through create, update, patch and delete. `visible` grants by organization role.
