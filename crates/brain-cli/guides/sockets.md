# guide: sockets — live updates with liveResource

`liveResource` refetches a resource when the server says its rows changed. The socket carries only "this changed"; the
HTTP read stays the source of the data and of its permissions.

## When

Several people work on the same rows and a change must appear without a reload. A dashboard that only needs fresh numbers
is simpler with `resource(..., { every: '30s' })`.

## Steps

1. Turn it on in `brain.json`: `backend: { origins: ['https://app.example.com'], sockets: { allowedOrigins: ['https://app.example.com'] } }`.
2. Pass an existing resource and its rooms, with a public key of the signed-in user as the scope:

   ```js
   const rows = resource(() => api.get('/posts'))
   const updates = liveResource(rows, { rooms: [{ resource: 'posts' }], scope: () => session.user ? session.user.id : null })
   ```

3. Show `updates.state` where the user needs to know. `{room: 'user:self'}` follows only the signed-in owner.
4. Run `brain check`: B0744 and B0745 report the origins, B0775 a scope that is always null.

## Who can connect

Only a signed-in user. There is no anonymous socket: the upgrade authenticates, and every event checks the row against
that user again. A public page refreshes with `resource(() => api.get('/posts'), { every: '30s' })`, which pauses while
the tab is hidden or offline.

## States

| `updates.state` | Meaning | What to do |
|---|---|---|
| `idle` | no rooms, or rendered on the server | nothing |
| `anonymous` | rooms, but the scope is null: nobody is signed in | offer sign-in, or use `every` for a public page |
| `connecting` · `retrying` | on the way; retries are bounded | show the last data |
| `open` | joined; `updates.connected` is true | nothing |
| `auth-required` | the session ended, here or in another tab | ask to sign in, then `updates.reconnect()` |
| `forbidden` | every room was refused for this user | check the room and the row condition |
| `exhausted` | the retry budget ran out | offer a retry with `updates.reconnect()` |
| `policy-closed` · `protocol-error` · `invalid-config` | the server or the build refused this client | read the server log |
| `refresh-pending` · `refresh-timeout` · `refresh-error` | the read after an event is late or failed | the resource's `{:catch}` shows the error |
| `unsupported` · `closed` | no WebSocket in this browser · closed on purpose | nothing |

## Session changes

A session closed in another tab ends the socket within about 30 seconds, because the server checks the user on every
heartbeat. A later connection that the server refuses looks like a network failure to the browser, so the runtime reads
the resource once per outage: a 401 becomes `auth-required`, and any other answer keeps the retries.

HttpOnly cookies are invisible to the page. Call `updates.reconnect()` after a same-user sign-in or session rotation, and
clear the scope on sign-out.

## Verify

`brain check` reports B0744, B0745 and B0775. Sign in, open the page in two tabs and change a row in one: the other
refetches and `updates.state` is `open`. Sign out in one tab: within about 30 seconds the other says `auth-required`.

## Common mistakes

- A token or an email as the scope: it is a public key that only tells the runtime when the user changed.
- `scope: () => null` on a public page (B0775): nobody can connect; use `every`.
- Treating an event as data: it says only that a row changed; the refetch brings the row the user may read.
- Leaving `auth-required` unhandled: the list stays on screen with old rows and no hint.

## Minimal correct example

```brain
<script>
  const rows = resource(() => api.get('/posts'))
  const updates = liveResource(rows, { rooms: [{ resource: 'posts' }], scope: () => session.user ? session.user.id : null })
</script>

{#if updates.state === 'auth-required'}
  <p role="alert">Your session ended. <a href="/login">Sign in again</a></p>
{/if}
```
