# HTML first QA (task #157, INTERNALS §38)

Every page of a fresh scaffold and of `examples/` with JavaScript disabled, then the same pages with JavaScript on.

## Method

```sh
node bench/htmlfirst/run.mjs                      # cargo build --release -p brain-cli, then every check (about 1 min)
BRAIN=/path/to/brain node bench/htmlfirst/run.mjs # test a given binary, no build
```

- The binary is copied to `BRAIN_HTMLFIRST_WORK` (default `~/tmp/brainllm-htmlfirst`) and fingerprinted (sha256,
  first 12 hex), so a concurrent rebuild of `target/` cannot change what is tested.
- Apps: `brain new scaf` plus the probe files in `probe/` (`views/probe-form.brain`: a `form()` whose `submit` is a
  `server:` function that can throw `{ errors }`, throw an `Error` or return `{ redirect }`; `views/probe-client.brain`
  with `comp/probe-chart.brain`, a `<meta client height="320">` canvas). The scaffold has no direct-submit form and no
  `<meta client>`, so the probes cover those paths. Second app: a copy of `examples/` without `invalid/`.
- Both are built with `brain build` and served with `brain start` (the scaffold with `--env development`, where its
  demo login lives; the password is read from the scaffold's own `.brain_env` and never printed).
- The browser (headless Chrome through `bench/lib/cdp.mjs`) talks to a counting proxy in front of `brain start`, so
  every POST is counted at the server side. JS is disabled with `Emulation.setScriptExecutionDisabled` before the
  first navigation. No-JS submissions are native (`HTMLFormElement.prototype.submit`, which also skips the browser's
  constraint validation, so the server's validation is what answers).
- Defences are raw HTTP requests straight to `brain start`.
- JS mode: a `window.__probe` value set before a click must survive the navigation with no `load` event, and the
  page reached must be the one the no-JS run reached (same URL after redirects, same `h1`). Submit buttons are clicked
  twice in a row; the proxy must see exactly one POST.
- Every supervisor is stopped with `brain stop` and then killed only by the pids spawned or read from
  `brain status --json`; Chrome is closed by its own handle. The run fails if a process is left.
- Output: `results/summary.json` and `results/summary.md` (homedir shown as `~`). Exit 1 on any failed check.

Run of 2026-10-13: binary sha256 `c34404783cc5`, Node v26.8.1, macOS arm64. **83/89 checks pass, 6 fail.**

## Results by page and form

| App | Page / form | no-JS | JS |
|---|---|---|---|
| scaffold | `/`, `/about`, `/posts`, `/posts/hello-world`, `/posts/second-post`, `/403`, `/probe-client` | PASS: 200, h1 + main text, every internal link loads its page | PASS: mounted, every link navigates on the client (probe kept, 0 loads, same page as no-JS) |
| scaffold | `/no-such-page` | PASS: 404 with the app's 404 view | — |
| scaffold | `/account` (signed out) | PASS: 303 → `/login?next=/account` | PASS: client guard lands on `/login?next=/account` |
| scaffold | `/login` lowering | PASS: `<form method="post">`, `name=` on fields, `_brain=xq8y/login` | — |
| scaffold | `/login` wrong credentials | **FAIL**: 422, but email empty and no "Wrong email or password" | PASS: 1 POST on a double click, message shown, no reload |
| scaffold | `/login` invalid input | **FAIL**: 422, but no value and no messages | — |
| scaffold | `/login` right credentials | **FAIL**: 303 back to `/login?next=/posts`, not to `/posts` (the session is set: `/account` shows the user) | PASS: 1 POST, client navigation to `/posts`, probe kept |
| scaffold | `/login` after a native 422 | — | **FAIL**: form() hydrates empty (same cause as the no-JS failure) |
| scaffold | `/probe-form` lowering | PASS: `<form method="post">`, names, `_brain=vx8w/send` | — |
| scaffold | `/probe-form` invalid | PASS: 422, values kept, "Required" / "Enter a valid email" / "Must be at least 18" | PASS: 0 POSTs, same 3 messages |
| scaffold | `/probe-form` server `{ errors }` | PASS: 422, "That name is taken" on the field | PASS: 1 POST, same message |
| scaffold | `/probe-form` unexpected failure | PASS: 422, "Something went wrong. Try again." (the thrown text is not shown) | **FAIL**: shows "Internal error" |
| scaffold | `/probe-form` `{ redirect: '/about' }` | PASS: 303 → `/about` | **FAIL**: stays on `/probe-form` (redirect ignored) |
| scaffold | `/probe-form` valid | PASS: 303 PRG to `/probe-form`, 1 POST | PASS: 1 POST on a double click, no error, no reload |
| scaffold | `/probe-form` after a native 422 | — | PASS: hydrates with the posted values and the 3 messages |
| scaffold | `/probe-client` `<meta client>` | PASS: placeholder 320 px, `#after` at 573 px | PASS: canvas 320 px mounted in place, `#after` still 573 px, CLS 0 |
| scaffold | defences (raw HTTP) | PASS: Origin evil → 403 · Sec-Fetch-Site cross-site → 403 · neither header → 403 · JSON → 415 · unknown hash / unknown function / no `_brain` → 400 · controls (Origin, Sec-Fetch-Site same-origin) → 303 | — |
| examples | `/`, `/dashboard` | PASS: 200, h1 + text, links load | PASS: links navigate on the client |
| examples | `/` to-do form (client-only) | PASS: stays a plain form, no POST lowering | PASS: adds the item, no reload |
| examples | `/no-such-page` | PASS: 404 with the bare fallback (no `views/404.brain`) | — |

## Failures and owners

1. **Scaffold login re-renders without the posted state (no-JS 422 and JS hydration after it).** Owner COMPILER #154
   (form() → `<form>` lowering). The login's `submit` is an arrow that calls `login(v)`; `server_forms`
   (`crates/brain-codegen/src/lib.rs:864`) resolves it to `login` for `$forms` and the hidden `_brain`, but the id
   never reaches `form()`, which only reads `o.submit.id` (`runtime/brain-form.js:129`), so neither the SSR re-render
   nor the hydration finds `xq8y/login` in the posted state. Direct `submit: send` works (probe-form). Fix: hand the
   resolved id to `form()` (for example an `id` option emitted by the compiler), or RUNTIME reads it from the form's
   `_brain` field.
2. **Scaffold login without JS does not go to `next`.** Owner TOOLCHAIN (scaffold template
   `crates/brain-cli/src/templates/new/views/login.brain`): `login` returns the user and the client navigates to `next`;
   without JS the server only sees no redirect, so the PRG returns to `/login?next=/posts`. Fix: `login` returns
   `{ redirect: next }` (from `ctx.url`, same-origin only) and the client follows it (needs failure 4).
3. **Unexpected server failure: JS shows "Internal error", no-JS "Something went wrong. Try again."** Owner RUNTIME
   client form(): `runtime/brain-form.js:234` shows the RPC's 500 body text (`server.mjs:635`); `callForm` uses the
   generic text (`server.mjs:658`). The two paths should show the same message.
4. **A `server:` submit that returns `{ redirect }` is ignored with JS.** Owner RUNTIME client form()
   (`runtime/brain-form.js:228`): the result is returned to the caller; no navigation, while no-JS gets a 303.

## Notes

- First runs of the day (binary `c16aeeb29b6f`) found `examples/` not building: the default social image template
  used `--space-6`, `--surface` and `--font`, missing from `examples/tokens.cssbrain`. The tokens were added in the
  working tree during this task; the final run builds the examples as shipped.
- `brain check` prints B0730 as a `warning`; INTERNALS §38 and `crates/brain-codegen/tests/html_first.rs` call it
  info.
- `brain build` lists `/probe-form` twice in the page table (one prerendered with HTML, one with `html 0`).
- An app without `views/404.brain` gets a bare 404 (`<h1>Not found</h1>`, no app shell, no link back).
