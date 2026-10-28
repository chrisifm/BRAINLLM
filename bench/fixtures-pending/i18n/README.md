# Language fixtures (INTERNALS §37, B1401–B1405, B0306)

The i18n setup (brain.json `i18n`, `i18n/<locale>.json`) belongs to the whole app, so each case is a mini app that
`run.mjs` generates in a temp dir. Every app starts from the same **control app**: `brain new .`, then 3 locales
(`en` default, `es`, `ar` right-to-left), every visible string through `t()` (including the layout nav), a plural key
and a key with a placeholder, and logical CSS only (`margin-inline`, `padding-inline-start`, `text-align: start`). The
scaffold's demo component, its test and `ui/badge.brain` are removed because they carry fixed text outside these
cases. The control must give `[]`; each case applies exactly one change, so the expected code must be the only code
in the output.

Repository rule: no Spanish or Arabic in files, so the `es` and `ar` files hold the English text tagged
`[es]` / `[ar]`.

| Code | Case | Command | Expected |
|---|---|---|---|
| — | control app (3 locales incl. RTL, all text through `t()`) | `brain check --json .` | `[]` |
| B1401 | a view calls `t('nav.contact')`, a key in no locale file | `brain check --json .` | 3 × error, one per locale file (`i18n/en.json`, `es`, `ar`), each with an insert edit |
| B1401 | `i18n/es.json` is not valid JSON | `brain check --json .` | error on `i18n/es.json` (locale-file check, §37) |
| B1402 | `es` value identical to `en`, several words | `brain check --json .` | warning on `i18n/es.json` |
| B1402 | `es` value identical to `en`, one word (`Home`) | `brain check --json .` | info on `i18n/es.json` |
| B1403 | `<p>Welcome back</p>` with 3 locales | `brain check --json .` | 4 × warning: the view (edit to `{t('home.welcome_back')}`) + one per locale file (edit adding the key) |
| B1403 | the same text in a single-locale site | `brain check --json .` | nothing |
| B1404 | `t('home.greeting')` without `{ name }` | `brain check --json .` | 1 × error on the view |
| B1404 | `es` uses `{username}` where `en` uses `{name}` | `brain check --json .` | error on `i18n/es.json` |
| B1405 | brain.json `locales` includes `klingon` | `brain check --json .` | error on `brain.json` |
| B1406 | `<meta route es=… ar=… fr=…>`, `fr` is not a site locale | `brain check --json .` | 1 × error on the view |
| B0306 | `margin-left` + `text-align: right` with an RTL locale | `brain check --json .` | 2 × warning (`margin-inline-start`, `end`) |
| B0306 | the same CSS with LTR locales only | `brain check --json .` | nothing |

`run.mjs` prints per case `OK`, `MISS` (expected code absent), `EXTRA` (other codes appeared) or `DIFF` (expected
code with the wrong count or level), and lists the MISS cases separately: every case is proved by
`crates/brain-check/tests/i18n.rs`, so a MISS means the CLI does not reach that check yet. Exit 1 on any non-OK case
or a control app that is not clean.

```bash
cargo build -p brain-cli && node bench/fixtures-pending/i18n/run.mjs
```

Status (2026-10-13): the per-view checks (B1401 for `t()` keys, B1403, B1404 for `t()` vars, B1405 for
`<meta route>`, B0306) are `OK`. The locale-file checks of `brain_check::i18n::check_locales` (B1401 malformed file,
B1402, B1404 between files, B1405 in brain.json) are `MISS`: no command calls `check_locales` yet (TOOLCHAIN wiring).
Not covered by any code yet: a locale listed in brain.json with no `i18n/<locale>.json` file is silently skipped.

Move these into `examples/invalid` once every case prints `OK` (each needs its own app root, so they stay generated).
