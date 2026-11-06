# Observe and plan edge caching

`brain edge plan --json` reads the current build and produces a read-only cache plan. Exact immutable
asset URLs come from emitted host-header evidence, including custom static folders. A missing inventory
does not authorize a wildcard immutable rule.

`brain edge check --url https://example.com --json` (alias `audit`) makes bounded anonymous HTTP
observations against the local build's mounted base. It compares declared and delivered cache TTL,
Content-Type and `Vary: Accept`, including HTML/Markdown representations and immutable assets. It flags
private or Set-Cookie responses observed in shared cache. Two `MISS`/`DYNAMIC` responses are an
inconclusive cache observation, not proof that the provider must produce a HIT.

Coverage is explicit: at most eight concrete public routes, four exact immutable assets, two requests
per representation, three seconds per request and thirty seconds total. Redirects are not followed;
Cookie/Authorization and response bodies are not captured. Parameterized or authenticated routes,
body correctness, CSP and a provider's complete cache key are outside this header audit. A mismatch
or unavailable observation exits `2` with one complete JSON report. `brain release-check --url ...`
includes this check alongside the existing deploy and SEO checks.

## Explicit Cloudflare operations

Configure private `CLOUDFLARE_ZONE_ID` and `CLOUDFLARE_API_TOKEN` in the selected app environment.
The selected HTTPS host must belong to that exact zone. The token is sent only to the official fixed
HTTPS API, without redirects or environment proxies. No account discovery occurs.

```sh
brain edge plan --provider cloudflare --url https://example.com --json > edge-request.json
brain edge audit --provider cloudflare --url https://example.com --json
```

Review the plan and its `applyRequest` before explicitly applying it:

```sh
brain edge apply --provider cloudflare --url https://example.com --request edge-request.json --json
```

The request binds the zone, HTTPS origin, mounted base, build plan, current ruleset revision and exact
desired rules. Stale or modified requests fail closed. Apply supports only its own scoped Cache Rules
references: exact immutable anonymous GET/HEAD assets may respect origin caching; all other requests
in this host/base scope bypass caching. HTML remains bypassed, including cache-declared pages, because
this module does not establish that a provider cache key varies correctly by Accept.

At most 128 safe exact asset paths and a 4,000-character expression are supported. No Cookie or
Authorization request, mutation, API, `/_r`, `/_brain`, `/_server`, `/s`, health probe or service worker
becomes cache eligible through these rules. An unsupported inventory requires a manually reviewed
policy. Provider permissions and plan capabilities can refuse the proposed subset.

Apply uses granular POST/PATCH requests for its own references, installs bypass first, rereads before
each write and checks readback, ordering and foreign rules afterwards. It never rewrites the shared
entrypoint or modifies/deletes foreign rules. A missing phase requires manual setup; it does not
create an entrypoint, purge caches, change Workers/global settings or silently activate during deployment.

Concurrency is optimistic: the documented API does not provide an atomic compare-and-swap precondition.
A same-rule edit after the last read may race. Stale state, changed foreign state, partial or unknown
outcomes stop further writes and require a fresh audit/plan. There is no automatic retry or rollback
after an uncertain mutation. Other phases, Workers, existing cached objects and later account changes
remain outside this policy proof.

Use a token restricted to this zone with Zone Read and Cache Settings Read for observation. Add Cache
Settings Write only when explicitly applying a reviewed plan. Tests use an offline provider transport;
local success does not establish the availability or configuration of a real account.
