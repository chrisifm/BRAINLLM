# Official SDK integrations

The integration catalog discovers maintained official SDKs and prepares selective revision-checked plans. `integrations::catalog()` returns a versioned embedded manifest; `preview(root, id, options)` returns proposed package/source operations. Neither operation installs packages, loads credentials, contacts an AI provider or deploys anything.

Supported catalog IDs are openai, claude, gemini, maps and storage. Existing Google Sign-In and optional Google Cloud Storage capabilities are identified rather than replaced. The checked-in manifest records exact versions, npm integrity, repository, license, Node engines, official documentation and verification time. Versions were fetched from npm's latest manifests on 2026-10-30; a catalog snapshot is not a claim that no newer release exists later.

| Integration | Official package | Verified snapshot |
| --- | --- | --- |
| OpenAI | openai | 7.45.1 |
| Claude | @anthropic-ai/sdk | 1.3.0 |
| Gemini | @google/genai | 2.32.0 |
| Maps | @googlemaps/js-api-loader | 2.1.3 |
| Cloud Storage | @google-cloud/storage | 8.4.0 |

Package provenance is recorded directly in the [embedded manifest](../crates/brain-cli/src/templates/integrations/catalog.json), including registry URLs and integrity values. No SDK implementation is copied into the framework.

## Preview and selective updates

Options accept an exact stable `version`, optional package `expectedRevision`, and `includeTemplate` (default true). An omitted version selects the verified snapshot. A different explicit version is marked unverified and has no attributed integrity. Tags and ranges are refused. Existing unrelated package fields/dependencies and peer requirements survive; ambiguous duplicate installable declarations require explicit resolution. Existing integration source is preserved.

Operations carry `expectedRevision` for the guarded change planner. Review and apply the plan, explicitly install the pin with the app package manager, review the lockfile and production dependency availability, then run the relevant checks. The preview does not select or execute a package manager and does not silently update all integrations. Node engine requirements belong to the actual official package.

Adapter contract version 1 describes the framework-owned glue and configuration. Update one SDK at a time using its current official documentation and release notes; review its engines, exports and API shape, update catalog provenance, and rerun adapter tests. Official SDK property names, side effects, export contracts, request/response keys and authentication behavior are external contracts. Compiler optimization may reduce proven unused ESM and owned glue; it must not blanket-mangle SDK APIs or assume every dependency is side-effect-free. CJS/dynamic behavior needs conservative dependency analysis.

## Server-only AI examples

Generated helpers live in `server/integrations/*.jsbrain` with `<meta server>` and direct official bare imports. Node resolves those imports from the deployed app's node_modules; they are not bundled into browser assets. The app must install production dependencies. The helper is imported by an actual `server:` function in a view/component; exporting a server module helper alone does not create a public RPC endpoint.

The example `ask(ctx, input)` checks `ctx.user`, requires a server-only credential and explicit model environment name, bounds input/output and limits simultaneous local calls to eight. Use a normal Brain server function so the existing per-IP/function rate limiter also applies. Capacity is per process, not a distributed tenant quota. Add app authorization/billing/quota rules appropriate to the selected model and workload.

OpenAI uses the official [Responses API](https://developers.openai.com/api/reference/typescript/resources/responses/methods/create) and SDK output_text. Claude uses the official [Messages API](https://platform.claude.com/docs/en/api/messages/create) and selects text blocks. Gemini uses the recommended [Google GenAI SDK](https://ai.google.dev/gemini-api/docs/libraries) and models.generateContent. Model names are required configuration, not hardcoded defaults. These are deliberately small text examples; apps can use other official SDK methods directly without a framework wrapper replacing the SDK contract.

The templates request an eight second SDK HTTP timeout, bounded output tokens and no automatic retries (one attempt for Gemini). Provider-side work or charges may continue after client cancellation; timeout settings are not proof of remote cancellation. SDK errors are replaced with a generic safe error; prompts, raw responses and credentials are never logged by the templates. Server traces can correlate the safe failure location. Live account/model access, provider billing and real network failures are not exercised by offline tests.

## Lazy Maps and native storage/auth

The Maps example imports only official setOptions/importLibrary exports and starts loading on `loadMap`, following the [official loader](https://developers.google.com/maps/documentation/javascript/load-maps-js-api). Browser package imports require the compiler's pinned ESM dependency pipeline; server bare-import support alone does not resolve them. Keep this qualification until that pipeline is verified on the application build.

Maps keys are public browser configuration: restrict them by HTTP referrers and allowed APIs. Never reuse server AI credentials. The SDK receives a quarterly Maps API channel independently of the pinned npm loader version. Set a nonce on trusted page scripts when required; follow Google's [Maps CSP guide](https://developers.google.com/maps/documentation/javascript/content-security-policy) for the precise services enabled by the app. Do not loosen all CSP directives or auto-load unused libraries. The template does not fetch maps on import.

Google Sign-In uses the existing auth integration and server verification. Cloud Storage uses the optional native storage adapter or the official SDK with ADC and explicitly scoped runtime setup. Catalog discovery never creates buckets, publishes objects or writes to a cloud account.

## Verification

`node --test crates/brain-cli/src/templates/integrations/adapters.test.mjs` runs actual example source with local fake official-shaped packages, checking auth/configuration, request shapes, response normalization, safe failures, capacity and release. Rust inline regressions verify selective CAS previews and compile server examples with the current compiler. These checks require no provider calls, installed SDKs or paid credentials. Passing fake-package tests does not prove a new SDK release remains API-compatible; review its official published types and run additional scoped integration checks as needed.
