# AGENTS.md — working on the BRAINLLM repository

- Changing the framework: read `CONTRIBUTING.md` fully, then the `docs/INTERNALS.md` section you touch.
- Writing an app with BRAINLLM: read `SPEC.md` (or run `brain spec`); nothing outside it exists.
- Before finishing: `cargo test --workspace && npm test --prefix runtime && node bench/fixtures-check.mjs && scripts/check-english.sh`.
- Changes that affect what app authors write: run the LLM gate (`node bench/llm/gate.mjs`).
- Never add third-party parsers/bundlers/minifiers. Never merge or publish packages automatically.
- Framework defects and measurable improvements may be fixed while building an app: follow `docs/EVOLUTION.md`.
  Use isolated proposals, regression tests and all gates; automatic pushes go only to a configured review branch.
  Every contribution requires @chrisifm's review before merge. Do not weaken gates to pass them.
- Migrating an existing frontend: use `scripts/migrate-plan.mjs` and `docs/MIGRATION.md`; preserve behavior in tested slices.
