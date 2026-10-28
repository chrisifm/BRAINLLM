# AGENTS.md — app BRAINLLM
1. Before writing code read `brain spec` (or `brain spec --topic <topic>`); what is not there does not exist.
2. Read `brain map --json` to see existing routes, components, props and modules; import only what it lists.
3. Create files with `brain add view|comp|ui <name> --props "…"` and complete them; do not write them from scratch.
4. Loop: edit → `brain check --json` → apply `brain fix`, then each remaining error's `fix` → repeat until `[]`.
5. A diag with `"stuck": true` failed the same way 3 checks in a row: stop patching, read its `explain` (why + correct example) or `brain explain <code>`, and rewrite that part.
6. Done means `brain doctor` exits 0 (check + tests + accessibility + leaks). Not before; never claim a task done with doctor failing.
7. `brain run` serves the app. Every command accepts `--json` and exists in MCP (`brain mcp`); a command that fails internally exits 3 and prints the next command to run.
