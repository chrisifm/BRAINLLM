# guide: forms — validation, submit, server errors, edit forms

## When
Any input the user submits: sign in, sign up, search, create or edit a record, settings.

## Steps
1. Declare the schema once: `{ email: 'email!', age: 'num>=18', name: 'str!<=80', tags: '[str]' }`.
   `!` required · types `str num bool email url date` · `[T]` list · a nested type object · bounds `>=`, `<=`.
2. `let f = form(schema, { submit: async (v) => …, init: { email: '' } })`. `v` is already validated.
3. Markup: `<form on:submit={f.submit}>` (prevents default), fields `bind:value={f.v.email}`, the message
   `{f.err.email}` (or the kit's `error={f.err.email}`), the form-level error `{f.error}`, the busy state `f.busy` on the submit button.
4. Labels: kit `<Input label="Email" bind:value={f.v.email}>` / `<Select label="…">` / `<Checkbox>`, or a raw control
   inside a `<label>` (B0403 otherwise).
5. Server side: a `server:` function in the same `.brain` validates `data` again (never trust the client):
   `server: async function signup(ctx, v) { … }`; the client calls `await signup(v)` (no `ctx`).
6. Field errors from the server: `throw { errors: { email: 'Taken' } }` (or return it) → 422, the message reaches
   `f.err.email`. Later: `f.errors({ email: 'Taken' })`. Any other failure shows in `f.error`.
   A plain `throw new Error('msg')` answers 500 and the client sees a generic message (the real one is in the log).
7. After success: a server function may `return { redirect: '/account' }` (without JS the form post answers 303 there);
   with JS, `navigate(r.redirect)` in `submit`.
8. Edit forms: values that arrive later → `f.reset(values)`. Never replace `f.v` or `f.err` (B0133).
9. Translated messages: add `form.required`, `form.min_chars`, `form.max_chars`, `form.min_items`, `form.max_items`,
   `form.item`, `form.list`, `form.number`, `form.min`, `form.max`, `form.email`, `form.url`, `form.date`,
   `form.error` and `form.check_failed` to `i18n/<locale>.json` (`{n}` = the limit; in `form.item` `{n}` = the 1-based
   item number and `{error}` = that item's message; plural objects allowed). English otherwise.
10. Expose the form to browser agents with `<form tool="…" tool-description="…">` (`brain guide webmcp`).

## Verify
```bash
brain check --json      # []
brain test              # an empty submit shows the errors without calling the server
brain shot /signup --text   # every field has its label in the outline
```

## Common mistakes
| Mistake | Code | Fix |
|---|---|---|
| `f.v = {…}` or `f.err = {}` | B0133 | `f.reset(values)` / `f.errors({…})` |
| input without a label | B0403 | `<Input label="…">` or `<label>Text <input …></label>` |
| `<div on:click>` as a submit | B0402 | a `<button type="submit">` inside the `<form>` |
| validating only on the client | — | re-validate `data` inside the `server:` function |
| `throw new Error('Email taken')` for a field | — | `throw { errors: { email: 'Email taken' } }` |
| password or token stored in the session | — | `ctx.session.set({ id, email })`: ids and display data only |
| `bind:value` on a plain `const` | B0204 | bind to a `sig` or to `f.v.<field>` |

## Minimal correct example
```html
<meta seo title="Join" description="Create your account with an email address and a display name.">
<script>
  import users from '../server/users.jsbrain'
  server: async function join(ctx, v) {
    if (await users.exists(v.email)) throw { errors: { email: 'This email is already registered' } }
    const user = await users.create(v)
    ctx.session.set({ id: user.id, email: user.email })
    return { redirect: '/account' }
  }
  let f = form({ email: 'email!', name: 'str!<=80' }, {
    submit: async (v) => navigate((await join(v)).redirect),
  })
</script>
<main class="page">
  <h1>Join</h1>
  <form on:submit={f.submit}>
    <Input label="Email" type="email" bind:value={f.v.email} error={f.err.email} />
    <Input label="Display name" bind:value={f.v.name} error={f.err.name} />
    {#if f.error}<p role="alert">{f.error}</p>{/if}
    <Button type="submit" busy={f.busy}>Create account</Button>
  </form>
</main>
```
`server/users.jsbrain` starts with `<meta server>`, so it never reaches the browser.
