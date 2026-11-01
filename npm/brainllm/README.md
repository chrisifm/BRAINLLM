# BRAINLLM

## The web compiler for LLMs.

Your LLM owns the frontend. Give it a language built for the job.

BRAINLLM brings a component language, diagnostics, a Rust compiler and MCP tools into one agent workflow. You define
the intent, inspect the result and decide what ships. The model reads the spec, builds the frontend and checks its
work against a concrete contract. It is not a React or Next alternative: it aims to be the first self-improving compiler
for LLMs, where a compiler gap an agent hits becomes a tested proposal for a person to review, and `brain risk` measures
an edit before it is made.

After install, `brain guide docs` lists the reference pages and `brain capabilities` what an app can use; this package
carries them inside the binary.

This package is the native compiler launcher. The package build copies the full root marketing README here and
converts its image/document links to absolute public URLs. See the
[public repository](https://github.com/chrisifm/BRAINLLM) for current installation availability and measured benchmarks.

The platform binary is an optional dependency, with no install-time download script. `BRAIN_BINARY` can select a
local build. MIT licensed; principal author Christian Irack Flores Maldonado.
