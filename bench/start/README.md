# `brain start` QA (task #120, INTERNALS §34)

End-to-end checks of the production supervisor against real apps built by `brain build`.

## Method

```sh
node bench/start/run.mjs                      # cargo build --release -p brain-cli, then every case (about 4 min)
BRAIN=/path/to/brain node bench/start/run.mjs # test a given binary, no build
node bench/start/run.mjs --only 3,3b,3r --reps 10 --clients 50 --reloads 3
```

- The binary is copied into the scratch dir (`BRAIN_START_WORK`, default `<tmp>/brainllm-start-bench`) and
  fingerprinted (sha256, first 12 hex), so a concurrent rebuild of `target/` cannot change what is tested.
- Apps: `brain new srv` with its own `views/about.brain`, written whole by the bench (a `server:` function and a
  button that calls it, so the build emits `dist/server.mjs`, the real RUNTIME server; it does not patch the
  scaffold's file, so a scaffold change cannot break the setup), and `brain new stat` made truly static: the scaffold's guarded and server views (`/403`,
  `_guard`, login, account, posts), `server/*.jsbrain`, the session layout and every view with `load` or `server:`
  are removed, the nav links to them dropped, and the build must emit no `server.mjs`. A failed app precondition
  fails only the cases that need that app; the rest still run. Each case runs on a fresh copy of the built app.
- The first exec of a freshly copied binary waits for macOS to assess it (126 s on a loaded machine, 2026-10-13). That
  is what killed `brain new srv` in the failed run: every command has a 120 s timeout. The bench now runs
  `brain help` once right after the copy, with no timeout, and records the time (`firstExecMs`).
- Every setup command (`brain new srv`, `brain new stat`) must exit 0: otherwise the bench stops at once with the
  command, its exit code and its stderr.
- `brain reload` is called asynchronously, so the load generator keeps running while the CLI waits for the result.
- Every supervisor started is stopped with `brain stop`; anything still alive is killed only by the pid the script
  spawned or read from `brain status --json`. The run fails if any process is left.
- `--install` runs with `HOME` set to a temp dir and is never enabled. The plist's argv is run with only the plist's
  environment (plus `HOME`/`USER`), which is what launchd does. The systemd branch is compiled out on macOS, so the
  unit template is checked in `crates/brain-cli/src/start.rs`.
- Output: `results/summary.json` and `results/summary.md` (committed); `results/raw/` holds per-request data,
  `logs/app.log`, stderr of every supervisor and the plist (ignored by git). Exit 1 on any failed case.
- `3p`/`3bp` (only with `--only`): the reload cases with a candidate drain patched into `dist/server.mjs`; kept as the
  evidence that led to RUNTIME's drain fix.

Run of 2026-10-13: binary sha256 `3eb3ee4dd74c` (`target/release/brain`, after TOOLCHAIN's #120 fixes and
RUNTIME's drain fix), Node v26.8.1, macOS arm64.

## Results

| Case | Result | Numbers | Evidence |
|---|---|---|---|
| 1a kill -9 of the child, fresh supervisor per rep (10 reps) | PASS | kill → first 200 on `/_health`: median 53 ms, max 80 ms, min 38 ms | `raw/1a-kill.json` |
| 1b 12 kill -9 back to back, one supervisor | PASS | 88, 113, 84, 95, 92, 78, 65, 81, 84, 83, 77, 94 ms (median 84, max 113); still serving, no crash loop | `lastExit` "killed from outside (killed by signal 9)", restarts 12 |
| 2 RSS over `--max-memory 1M --rss-ms 200` | PASS | `memory` event (rssKB 53168, limit 1024), pid 83173 → 83208, 200 again 233 ms after watch start; `lastExit.reason` "memory", `memoryRestarts` 1 | held over the limit for 15 s: 70 memory restarts, supervisor still up |
| 3 `brain reload` ×3, 50 keep-alive clients, `--instances 2` | PASS | 113,606 requests, **0 failed**; during reloads 8,318 req, p99 13.1 ms (p50 1.8, max 29); overall p99 5.6 ms | every CLI reply `{"ok":true,"reload":"done","build":"7ptme391"}`, exit 0; all pids replaced |
| 3b same, `Connection: close` clients | PASS | 51,248 requests, **0 failed**; during reloads 5,291 req, p99 21.4 ms | same |
| 3r reload to a build whose child crashes at startup, 50 keep-alive clients | PASS | reload exit 1 after 134 ms, `ok:false` "a new instance exited before it was ready (exit code 1); the old instances keep serving"; instances kept; 40,744 requests, **0 failed** (p99 95 ms during the reload) | `raw/3r-requests.json` |
| 4a child dies before ready, always | **FAIL** | exit 1 after **51.8 s**, 9 restarts, crash loop declared, stderr explains it and shows the child error | exits at +0, 0, 1, 1, 2, 4, 7, 14, 26, 52 s |
| 4b ready once, then every restart crashes | PASS | exit 1 after 52.8 s, 9 restarts / 10 exits, stderr "crash loop (10 crashes in 60 s)" + last log lines | |
| 5 `--install` | PASS | 16/16 checks: plist `PATH` has node's dir first, the plist's argv serves with only the plist's environment; systemd template has `User=` (never root), quoted `Environment="PATH=…"`, quoted `ExecStart`/`WorkingDirectory` | plist below |
| 6 static app, no `server.mjs` | PASS | `ETag "ujfsd85a"` = signature printed by `brain build`; `If-None-Match` → 304 with no body; `/_health` → `{"ok":true,"build":"ujfsd85a"}`; 0 processes under the supervisor; 10/10 `_headers` headers; `/_headers` → 404 | |

## Open failure

**4a: a child that dies before `brain:ready` takes 52 s to end in a crash loop (owner TOOLCHAIN,
`crates/brain-cli/src/start.rs:524-529`).** Detection is now immediate and stderr is right, but a child that never
reached ready doubles the backoff on every crash (100 ms → 25.6 s), so the 10th crash lands at about 51 s (the sum
of 100 ms × 2⁰…2⁸) instead of "well under 30 s". Options: cap the backoff for deaths before ready (for example at 1-2
s), or give up after N consecutive deaths before the first ready (a build that cannot boot will not heal by waiting).

## Notes

- The plist's `PATH` is the whole interactive `PATH` of whoever runs `--install` (35 entries here, with versioned
  Cellar, nvm and tool directories). It works and has no secrets, but it freezes a developer shell in a boot service;
  node's directory plus the system defaults would be enough.
- With `--max-memory` below the idle RSS the app restarts forever (70 times in 15 s here) without a crash loop, by
  design (memory restarts are planned). A limit set too low is only visible in `memoryRestarts` and the log.

## Fixed since the first run (binary `c9b24f3832f4`, same day)

Reload under load lost 150 of 192,633 requests (ECONNRESET, every keep-alive client once per reload) and 35 with
`Connection: close` (drain called `closeIdleConnections`; fixed by RUNTIME with the patch measured in 3p/3bp);
a child dying before ready waited 30 s with an empty stderr and no retry; the plist had no `PATH`, so `node` was not
found under launchd; the backoff grew on external kills, which counted toward the crash loop; `lastExit` did not say
"memory"; `brain reload` returned before the reload finished.

## `brain start --install` output (macOS, this run)

```
wrote ~/tmp/brainllm-start/work/install/.brain/run/dev.brainllm.install.plist
enable it with:
  cp ~/tmp/brainllm-start/work/install/.brain/run/dev.brainllm.install.plist ~/Library/LaunchAgents/ && launchctl load -w ~/Library/LaunchAgents/dev.brainllm.install.plist
```

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>dev.brainllm.install</string>
  <key>ProgramArguments</key>
  <array>
    <string>~/tmp/brainllm-start/work/brain</string>
    <string>start</string>
    <string>--port</string>
    <string>3999</string>
    <string>--instances</string>
    <string>2</string>
    <string>--max-memory</string>
    <string>300M</string>
  </array>
  <key>EnvironmentVariables</key>
  <dict>
    <key>PATH</key><string>/opt/homebrew/bin:/usr/local/bin:/System/Cryptexes/App/usr/bin:/usr/bin:/bin:/usr/sbin:/sbin:…(the rest of the interactive PATH, 35 entries)</string>
  </dict>
  <key>WorkingDirectory</key><string>~/tmp/brainllm-start/work/install</string>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
</dict>
</plist>
```

The full file is in `results/raw/5-install.plist`. Review: absolute path of the running `brain` · `start` plus the
flags given · node's directory first in `PATH` · `WorkingDirectory` = app root · `RunAtLoad` + `KeepAlive` · nothing
written under `HOME` · no sudo · the secret added to `.brain_env` is not inlined.

systemd template in `start::install` (Linux only; `User=`, `Environment=` and every path are filled and quoted at
install time):

```ini
[Unit]
Description=BRAINLLM app {name}
After=network.target

[Service]
{user_line}WorkingDirectory={}
{env}ExecStart={}
ExecReload=/bin/kill -HUP $MAINPID
Restart=always
KillSignal=SIGTERM
TimeoutStopSec=15

[Install]
WantedBy=multi-user.target
```
