# Importing existing data (B-28, B-39)

`brain backend import` brings accounts and records from another system with their ids, owners, references and dates.
It is for a reviewed cutover, not for daily sync.

```sh
brain backend import users --file users.jsonl           # preview: runs every insert and rolls back
brain backend import users --file users.jsonl --apply
brain backend import shops --file shops.jsonl --apply
brain backend import products --file products.jsonl --apply
```

One file is one transaction: a single bad row leaves everything as it was (B0798). The report gives each problem's line
and never a value from the file. Run `brain backend migrate` first, then import accounts, then resources in dependency
order, because owners and references are real foreign keys.

## Accounts

One JSON object per line:

```json
{"id":"…uuid…","email":"a@example.test","role":"user","verified":true,"created_at":"2024-03-01T10:00:00Z","password_hash":"$scrypt$ln=15,r=8,p=1$c2FsdA$…"}
```

`id`, `email` and `role` are required, and the role must be declared in `server/auth/roles.jsbrain`. `password_hash` takes:

- `$scrypt$ln=<log2 N>,r=<r>,p=<p>$<salt>$<hash>` with unpadded base64. Bounds: `ln` up to 20, `r` up to 32, `p` up to 8,
  1 GiB of memory. If your system stores `salt:hash`, convert it: the salt in this string is the base64 of the exact bytes
  your system passed to scrypt.
- An Argon2 PHC string, verified with the parameters it carries.

Anything else is refused. Without a hash the account has no usable password: recovery or a linked provider signs it in.
The first successful login with a scrypt hash stores an Argon2 one, without ending sessions or changing the password,
even when the old password is shorter than the signup minimum.

## Records

```json
{"id":"…uuid…","owner":"…user uuid…","data":{"title":"Rake","shop":"…shop uuid…"},"created_at":"2023-01-02T03:04:05Z"}
```

`id`, `owner` and `data` are required. `data` passes the same field validation as a create. `updated_at` defaults to
`created_at`. Declared constraints and uniqueness still hold, so a duplicate refuses the file. State machines and
before-hooks do not run and no webhook, socket event or after-hook fires: history is being restored, nothing has just
happened. An old numeric id belongs in a declared field, see [ids](ids.md).
