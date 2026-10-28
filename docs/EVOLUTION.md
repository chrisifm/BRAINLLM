# Improving BRAINLLM while using it

An LLM that finds a compiler bug, a repeated workflow failure, or a measurable performance opportunity may fix
the framework. Do not work around a compiler defect forever, silently drop application behavior, disable a checker,
weaken a test or claim an improvement without evidence. App work and framework work have different contracts:
read `brain spec` for the app; read CONTRIBUTING and the relevant INTERNALS section for the compiler change.

## Local proposal protocol

Use a clean, committed framework checkout and Node 22+ on macOS or Linux. The original app and framework checkout
remain intact. The controller is `node scripts/evolve.mjs`; every result is JSON with `schemaVersion: 1`.

1. Reproduce the failure with synthetic inputs and identify the responsible layer. Record expected/actual behavior.
2. Run `node scripts/evolve.mjs start cache-fix`. Edit only the returned detached `worktree`, including a regression
   test. Keep the reproducer small; for performance work measure before and after with the same workload.
3. Run the **original checkout's** `node scripts/evolve.mjs finish cache-fix`. The controller commits only the
   isolated proposal, audits its source, builds, runs Rust/runtime/fixture/English checks and builds a release binary.
   Logs and the proposal record remain private under `.brain/evolve/`. A failed gate leaves the proposal available
   for correction and leaves the active compiler unchanged.
4. For changes to the app-author contract, supply `--llm-run /path/to/run.json`. Generate that run with the candidate's
   release binary using `bench/llm/run.mjs --brain <candidate>/target/release/brain --model <explicitly selected model>`.
   A matching binary hash and a stable run are required; the baseline gate must pass. Historical unrelated results
   are not evidence. The controller does not select a model or silently buy model runs.
5. Run `node scripts/evolve.mjs activate cache-fix` to select the validated immutable binary. From the app directory,
   run `node /path/to/BRAINLLM/scripts/evolve.mjs run -- build`. This bounded command writes output to the returned
   private log. `rollback` restores the previous selection. Existing servers retain their original binary; restart
   the owned dev server to use another compiler. There is no executable hot-patching or automatic production deploy.

`status` returns stage results, SHA-256 and active/previous IDs. At most four proposals are retained; `discard <id>`
removes an inactive, clean worktree and its owned artifacts. It refuses dirty, active and rollback worktrees.
One operation holds a shared Git lock; overlapping work fails with an actionable message instead of queuing forever.
Each gate has a deadline, bounded log, process-group cleanup and two Cargo build jobs. On an abrupt controller crash,
inspect the recorded owner PID before manually removing its lock. Detached descendants and OS-level memory limits
require an external sandbox; this local trusted-contributor workflow is not one.

## Automatic contribution, maintainer review

Configure your own fork (or an authorized repository) once in the ignored `.brain/evolution.json`:

```json
{
  "pushUrl": "git@github.com:YOUR_ACCOUNT/BRAINLLM.git",
  "autoPush": true,
  "activate": false
}
```

After all gates pass, `finish` automatically pushes the exact validated commit to
`llm/<proposal>-<commit>`. You can also call `push <id>`. The controller checks source and reachable history for
private material and requires a license. Authentication stays in Git's credential store. It never force-pushes,
pushes tags, merges a branch, publishes npm packages or changes production. If the network result is uncertain,
inspect the exact destination ref before retrying. `activate: true` additionally selects a successful candidate
locally; a push failure does not undo successful local validation.

Open a pull request from that branch to `chrisifm/BRAINLLM:main`, using the PR template. All contributions, human
or LLM, require **Christian Irack Flores Maldonado (@chrisifm)** to review before merge. CI is read-only for PRs;
never run untrusted PR code with publication credentials. Rules, tests, dependencies, CI and the controller's own
policy cannot be changed by its automatic lane; propose those through a normal reviewed PR.

Public author information (full name, christian@irack.mx, ingenieria.dev and irack.mx) is intentional. App secrets,
private data, machine paths, environment files and session artifacts are not. `node scripts/privacy-audit.mjs
--history` emits categories and locations without matched values. Supplement it with a full secret scanner before
public releases. Detection is evidence of what was scanned, not a guarantee against every possible secret.

## Agent prompt

> Reproduce the framework failure with synthetic data. Read CONTRIBUTING and the affected INTERNALS contract.
> Start one isolated proposal, add a failing regression test, implement the smallest compatible correction and run
> every required gate. Do not weaken security or tests to pass. For app-author changes collect a fresh frozen LLM
> result; for speed claims measure comparable before/after workloads. Activate only a validated binary and retain
> rollback. If automatic contribution is configured, push the validated proposal branch, then open a PR for
> @chrisifm. Record gaps and failed stages honestly. Never merge or publish packages automatically.
