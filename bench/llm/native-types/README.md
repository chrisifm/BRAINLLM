# Native type mismatch fixtures

These synthetic components introduce one declared mismatch each before browser
execution. `brain-check/tests/native_types.rs` checks the exact warning JSON,
1-based locations, severity and safe repairs. The prop fixture receives the
Counter prop contract; the other schemas are local declarations. Files are
checked as `comp/flow.brain` to keep the diagnostic contract stable.

| Fixture | Diagnostic | Introduced mismatch |
|---|---|---|
| prop | B2001 | Numeric prop passed a string literal |
| field | B2002 | Unique field typo on a closed object |
| body | B2003 | Number supplied for a declared string request field |
| binding | B2004 | Value binding on a boolean form field |
| nullable | B2005 | Nullable receiver dereferenced without a guard |

These fixtures provide deterministic checker evidence. They do not establish an
LLM success rate or run a model. A provider/model/effort-pinned authoring gate is
still required for measured model results; no paid provider is invoked here.
