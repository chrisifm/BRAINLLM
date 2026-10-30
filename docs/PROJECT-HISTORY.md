# Private project history and explicit learning

Project history is a bounded local checklist, changelog and evidence ledger for agents. It stores metadata in `.brain/history/history.json`, outside app assets. It does not retrain an LLM, execute checks, modify framework rules or publish incidents.

The framework APIs are `history::query(root, operation, args)` and `history::edit(root, operation, args)`, both returning `Result<Value, String>`. Arguments are JSON objects. Query `status` first to obtain the current revision. Every edit requires `expectedRevision`; use explicit null only when no store exists. A concurrent edit refuses a stale revision. Private writer locking and atomic replacement prevent lost updates. Schema version 1 includes a generator identifier; unsupported or unowned stores are refused.

## Checklist, evidence and lessons

Queries support `status`, `tasks`, `task`, `evidence`, `lessons`, `incidents` and `timeline`. Collection queries accept `limit` from 1 to 100, numeric `cursor`, and an optional `taskId`. The default page contains at most 25 records.

Edits support these explicit operations:

- `task-create`: `title` and `summary`; initial state is open.
- `task-update`: `taskId`, `state` and `summary`; states are open, active, blocked, done or cancelled.
- `evidence`: `taskId`, `summary`, optional `kind` (command/manual/artifact/observation), and `state` (passed/failed/unknown/not-run). Command evidence may name an executable with `tool` and an optional `exitCode`. Arguments and output are not stored or executed.
- `lesson`: `taskId`, `proposal`, `rationale`, and optional `evidenceIds` referencing evidence for the same task. Lessons remain proposed, including when no evidence is supplied. Review evidence quality explicitly before applying a lesson elsewhere.
- `incident`: `summary`, `cause`, `fix`, optional `classification` (app/framework/unknown), optional `taskId`, opaque `rid`, and optional existing source `file` plus `line`.

Task completion, command outcomes, manual observations and classifications are caller reports, not independent verification. Unknown and not-run are first-class evidence states. A lesson references reported evidence, not a guaranteed root cause. There is no automatic acceptance, rule mutation, model training or hidden command execution.

Each edit appends a compact timeline event with sequence, timestamp and record checksum. The whole document has a SHA-256 revision for CAS. These hashes detect revision differences; they are not signatures, trusted attestations or proof a command ran. History retains up to 1000 records per collection and 1 MiB total. A full store refuses edits and requires explicit private archival; it does not silently discard old evidence.

## Safe local incidents

An incident is a local structured bundle, returned immediately and queryable later. Its checksum covers the report metadata. Optional source locations must reference an existing guarded source file and real line, and include that source revision. Columns are unknown. A declared line does not establish the root cause. App/framework classifications explicitly remain caller-reported; unknown is the default.

Input schemas accept summaries rather than source bodies, environment dumps, request payloads, logs or raw stacks. Only known fields are persisted. Sensitive-looking text is refused, URL query strings and fragments are removed from summaries, strings and collections are bounded, and detected secrets prevent normal history loading. These checks cannot prove arbitrary prose secret-free: provide public technical summaries. No service notification, GitHub issue or remote upload is created.

## Upgrade adoption

The existing exact package-pin upgrade preview remains separate from optional design adoption. Its `operations` continue to describe only the package pin. The additive `adoption` field provides a known-file inventory, a separate non-destructive `designAdoption` plan when safe, preserved boundaries and explicit review instructions. A missing or invalid foundation yields needs-inspection, with no invented source transformation.

The design plan preserves native or legacy tokens and unknown baseline fields. Apply it separately through revision-checked changes after confirming the token source revision still matches. Neither preview installs dependencies, updates lockfiles, modifies component behavior, rewrites data nor claims a versioned source migration exists. See [the foundation contract](DESIGN-SYSTEM.md).
