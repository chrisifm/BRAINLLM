# Reproducible agent benchmarks

The provider-neutral runner under `bench/agents/` measures explicit adapter invocations against isolated create, edit, repair, atomic-conflict and CRUD fixtures. It does not choose a provider, model, reasoning effort or paid default. No provider was invoked while implementing its replay tests. A fresh model run and the existing LLM release gate remain **pending**.

## Invocation

Prepare a private configuration file with an explicit executable argument array, model, effort, compiler binary, destination and validators. Arguments are passed directly without a shell. `{model}`, `{effort}`, `{binary}` and `{workspace}` substitutions work within command arguments. For example, replace the illustrative adapter path, model and effort with an approved adapter configuration:

```json
{
  "command": ["/absolute/path/to/adapter", "--model", "{model}", "--effort", "{effort}"],
  "model": "EXPLICIT_APPROVED_MODEL",
  "effort": "EXPLICIT_APPROVED_EFFORT",
  "binary": "/absolute/path/to/brain",
  "out": "/absolute/path/to/private-run.json",
  "timeoutMs": 120000,
  "maxOutputBytes": 1048576,
  "contextPaths": [],
  "validators": [
    ["{binary}", "check"],
    ["node", "/absolute/path/to/BRAINLLM/bench/agents/verify-fixture.mjs", "{binary}"]
  ]
}
```

```sh
node bench/agents/run.mjs /private/config.json bench/agents/fixtures/tasks.json
node --test bench/agents/runner.test.mjs
```

The output path must be new; an existing result is never overwritten. Keep generated reports outside the source tree, preferably in the user's chosen artifact directory. Do not put credentials in command arguments or prompts. Authentication belongs in the adapter's configured secret store/environment. Reports contain captured adapter output and use private file permissions.

## Adapter protocol

The adapter receives exactly one JSON request on stdin, ending in a newline:

```json
{
  "schemaVersion": 1,
  "task": {"kind":"create","prompt":"...","files":{}},
  "workspace": "/temporary/owned/workspace",
  "model": "explicit configured model",
  "effort": "explicit configured effort",
  "binary": "/absolute/compiler/path",
  "context": [{"path":"SPEC.md","sha256":"...","content":"..."}]
}
```

Its working directory is the seeded workspace. It may modify that workspace and use the supplied compiler. It must emit one JSON object on stdout and send diagnostic logs to stderr. A successful process must still pass every configured validator. Model and effort are configured identity, not independently verified provider identity; adapters must faithfully forward them.

```json
{
  "metrics": {
    "firstPass": true,
    "turns": 1,
    "inputTokens": 1000,
    "outputTokens": 200,
    "interventions": 0
  }
}
```

Every metric is optional. Unknown turns, token counts, interventions and first-pass status remain `null`, with `not-reported` provenance. Reported metrics carry `adapter-reported` provenance. The runner never infers first-pass from exit code, estimates token usage from text length or treats missing interventions as zero. Define the adapter's turn, first-pass and intervention accounting in its own versioned documentation before comparing runs. Local replay results have `evidenceMode: replay` and do not count as LLM measurements.

Validators receive `{task,report}` on stdin. Their exit status is measured independently from adapter metrics. Include the compiler check plus task-specific behavioral validators when evaluating behavior, not merely source validity.

## Fixtures and evidence limits

The five bundled prompts cover:

- Create: an accessible keyed task list with an add flow.
- Edit: preserve a counter and unrelated bytes while adding configurable increments.
- Repair: resolve an actual undefined-name diagnostic while preserving counter behavior.
- Atomic conflict: provide and execute a bounded script exercising stale revision rejection and preservation of concurrent bytes.
- CRUD: generate and apply resource management UI from a real static backend contract while preserving its policy.

`verify-fixture.mjs` checks source acceptance and preservation and executes the atomic proof script. Its text checks are deliberately labeled source acceptance; they do not establish browser behavior, accessibility conformance, semantic correctness of agent-written proof code or database integration. Review the conflict script and add an independent adversarial validator for release claims. The CRUD fixture has no running database, so its result cannot establish live CRUD success. Do not compare React, Angular, Next.js or other frameworks without a separately versioned adapter, matching task semantics and equivalent validators.

Each result records SHA-256 identities for framework sources, SPEC, compiler binary, additional context, task fixture and exact adapter input. Sources and binary are hashed before and after the run; changes mark the result unstable. The exact input contains a unique temporary workspace, so its hash differs across runs; fixture/context hashes identify the reproducible content. The result records local execution duration, bounded output bytes, exit status, cancellation reason and validation outcomes. Source identity covers `crates`, `runtime`, `SPEC.md`, Cargo manifests/lock and the runner/fixtures. It intentionally excludes hidden files/directories (including `.env*`, `.brain_env` and `.brain`), private/secret/credential directories, key/certificate/database/log files, build output, dependencies, caches and result directories. Additional context paths are explicit inputs: never supply secret files.

## Resource lifecycle

Concurrency is one. A batch contains at most 20 tasks and eight validators per task. Each task shares a total deadline across the adapter and validators; the batch has a 30-minute deadline. Configured task time is 50 ms to ten minutes. Combined stdout/stderr retention is capped at 1 KiB to 4 MiB per command. Source hashing has file-count and byte bounds; context is capped at 32 files of 4 MiB each and 16 MiB total. Fixture seeds have at most 64 files of 4 MiB each and 16 MiB total.

Each command owns a POSIX process group. Timeout, output overflow and interrupt send TERM, escalate to KILL and reap the child; remaining group descendants are terminated on completion. Commands execute without a shell and are never retried. Temporary workspaces are removed in `finally`. This is process ownership and workspace separation, **not an operating-system sandbox**: trusted adapters must respect the workspace boundary. Deliberately detached external processes or remote provider jobs need adapter-specific cancellation and cannot be claimed cancelled by this runner. On Windows process-tree ownership requires an external Job Object wrapper; POSIX group cancellation is unavailable there.

## Existing LLM gate

Run the repository's established `bench/llm` workflow with explicitly approved model settings, then pass the fresh result to `node bench/llm/gate.mjs <result.json>`. This new runner does not silently invoke that workflow, accept a baseline or replace the release gate. Local replay validation verifies benchmark plumbing only. A release claim remains pending until a fresh provider run and all required gates supply actual evidence.
