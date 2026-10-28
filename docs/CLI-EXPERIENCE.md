# A terminal with a point of view

Your intent. Its language.

BRAINLLM brings its circuit-brain identity into `brain check`, `brain build` and `brain run`. In an interactive
terminal, a compact ASCII mark introduces the task, then completion reports the elapsed time of that invocation.
The existing build phase display and dev rebuild timings report real work as it happens.

| Command | Opening | Completion |
|---|---|---|
| `brain check` | Checking your frontend against its contract. | Check complete. Ready for your next idea. |
| `brain build` | Turning your frontend into browser-ready output. | Build complete. Your intent, compiled. |
| `brain run` | Wiring your frontend for the next idea. | Dev server ready. Save a change and keep building. |

A failed check says that issues were found; a failed build says it stopped. Neither prints the successful completion
message. Timing measures this invocation, including its command setup, rather than comparing it with another tool.
A diagnostic, not a slogan, tells the agent what to fix.

## Human personality, machine precision

The banner and its conversational messages are written to stderr only when both stdout and stderr are terminals.
They are suppressed for redirected streams, `--json`, `--profile`, `--quiet`, `-q`, `--progress=off`,
`--progress=plain`, nonempty `CI` or `BRAIN_MCP`, and `TERM=dumb`. `--progress=rich` cannot override those branding
protections. Setting `NO_COLOR`, even to an empty value, removes the banner's color while retaining plain ASCII in
an interactive terminal. The existing progress renderer has its own mode selection.

LLMs should use the existing JSON commands or MCP tools: exact diagnostics, file locations, suggested fixes and
measured results carry the feedback. Branding adds no fields to those protocols. It adds no spinner thread, timer,
network request or background process.

## Speed claims must earn their place

A fresh build does not establish a competitor comparison. Never print a fixed multiplier on every command.
The [build fixtures](../bench/build/results/summary.md) and
[cache-key microbenchmark](../bench/build/results/compiler-cache-lookup.json) measure different operations.
Any comparative message must identify the measured operation, baseline and benchmark conditions; neither result
is a universal promise about app performance or every invocation.
