# Native type evidence without TypeScript

BRAINLLM checks proven data shapes in its own AST. B2001–B2005 are warnings in
0.4.0; security diagnostics retain their existing severities. Runtime validation
and backend authorization remain authoritative. Unknown values stay unknown.

| Code | Proven condition | Mechanical correction |
|---|---|---|
| B2001 | Value conflicts with a component prop | A canonical finite numeric text literal such as `count="3"` becomes `count={3}`; other conversions require a decision |
| B2002 | Named field absent from a closed, declared or native resource shape | Replace one uniquely closest field within two edits; ambiguous names have no edit |
| B2003 | Literal API body has an unknown field, missing required field or incompatible known value | A unique field typo can be replaced if it cannot create a duplicate key; missing data is never invented |
| B2004 | Form initializer or checked/value binding conflicts with its schema | Explain the control/schema correction; no automatic control conversion |
| B2005 | Proven null/undefined value is dereferenced without a sufficient guard | Explain a nullish guard or optional chaining; no automatic insertion that could change behavior |

## One declared shape through fetching and rendering

```html
<script>
  import usersApi from '../api/users.jsbrain'
  let users = resource(usersApi.get)
  let current = drv(() => users)
</script>
{#await current}
  <p>Loading</p>
{:then rows}
  {#each rows as user (user.id)}
    <p>{user.name}</p>
  {:empty}
    <p>No users</p>
  {/each}
{:catch error}
  <p>Unable to load users</p>
{/await}
```

A literal API declaration with `get: { out: '[User]' }` derives its fields from
`types.jsbrain`. Inference follows the API method, `resource`, simple `sig`/`drv`
values, await results and each bindings. An async function can obtain the same
shape through `await usersApi.get()`. Default exports of directly imported
`.jsbrain` API modules are inspected without executing the module. Root type
imports and aliases are accepted only when their actual file identity is proven.

For a literal native `/api/posts` resource declared under
`server/resources/posts/schema.jsbrain`, an API method with no explicit `out`
can derive the actual backend envelope: list `{items,limit,offset}`, or one row
`{id,owner,data}` for the detail endpoint. `row.data` uses the backend field
contract. No method named `get` acquires fields without a real declaration.
Explicit `out` schemas continue to describe the generic API response; they do
not silently flatten native row envelopes.

Generic API numeric validation accepts numbers and numeric strings without
converting JSON. Inference preserves that representation uncertainty: it does
not claim a guaranteed JavaScript Number. Native Rust resource numeric fields
are JSON numbers. Form values remain unknown while controls are edited; their
schema still supplies field names and boolean/value binding checks.

## Nullish guards and lexical scopes

Optional schema scalars may be null or absent. Required scalar rules remove
that uncertainty. A literal `condition ? null : {name:'Ada'}` carries its known
null branch instead of losing its object shape.

The checker narrows truthy branches, nullish equality checks, short-circuit
conditions, conditional expressions and simple early-return guards. Each,
await/catch, function and slot bindings have scoped environments. Branch facts
are restored before siblings. `value !== null` does not prove that an optional
field is defined; `value != null` excludes both null and undefined.

Continuous optional chains retain their short-circuit behavior;
`user?.name.length` differs from `(user?.name).length`. A nullable `name` field
still needs its own check even when the `user` receiver is protected.

## Conservative boundaries and resource bounds

Named shared schema extraction currently accepts flat scalar/list rule objects;
nested schema objects remain unknown. Inference is deliberately partial.
Assignments, aliases, escapes to unknown calls and conflicting bindings can
invalidate evidence. Computed field names, spreads, unknown functions, complex
return joins and unsupported schemas are not assigned guessed shapes. Primitive
names shadowed by application bindings are not framework calls. This is an
evidence pass, not whole-program alias analysis or a replacement for runtime checks.

Context loading reads only directly imported modules and schemas for referenced
literal resource paths. It observes files and absent schema paths/directories
for normal watcher invalidation. Reads reject symlinks and paths outside the
project, cap a file at 256 KiB and the additional context scan at 8 MiB, inspect
at most 128 direct statements and 128 resource paths, and retain bounded shapes.
Inference stops after 32 value levels, considers at most 512 top-level
declarations and emits at most 128 findings. Suggestions cap names at 80 bytes.

Focused regression commands:

```sh
cargo test -p brain-check --test native_inference --test native_types -j 2
```

These synthetic fixtures do not invoke an LLM or provider. The fresh app-author
LLM gate remains required before claiming measured authoring improvements.
