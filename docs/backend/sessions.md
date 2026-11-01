# Session controls

What the native backend does for the sessions it owns. Everything here is off or unchanged until you set it.

| Setting | Effect |
|---|---|
| `BRAIN_AUTH_SESSION_SECONDS` | Length of new sessions (default unchanged) |
| `session_seconds: { admin: 3600 }` in `server/auth/roles.jsbrain` | Length of new sessions for a role; other roles use the setting above |
| `BRAIN_AUTH_IDLE_SECONDS` (300..31536000) | A session unused this long stops working, for the cookie, the access token and the refresh token |
| `reauth: 300` on a signed-in route | The route needs a password proof at most that many seconds old; otherwise 401 `REAUTH_REQUIRED` |

A role's length applies when the session starts: a role change affects the next login. The last use is written at most once
a minute, so an idle timeout is accurate to about a minute.

## An account's own sessions

| Route | Answer |
|---|---|
| `GET /_brain/auth/sessions` | `{ sessions: [{ id, current, created_at, last_used_at, auth_time, expires_at }] }`, live ones only |
| `POST /_brain/auth/sessions/{id}/revoke` | Ends another session. 404 for an unknown id and for another account's, alike; 409 `CURRENT_SESSION` for this one (use logout) |
| `POST /_brain/auth/sessions/revoke-others` | `{ revoked: n }`; keeps this session and ends the rest, refresh tokens included |
| `POST /_brain/auth/reauth` `{ password }` | Renews `auth_time` of this session, under the same attempt limits as login. A wrong password answers 401 `LOGIN_FAILED` |

They take a session credential; an API key answers 401. An account without a password (Google only) cannot pass `reauth`:
it signs in again, which starts a new session with a fresh `auth_time`. Requests are not recorded by device or address, so the
list shows times only.

## An app-owned session (BFF)

With `BRAIN_BACKEND_SESSION=app` the app issues its own session after the backend login, and everything above acts on the
backend session, not the app's. `brain doctor` warns B0799. To keep them together, store the backend's token with
`ctx.session.setSecret`, send it on each backend call, and end the app session when the backend answers 401:

```js
// server/account.jsbrain
<meta server>
export async function loadAccount(ctx) {
  const answer = await fetch(`${process.env.BRAIN_BACKEND_URL}/_brain/auth/me`, { headers: { authorization: `Bearer ${ctx.session.secret()}` } })
  if (answer.status === 401) { ctx.session.clear(); throw redirect('/login') }
  return answer.json()
}
```

The same check serves `REAUTH_REQUIRED`: a BFF sends the user to confirm the password rather than signing them out.
