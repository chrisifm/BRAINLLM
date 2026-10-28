# @christianirack/brainllm

BRAINLLM is a web framework designed for LLMs to write: one `brain` binary with the compiler, dev server, production
build and MCP server.

```sh
npm create @christianirack/brainllm my-app
cd my-app && npx brain run
```

- `brain spec` prints the language spec the model writes against; `brain mcp` exposes every command as an MCP tool.
- The binary for your platform comes from an optional dependency (`@christianirack/brainllm-<os>-<arch>`); no
  postinstall script and no download.
- A global `brain` runs the version the project pins in its `package.json`. `BRAIN_BINARY=<path>` runs another binary.
- `brain.schema.json` is the JSON Schema of `brain.json`.
