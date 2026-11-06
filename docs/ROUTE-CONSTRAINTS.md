# Frontend route parameter constraints

Opt in to a bounded raw-segment constraint in `brain.json`. The key is the exact route printed by `brain map`, not a filename. `views/[code].brain` remains the filename:

```json
{"routeParams":{"/:code":{"code":"[2-9a-z]{7}"}}}
```

The route is a candidate only when each declared parameter matches its whole original encoded segment. `/3abcdef` matches; `/zzz`, `/0abcdef` and `/%33abcdef` do not. No decoding is added to `ctx.params`. A mismatch skips that route before its guard/middleware/load/form handler and continues through the existing route order to a catch-all or 404. Static routes and public-file/security precedence remain in effect. Catch-all parameters cannot themselves receive a constraint. Parameters before a catch-all may be constrained.

Pattern syntax is deliberately limited: ASCII literals/classes (`[a-z0-9]`), fixed `{n}` or bounded `{min,max}` repetition. A literal regex metacharacter is escaped (`\\.` in a JSON string). No groups, alternation, negated classes, shortcuts such as `\\d`, backreferences, lookaround, anchors, open counts or `* + ?` quantifiers. Variable counts must have a disjoint following character set, including nullable following atoms; `[a-z]{1,8}-[0-9]{2,4}` works and `[a-z]{1,8}a` fails. Limits: 256 pattern bytes, 32 atoms, at least one required character and at most 256 input characters. No route priority changes are inferred from a pattern.

Unknown routes/parameter names, duplicate keys and invalid patterns fail with B1880 when explicitly opted in. Existing routes without this key keep their behavior. `paths()` values are checked after URL encoding and before any generated page load; invalid values stop the build. Live sitemap functions omit entries that do not match their actual route. Route constraints are frontend view contracts; native backend route APIs are unchanged.

## Redirect predicates

`redirects.json` uses the same grammar in a rule's optional `params` object:

```json
[{"from":"/old/:code","params":{"code":"[2-9a-z]{7}"},"to":"/:code","status":308}]
```

A nonmatching source does not redirect. Development and SSR share exact-before-pattern ordering, base-prefix handling, query retention/override and slash/canonical-host policy. Existing literal redirects and unconstrained parameter rules retain their syntax.

Constrained redirects require the BRAINLLM server evaluator. An automatic build emits that runtime even when every page is static. Convenience `_redirects`, nginx and Caddy artifacts are omitted as whole files when they cannot preserve the raw-segment contract, with an explicit warning and `_brain/redirect-capabilities.json` describing unsupported formats. No rule is silently widened or partially dropped. SSR retains all rules. A static export with constrained redirects fails before replacing `dist/`; finite constrained routes from valid `paths()` remain supported. Explicit host export is an actionable capability error; nginx/Caddy URI equivalence is not claimed merely because a regex can be emitted.


Route constraints validate frontend route shape; they do not authenticate a visitor or authorize a backend operation. Keep guards and backend policies for access control.

When this opt-in is present, production also refuses origin-form aliases containing literal or encoded dot segments, backslashes or fragments before URL parsing or canonical redirects can normalize them into a constrained route. Unconfigured applications preserve their legacy dispatch. Canonical native API paths are unchanged.
