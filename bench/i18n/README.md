# Languages QA (#153, INTERNALS §37)

A 3-locale example site and an end-to-end run over it, plus the LLM gate.

- `app/` — the example: `en` (default), `es`, `ar` (right-to-left). Views: `home` (static), `about` (static, translated
  slug `<meta route es="/es/about-us" ar="/ar/about-us">`), `counter` (interactive: counter with a plural `t()`, a
  `{name}` placeholder, `fmt.date`, `fmt.number`, `fmt.money`). Every visible string goes through `t()`, CSS uses
  logical properties only. Repository rule: the `es` and `ar` files hold the English text tagged `[es]` / `[ar]`.
- `run.mjs` — copies the binary and the app to `~/tmp/brainllm-i18n153` (sha256 of the binary in the output), then:
  1. `brain check` → `[]`, `brain doctor` → 0, and probes of the §37 features the example had to work around (each in
     its own scratch copy with one change): `t()` in `<meta seo>`, `<LangSwitch>` from the kit, `t()` in the
     `app.brain` shell, a plain `/about` link on an `es` page, `brain spec` teaching i18n, `brain add locale`,
     `brain i18n --missing` (BLOCKED while the command does not exist).
  2. `brain build` + `brain start`, then one fresh headless Chrome per page (CDP, `bench/lib/cdp.mjs`): URL scheme
     (`/`, `/es`, `/es/about-us`, `/ar/counter`…), `<html lang dir data-default-lang>`, texts in the locale and no raw
     keys after hydration, `hreflang` for the 3 locales + `x-default`, mirrored header for `ar` (the brand, first in
     the flex row, sits to the right of the nav), plural before/after a click, placeholder, `fmt.*` equal to the
     browser's `Intl` for that locale and equal to the prerendered HTML.
  3. Network of each first visit: the interactive view downloads exactly its own `i18n-<view>-<locale>` chunk, static
     views none; switching language (hover, then click the `es` link, then the `ar` link) loads the other chunk only
     then. Dictionary chunk sizes (raw, gzip, keys) go to the summary.
  4. LLM gate over `bench/autonomy/results/i18n153-{sonnet,haiku}` (brief: `briefs-153.json`): i18n in brain.json,
     `i18n/en.json` + `es.json`, no B1403, doctor 0, routes ≥ brief; reports whether `brain add locale` was called.
- `results/summary.{json,md}` — every check with PASS / FAIL / BLOCKED / INFO, evidence and owner (homedir as `~`).

```bash
cargo build --release -p brain-cli && cargo build -p brain-cli   # the autonomy harness uses target/debug/brain
node bench/autonomy/run.mjs --model claude-sonnet-5-5 --mode mcp --briefs ../i18n/briefs-153.json --tag i18n153-sonnet
node bench/autonomy/run.mjs --model claude-haiku-5-5 --mode mcp --briefs ../i18n/briefs-153.json --tag i18n153-haiku
node bench/i18n/run.mjs            # exit 1 on any FAIL; --llm runs the two harness commands first
```

Nothing is left running: `brain start` is stopped by its saved PID (SIGKILL after 5 s), each Chrome by `cdp.mjs`.

## Workarounds in the example (each is a FAIL or INFO probe in the run)

| Workaround | Why | Owner |
|---|---|---|
| Header, nav and language links in `views/_layout.brain`, `app.brain` is only the shell | `{t()}` and `<LangSwitch />` in the `app.brain` body are emitted literally; `brain check` says `[]` | COMPILER/BUILD |
| Language links written with `alternates()` | `<LangSwitch />` fails the build: `ui/lang-switch.brain` keeps `sig()` state in `ui/` (B0116) | RUNTIME #150 |
| Literal English `<meta seo title description>` on every locale | `<meta seo title={t('…')}>` makes the prerender throw `t is not defined`: pages render empty | COMPILER #149 |
| No `canonical="/about"` in views | a literal canonical is copied to every locale (B1828 error, cross-locale canonical); without it the build writes one per locale | SEO #152 |
| Nav paths in `nav.*_href` keys per locale | a root-relative `href="/about"` on an `es` page stays `/about`; no localized link helper | RUNTIME/BUILD |
