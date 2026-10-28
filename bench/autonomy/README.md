# Autonomy benchmark (INTERNALS §25)

Can an agent build a complete multi-view app from a one-paragraph brief, alone? Each brief in `briefs.json`
goes to `claude -p` in an empty directory with no human turns, no Bash, no web, and no user settings or hooks.

| Mode | Tools the agent gets |
|---|---|
| `mcp` | only the `brain` MCP server (`brain_write`/`brain_edit` included) — the §25 target |
| `mcp-host` | the `brain` MCP server plus the host's `Read`/`Write`/`Edit` — baseline |

**Done** = an independent `brain doctor --json` exits 0 after the agent stops **and** `views/` still holds at least
the routes the brief asks for (`pages`). Pruning the brief to get a clean doctor does not count.

Metrics per app: done, routes/brief, tool calls (per tool), input/output tokens, `stuck: true` answers (stuck
loops), human interventions (the final message asks the user something), wall time, cost.

```bash
cargo build -p brain-cli
node bench/autonomy/run.mjs --model claude-haiku-5-5 --mode mcp          # all briefs
node bench/autonomy/run.mjs --model claude-sonnet-5-5 --mode mcp --only habits,events
```

The binary is copied into the run directory first, so a concurrent rebuild cannot change it mid-run. Output:
`results/<tag>/<id>/` (the app), `results/<tag>/<id>.transcript.jsonl` (stream-json) and `results/<tag>.json`.
