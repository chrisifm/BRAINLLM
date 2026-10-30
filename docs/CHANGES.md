# Concurrent source changes

The change API coordinates agents through file leases, exact revisions and checked
plans. A lease reserves individual app-relative paths, so two agents can prepare
changes to separate files concurrently. The existing AST/text writer lock is held
only during commit and recovery, after compilation of a private preview.

## Protocol

The CLI/MCP adapter calls `changes::dispatch(root, operation, arguments)` with JSON.
The operations are `read`, `lease`, `plan` (alias `preview`), `apply`, `recover`
(alias `rollback`), `status` and `discard`.

1. `read` takes `path`, returning UTF-8 `content` and a lowercase SHA-256
   `revision`. A missing file returns null for both fields.
2. `lease` takes `action: "acquire"`, `owner`, `paths` and optional `ttlSeconds`
   (default 300, range 5–900). It returns an opaque OS-random `token`, expiry and
   current revisions. Each path has one lease, including paths not yet created.
3. `plan` takes `owner`, `token` and `operations`. Each operation requires
   `path`, `expectedRevision` and `content`. Use a null revision for a new file;
   use null content to delete an existing file. It validates an isolated app
   snapshot and returns `planId`, `dryRun: true` and an impact summary. App source
   files are unchanged.
4. `apply` takes `planId`, `owner` and `token`. It validates a fresh snapshot,
   including intervening changes from other agents, then commits with revision
   guards. Success returns `state: "committed"` and `dryRun: false`.
5. `recover` takes the same fields. It rolls back a committed batch or recovers
   an interrupted one. A file is restored only when its current revision matches
   the planned post-change revision. Other edits are preserved and reported as
   `preservedExternalChanges`; state remains `recoveryConflict` until resolved.
6. `status` reads metadata and impact; `discard` removes retained private payloads
   for a prepared, committed or rolled-back plan. Both require its owner and a
   valid lease covering its paths. Interrupted or conflicted plans must be
   recovered before discarding.

For `lease`, `renew` and `release` require `owner`, `token` and `paths`. Renewal
uses a new finite TTL. `list` returns active paths, owners and expiry without
exposing tokens. Leases expire automatically; a process holding no valid lease
cannot apply. After expiration, the plan owner can reacquire its paths with a new
token before applying or recovering. Owner names identify collaborators; they are
not an authentication boundary against another process running as the same OS
user.

```json
{
  "owner": "agent-profile",
  "token": "<token returned by lease>",
  "operations": [
    {
      "path": "comp/profile.brain",
      "expectedRevision": "<revision returned by read>",
      "content": "<p>Updated profile</p>\n"
    }
  ]
}
```

## Guarantees and conflicts

A same-file lease conflict fails before preparation. An expected revision mismatch
fails before overwriting that file and asks the caller to reread and prepare a
new plan. Plans prepared together for different files can apply successively:
application revalidates against current app source instead of requiring the
original whole-project snapshot to remain unchanged. If a dependency changes
*during* validation, the operation retries fresh validation once. Continued
changes return a retryable snapshot conflict.

Each replacement uses a unique temporary file, file synchronization and rename.
New files use exclusive hard-link installation so a competing creation cannot
be overwritten. A manifest with before/after hashes is synchronized before any
source writes. Original and planned bytes are separate private payloads, not
inline journal data. Recovery inspects all planned paths, including a write that
completed immediately before its progress record could be saved. Cooperating
AST, MCP text and transaction writers share the existing kernel writer lock.

A multi-file batch is a **recoverable logical transaction**. It does not provide
filesystem-wide atomic visibility: an observer can see an intermediate batch.
An unrelated editor does not take this lock; revision checks are optimistic,
with a remaining race between the final check and replacement/deletion. This is
not an atomic compare-and-swap against arbitrary editors. Recovery also cannot
distinguish another editor independently writing the exact planned bytes.
Synchronizing files and, on Unix, parent directories improves crash durability;
filesystem and hardware guarantees still apply. Recovery is explicit and
requires valid leases, avoiding automatic overwrites when a new process starts.

## Isolation, limits and retained state

- Paths must be normalized, UTF-8, app-relative and at most 240 bytes/32 parts.
  Traversal, symlinks, hidden paths, environment/credential paths and generated
  dependency/output directories are refused.
- Source must be UTF-8. Detected secret literals are rejected. Snapshot environment
  files contain declared keys with empty values; real environment values are
  never copied into previews or retained payloads. The real declarations are
  checked before creating each snapshot.
- Previews live in private OS temporary directories outside every app and are
  removed on ordinary completion/error. A killed process can leave its own
  randomly named `brain-changes-*` temporary directory; this contains redacted
  app source, never environment values. Unrelated temporary directories are not
  cleaned automatically.
- At most four preparations per app run concurrently, using kernel lock slots.
  Snapshots are limited to 2,048 files, 2 MiB per file, 32 MiB total and 32 directory
  levels. A batch has at most 32 operations, 4 MiB of new source and 4 MiB of old
  source backups. Source preparation checks a cooperative 120-second deadline;
  the synchronous compiler call cannot be forcibly interrupted inside that call.
- At most 128 file leases and 16 retained plans exist per app. Retained payloads
  have a bounded 128 MiB source budget across full plans, plus small metadata.
  Completed plans remain available for explicit rollback until discarded.
- Private `.brain/changes/` metadata uses restrictive directory/file permissions.
  Lease tokens are stored only as hashes. Journal/impact responses expose paths,
  revisions, area, byte counts and transaction state, not source bodies. `read`
  explicitly returns the requested nonsecret source.

After a crash before the initial manifest is saved, a private orphan plan
folder can remain. Its directory has no committed source changes and may be
removed manually after verifying that no preparation owns it. Unresolved
recovery payloads are retained; there is no silent eviction of unfinished work.
