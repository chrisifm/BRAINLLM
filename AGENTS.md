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

## LinkedIn publication through Codex Viewer

- When publishing project content on LinkedIn through Codex Viewer, observe new editor and publication steps as they happen. Add each successfully verified reusable step to the existing `codex-linkedin` plugin in the local Codex Viewer installation, including its native-editor companion when applicable.
- Capture selectors, formatting, media upload and placement, draft persistence, publication and final URL verification. Record a failure lesson only after its cause and correction have been verified; do not label an untested path as supported.
- Reuse the assigned task tab and installed plugins. Preserve other sessions and unrelated edits. Keep browser identifiers, account data and credentials out of this public repository.
- Verify the plugin against the actual operation, document its invocation and supported interface language, and explicitly confirm its creation or update and the checks performed when reporting completion.
