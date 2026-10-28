# Build with it. Let your LLM make it better.

The agent building an app sees the compiler's rough edges first. Give it permission to turn a concrete finding
into a tested improvement. A useful contribution can be a better diagnostic, a reproducible bug or a faster
operation with the same result.

This document is a playbook for that agent. It does not install an autonomous background service. Running
`brain build` does not call an LLM or modify the compiler. The [evolution controller](EVOLUTION.md) implements the
isolated proposal, validation, optional push and reversible local activation workflow.

## Notice, prove, propose

After a completed app workflow, ask what actually caused friction. Record the input, expected behavior, observed
behavior and affected layer. If everything worked and no measurable improvement is apparent, continue building
the app. Do not manufacture a refactor to fill a quota.

For a real finding:

1. Reduce it to synthetic inputs. Keep customer code, credentials and private data out of the proposal.
2. Read [CONTRIBUTING.md](../CONTRIBUTING.md) and the affected [internals](INTERNALS.md). Preserve the app-author
   contract. Changes to that contract require the fresh frozen LLM benchmark described in the contributor guide.
3. From a clean, committed framework checkout, run `node scripts/evolve.mjs start focused-fix`. Work only in the
   returned isolated worktree. Add the regression test before implementing the smallest compatible correction.
4. For performance work, measure before and after on the same input and environment. Include correctness checks,
   cold and warm behavior when relevant, and the scope of the measurement. A cache-key win is not a build-speed win.
5. From the original checkout, run `node scripts/evolve.mjs finish focused-fix`. Inspect the returned stage results
   and logs. A failed gate is a finding to resolve, never a reason to remove the gate.
6. With automatic contribution configured, a successful finish pushes the validated commit to a review branch.
   Open a PR with the reproducer, resulting behavior and evidence. @chrisifm reviews every merge.

The controller's automatic lane deliberately excludes changes to its own gates, policy, CI and dependencies.
Those changes need the normal reviewed PR workflow. Consult EVOLUTION.md for the exact boundaries, proposal
limits, deadlines, locks, activation and rollback behavior.

## Opt in to automatic contribution

First choose a fork you own or a repository you are authorized to write to. In the ignored framework-local
`.brain/evolution.json`, use the actual credential-free Git URL:

```json
{
  "pushUrl": "git@github.com:YOUR_ACCOUNT/BRAINLLM.git",
  "autoPush": true,
  "activate": false
}
```

Replace `YOUR_ACCOUNT`; keep credentials in Git's credential store. Preserve any existing configuration unless
you intend to change it. `finish` pushes only after its gates pass. `node scripts/evolve.mjs push focused-fix` is the
explicit alternative. Neither path merges, force-pushes, publishes a package or deploys production.

Local activation is a separate choice: `node scripts/evolve.mjs activate focused-fix` selects the validated binary
for controller-managed commands. `rollback` restores the previous selection. Existing processes keep their binary;
this does not patch a running compiler in memory.

## Prompt for an agent with contribution enabled

Use this only after configuring the authorized destination above:

```text
While building this app, look for concrete BRAINLLM defects and measurable
improvements. At the end of each completed workflow, check the evidence for one
useful finding. Do not invent work when there is none.

I have opted in to automatic proposal pushes through the existing
.brain/evolution.json in the framework checkout. Keep its authorized destination
and activation setting unchanged. Do not copy credentials into the repository.

For each finding, read CONTRIBUTING.md, docs/INTERNALS.md and docs/EVOLUTION.md.
Reproduce it with synthetic data, start one isolated proposal with evolve.mjs,
add a failing regression test and make the smallest compatible correction.
For a speed claim, measure comparable before/after workloads. Run finish from
the original checkout and inspect every required result. App-author changes
need a fresh frozen LLM run with an explicitly selected model; do not silently
purchase model runs or substitute old results.

When the proposal passes, let the configured controller push its validated
commit. Open a pull request for @chrisifm with the evidence. Do not merge,
weaken checks, publish packages or deploy production. Keep app behavior intact.
Record blocked stages and remaining gaps. Stop owned temporary resources.
If the fix needs policy, CI or dependency changes, use a normal reviewed PR.
```

## Ideas to investigate, not implemented promises

These are candidate experiments. Some related mechanisms already exist; inspect the current code and tests
before proposing a change. An idea becomes a feature only after implementation, evidence and review.

| Idea | Small first experiment | Evidence before proposing it |
|---|---|---|
| Explain a cache miss to the agent | Add an opt-in diagnostic for one missed compiler-cache entry, with input categories rather than source or secrets | A synthetic source/options/dependency change produces the correct reason; disabled instrumentation adds no material cost |
| Reduce repeated compiler reads | Profile a multi-component build and reuse one proven immutable read within that operation | Equal output and diagnostics, correct invalidation after edits, lower measured reads or elapsed time |
| Make migration parity easier to inspect | Summarize a small migration ledger's missing behavioral checks | Known gaps remain visible; accepted slices always point to real evidence; no automatic acceptance based on compilation alone |
| Make a confusing diagnostic actionable | Reproduce one recurring failed agent repair and improve its fix text or safe edit | Regression test proves the proposed repair resolves the error without changing intended behavior; measure a fresh LLM run if the author contract changes |
| Give queue pressure useful feedback | Measure one saturated queue and report its bounded admission state | Load test proves the active/pending limits, timeout behavior and cleanup; reporting does not retain unbounded history |
| Catch browser-resource leaks earlier | Exercise cancellation at several points in one owned browser operation | The operation's pages, processes and listeners are released while borrowed resources remain intact |
| Trim the context needed for a focused change | Compare an existing map/context response with a smaller version for one task | Relevant dependencies are retained; no private values leak; agent completion and token use are measured on a frozen workload |

Start with one idea and one reproducer. The contribution should make the next app easier to build, with enough
evidence for another developer to understand why.
