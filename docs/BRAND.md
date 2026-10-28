# BRAINLLM messaging

## Position

The web compiler for LLMs.

Your LLM owns the frontend. Give it a language built for the job.

BRAINLLM treats the language model as the primary author of frontend code. A compact language, a normative spec
and actionable compiler feedback let the agent work inside an explicit contract. Human developers define the
product, inspect the output and review contributions.

## The improvement loop

Build with it. Let your LLM make it better.

The agent that uses BRAINLLM is also in a position to improve it. A concrete failure or measured bottleneck can
become an isolated proposal with a regression test and evidence. An explicitly configured controller can push
that validated proposal automatically. Every merge still requires the principal author's review.

Treat every completed workflow as a chance to notice an improvement, not as a requirement to change code. The
compiler does not call an LLM or rewrite itself merely because someone runs a build.

## The tooling-fatigue angle

A form shouldn't need an ecosystem.

Less tooling. More web.

Speak to developers who want the agent to spend its effort on application behavior. Make the smaller toolchain
concrete: BRAINLLM owns its Rust compiler, bundler and minifier; the browser runtime has no third-party dependencies.
Do not invent a universal number of packages required by other frameworks. Do not claim that BRAINLLM ships fewer
bytes than plain HTML. Plain HTML is a valid choice for pages that do not need this framework.

## Voice

Direct, technical and ambitious. Show the mechanism and give developers something to try. Keep the human in
control without making them the primary reader of the application language.

Use:

- "Built for the model writing your frontend."
- "Give your agent a language it can check."
- "From a product brief to a tested frontend."
- "The model builds. You decide what ships."
- "Build with it. Let your LLM make it better."
- "A form shouldn't need an ecosystem."
- "Less tooling. More web."

Avoid "humans not welcome": the language targets LLM authors, while the project welcomes human maintainers and
contributors. Avoid "AI-powered compiler": the compiler itself is a deterministic Rust toolchain, not an LLM API.
Do not promise universal speedups, zero bugs, instant migrations or autonomous production publication.

## Evidence behind numerical copy

The checked-in [build results](../bench/build/results/summary.md) and raw timing files report 48.2 ms warm median
for BRAINLLM, 338.7 ms for React 19 + Vite and 357.8 ms for Svelte 5 + Vite. These support 7.0x and 7.4x respectively
for the frozen dashboard fixture, with 20 measured runs and the [recorded versions](../bench/build/results/versions.json).
The fixtures differ in server/client rendering and offline output; link the [method](../bench/build/README.md).
Historical report prose contains older timings. Prefer the linked raw files over that prose when they disagree.

The [43.3x cache result](../bench/build/results/compiler-cache-lookup.json) measures 10,000 cache-key lookups only.
Do not use it as full-build performance or as a comparison with webpack. There is no evidence here for "50x faster
than webpack." Keep scope beside the number, including in image copy.

## Calls to action

Primary: Connect your coding agent. Tell it what to build. Try the app.
Secondary: Read the spec.
Contributor: Improve the compiler.
Proof: Inspect the benchmarks.

Lead with the agent connection and product brief, not shell installation. Keep manual commands in an optional
section. Machine-facing contracts and compiler feedback are the primary interface; humans describe, try and
review the app. Never promise that a human will never need to inspect code.

Pair optional installation instructions with an agent prompt that reads the served spec, runs actual checks and reports
gaps honestly. Never imply that npm packages have been published unless their registry release is verified.

## Source names, output names and tokens

Readable intent. Lean browser output.

Production minification shortens eligible JavaScript bindings while preserving required properties, exports and
external references. A long internal name can become a short name; no specific letter is guaranteed across builds.
Keep application source meaningful for the model and the reviewer. Do not equate minified output bytes with LLM
input or output tokens. Context reduction is a separate goal to measure with frozen agent tasks.

## Automatic protection language

Use "errors should give the agent a next move" for compiler diagnostics and captured runtime errors. Be specific:
component fallbacks, bounded retry, circuit breakers, form schemas, server boundaries and verification commands.
Never say every failure is caught, every diagnostic has an exact edit, or every error repairs itself. Form schemas
do not replace server authorization or application business rules. A passing compiler check is not full UI parity.

## Invite scrutiny

Don't trust the headline. Run the benchmark.

Speak to curious developers, researchers and experimenters. Give them the frozen fixture, methodology, raw timings
and an agent prompt that reproduces the comparison. A result that contradicts our headline is still useful evidence.
Use milliseconds for timings and KiB gzip for compressed payloads, with included files and exclusions explicit.

Link the public site prominently: https://chrisifm.github.io/BRAINLLM/.

Rust describes the implemented compiler pipeline. Real Chrome/CDP checks describe the browser interactions tested,
not universal compatibility. Historical model evaluations support only their recorded models, tasks and source
revisions. Do not turn those results into an assertion that frontier models authored the entire codebase. Do not
publish imagined development costs, millions of tokens spent or dollar totals without a real accounting ledger.
