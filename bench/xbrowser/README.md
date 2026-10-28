# Cross-browser matrix (WebKit · Firefox · Chromium)

The team develops Chrome first. This benchmark checks that Safari's engine (WebKit) and Firefox still work, with
Chromium as the control. It covers three real apps plus a probe app with a 1000-row list, in both modes.

## Method

`run.mjs` reproduces everything:

1. **Apps.** It copies `~/tmp/brainllm/apps/{tasks,landing,my-project}` into a scratch dir (the originals are never
   touched). It creates `biglist` with `brain new` and gives it one view: a keyed `{#each}`, filled with 1000 rows on
   click, plus buttons for reverse, swap-ends and remove-first. It then runs `rm -rf .brain/cache dist && brain build`
   in each copy. If the current binary fails a build and `BRAIN_BUILD` points to a frozen binary, that binary is used
   instead, and the cell records the failure and the frozen binary's sha256. Routes come from `brain map --json`;
   catch-all (`*`, the 404 view) and parametrized (`/posts/:id`) routes are not URLs and are not loaded. `biglist`
   loads only its own view `/` (the rest of the scaffold it is made from is not its subject). Skipped routes are listed
   in the summary.
2. **Servers.** For **dev** it runs `brain run --port P` and stops it by PID. For **build** it runs an in-process static
   server that copies the generated nginx snippet (INTERNALS §10): MIME types by extension (including
   `.brain/.jsbrain/.cssbrain`), clean URLs through `index.brain`/`index.html`, a 404 for anything else, and
   `immutable` caching on `/_b/`. The run checks that chunks are served as `text/javascript`.
3. **Per cell (app × mode × browser).** Each cell uses a fresh Playwright context. An init script records unhandled
   rejections, `history.pushState` calls, `document.startViewTransition` calls and their outcome, Navigation API
   `navigatesuccess` events and `brain:hmr` events.
   - **Routes.** Every route from `brain map --json` is loaded. A route passes when an `<h1>` mounts in the view.
     Console errors, page errors, failed requests, HTTP ≥ 400 and unhandled rejections are all collected; the target
     is 0. `favicon.ico` 404s are logged as notes, not counted as errors, because none of the apps has a favicon.
   - **Interactions.** These are read from the `.brain` sources:
     - tasks: type and submit, ignoring an empty submit, checkbox toggle with the "N left" count, the
       Done/Open/All filters (a keyed re-insert in the middle), remove, and a counter with a derived value and
       `class:hot`.
     - landing: nested keyed lists, then email validation and the `{#if}` thanks message.
     - my-project: `bind:value` on a textarea and an async copy handler.
     - biglist: fill, reverse (checks that the node is reused, not recreated), swap-ends and remove-first.

     A random `window.__probe` token proves that no step reloaded the document.
   - **First interaction.** Each app is loaded fresh and touched once the way a person would: a real click that
     holds the mouse button for 150 ms, and text typed before the island resumes. Build pages resume islands on the
     first event, so this is where builds can break.
   - **Client navigation.**
     - tasks: `/about` → `/` (the view's own link), then `/` → `/about` (an injected link), back and forward. It
       checks the `<h1>`, the URL and that the document was not reloaded. It also checks that the view still
       works after navigating, by adding a task.
     - Single-route apps: a link to `/?xb=1`. A single-route build ships no client router by design, so that
       check is n/a.
     - The router path used is recorded: Navigation API, or the History API fallback (seen as `pushState` calls).
       So is the View Transition outcome: started, finished or skipped.
   - **Fallbacks.** The run records whether `navigation`, `document.startViewTransition`,
     `Element.prototype.moveBefore`, `scheduler.yield` and `scheduler.postTask` exist.
   - **HMR (dev; tasks and landing, all three browsers).** The run edits one `<style>` line of
     `comp/counter.brain`, then one `<script>` line (`n * 2` → `n * 3`; the `n` sig must keep its value). It then
     restores the file in a single save, which changes the style and the script together. Each step must apply with
     no reload.
4. **Retry and exit code.** A cell with any failure is run once more (machine load can fail a navigation step); the
   cell keeps `retried` with the first attempt's failures and whether the retry passed, and the summary lists every
   retried cell. The script exits 1 when any cell still fails after its retry (or its app does not build), else 0.
5. **Output.** Results go to `results/summary.json` (every cell, step and error) and `results/summary.md`.

## Reproduce

```sh
# Playwright is a test tool only. Install it in a scratch dir (or run `npm run setup` in this folder):
mkdir -p ~/tmp/brainllm-xbrowser && cd ~/tmp/brainllm-xbrowser
npm init -y && npm i playwright@1.64.0 && npx playwright install webkit firefox chromium
cd <repo> && node bench/xbrowser/run.mjs            # optional: --browsers webkit --apps tasks --modes build
# BRAIN=<binary> XB_WORK=<scratch> XB_APPS=<source apps> BRAIN_BUILD=<frozen binary for build mode>
```

## Versions (latest run: 2026-10-13, task #286)

| Item | Version |
|---|---|
| Playwright | 1.64.0 |
| WebKit | 27.2 |
| Firefox | 157.0 |
| Chromium | 156.0.8078.4 (headless shell) |
| `brain` binary | release build of the 2026-10-13 tree, sha256 `e9453256ff44` (it built every app; no frozen fallback) |
| Node | v26.8.1 |

## Results

24/24 cells pass, with no retry: 0 failures, 0 errors. `biglist` is tested again (in the previous run its route list
held the scaffold's catch-all `*`, so the page was never loaded). Two earlier harness assumptions were fixed in this
run: route discovery skips catch-all and parametrized routes, and selectors no longer use the apps' class names,
which `brain build` now renames (#260). The tables below are `results/summary.md`; "interactions OK" includes the
first-interaction checks; HMR shows style / script / style and script in one save.

| app | mode | browser | routes OK | interactions OK | console/page/net errors | nav without reload (router) | Navigation API | View Transitions | moveBefore | scheduler.yield | HMR style / script / both-in-one-save |
|---|---|---|---|---|---|---|---|---|---|---|---|
| tasks | dev | webkit 27.2 | 2/2 | 9/9 | 0 | yes (History API (pushState)) | yes | yes | no | no | ok / ok / ok |
| tasks | dev | firefox 157.0 | 2/2 | 9/9 | 0 | yes (History API (pushState)) | yes | yes | yes | yes | ok / ok / ok |
| tasks | dev | chromium 156.0.8078.4 | 2/2 | 9/9 | 0 | yes (History API (pushState)) | yes | yes | yes | yes | ok / ok / ok |
| tasks | build | webkit 27.2 | 2/2 | 9/9 | 0 | yes (Navigation API) | yes | yes (4 started, 4 finished, 0 skipped) | no | no | n/a |
| tasks | build | firefox 157.0 | 2/2 | 9/9 | 0 | yes (Navigation API) | yes | yes (4 started, 4 finished, 0 skipped) | yes | yes | n/a |
| tasks | build | chromium 156.0.8078.4 | 2/2 | 9/9 | 0 | yes (Navigation API) | yes | yes (4 started, 4 finished, 0 skipped) | yes | yes | n/a |
| landing | dev | webkit 27.2 | 1/1 | 6/6 | 0 | yes (History API (pushState)) | yes | yes | no | no | ok / ok / ok |
| landing | dev | firefox 157.0 | 1/1 | 6/6 | 0 | yes (History API (pushState)) | yes | yes | yes | yes | ok / ok / ok |
| landing | dev | chromium 156.0.8078.4 | 1/1 | 6/6 | 0 | yes (History API (pushState)) | yes | yes | yes | yes | ok / ok / ok |
| landing | build | webkit 27.2 | 1/1 | 6/6 | 0 | n/a (none shipped (single route)) | yes | yes (n/a) | no | no | n/a |
| landing | build | firefox 157.0 | 1/1 | 6/6 | 0 | n/a (none shipped (single route)) | yes | yes (n/a) | yes | yes | n/a |
| landing | build | chromium 156.0.8078.4 | 1/1 | 6/6 | 0 | n/a (none shipped (single route)) | yes | yes (n/a) | yes | yes | n/a |
| my-project | dev | webkit 27.2 | 1/1 | 4/4 | 0 | yes (History API (pushState)) | yes | yes | no | no | not run |
| my-project | dev | firefox 157.0 | 1/1 | 4/4 | 0 | yes (History API (pushState)) | yes | yes | yes | yes | not run |
| my-project | dev | chromium 156.0.8078.4 | 1/1 | 4/4 | 0 | yes (History API (pushState)) | yes | yes | yes | yes | not run |
| my-project | build | webkit 27.2 | 1/1 | 4/4 | 0 | n/a (none shipped (single route)) | yes | yes (n/a) | no | no | n/a |
| my-project | build | firefox 157.0 | 1/1 | 4/4 | 0 | n/a (none shipped (single route)) | yes | yes (n/a) | yes | yes | n/a |
| my-project | build | chromium 156.0.8078.4 | 1/1 | 4/4 | 0 | n/a (none shipped (single route)) | yes | yes (n/a) | yes | yes | n/a |
| biglist | dev | webkit 27.2 | 1/1 | 5/5 | 0 | yes (History API (pushState)) | yes | yes | no | no | not run |
| biglist | dev | firefox 157.0 | 1/1 | 5/5 | 0 | yes (History API (pushState)) | yes | yes | yes | yes | not run |
| biglist | dev | chromium 156.0.8078.4 | 1/1 | 5/5 | 0 | yes (History API (pushState)) | yes | yes | yes | yes | not run |
| biglist | build | webkit 27.2 | 1/1 | 5/5 | 0 | yes (Navigation API) | yes | yes (3 started, 3 finished, 0 skipped) | no | no | n/a |
| biglist | build | firefox 157.0 | 1/1 | 5/5 | 0 | yes (Navigation API) | yes | yes (3 started, 3 finished, 0 skipped) | yes | yes | n/a |
| biglist | build | chromium 156.0.8078.4 | 1/1 | 5/5 | 0 | yes (Navigation API) | yes | yes (3 started, 3 finished, 0 skipped) | yes | yes | n/a |
| mode | browser | fill: rows right after click → final | reverse keeps node identity | reverse keeps focus |
|---|---|---|---|---|
| dev | webkit | 1000 → 1000 rows in 0 frames, 6 ms | yes | no |
| dev | firefox | 1000 → 1000 rows in 0 frames, 32 ms | yes | yes |
| dev | chromium | 1000 → 1000 rows in 0 frames, 9 ms | yes | yes |
| build | webkit | 1000 → 1000 rows in 0 frames, 5 ms | yes | no |
| build | firefox | 1000 → 1000 rows in 0 frames, 7 ms | yes | yes |
| build | chromium | 1000 → 1000 rows in 0 frames, 4 ms | yes | yes |

The findings below come from the earlier run of 2026-10-13 (binary `f32f78f8ab44`); the run above no longer
reproduces them (every interaction and HMR step passes).

## Findings

No failure is specific to WebKit or Firefox. Errors are 0 in all 24 cells, and routes, dev mode, the
Navigation API, View Transitions and the list fallbacks all work in all three engines. Every failure is one of
three bugs that Chromium also has. The browser only changes how likely a person is to hit them.

1. **RUNTIME: island resume swaps the DOM under the user** (`runtime/brain.js:390-421`, `resume`/`hydrate`). On
   the first event, `resume` mounts a fresh tree and `el.replaceWith(f)`s the prerendered one. This breaks the
   first interaction in two ways:
   - **Typed text is lost.** The value typed into the server `<input>` is not carried over, and the replayed
     `input` event reads the new, empty input. This happens in all 3 browsers. It causes the WebKit tasks failures
     (Playwright `fill` lands before the resume in WebKit) and the failure to add a task after client navigation
     in every browser.
   - **The first click is lost.** `focusin` (on mousedown, Chromium and Firefox) starts the resume, and the
     button is replaced before mouseup, so no `click` fires. This affects Chromium and Firefox with a 150 ms
     press, and Firefox even with Playwright's fast click. WebKit escapes it only because macOS buttons do not
     take focus on click.

   A related problem: the replayed event is synthetic (`isTrusted=false`), so APIs that need user activation see
   no gesture.

   Repro: build `tasks`, serve `dist/` with `immutable` on `/_b/`, open `/` and run
   `i=document.querySelector('#nt-1');i.focus();i.value='Alpha';i.dispatchEvent(new InputEvent('input',{bubbles:true}))`.
   After 300 ms, `#nt-1` is a new node with `value === ''`. For the click, open the landing build, press "Demo
   clicks" for 150 ms, and see that the counter is still 0.
2. **TOOLCHAIN: a save that changes `<style>` and `<script>` together never swaps the CSS**
   (`crates/brain-serve/src/client.js:63`). The client runs `if (m.js) return hot(...)` before
   `if (m.css) swapCss(...)`, so when the server sends `{js:true, css:true}` the old stylesheet stays. This happens
   in all 3 browsers. Repro: in dev, add `letter-spacing: 3px;` to the counter's `button` rule and save, then
   remove it and change `n * 2` to `n * 3` in the same save. The text updates, but `letter-spacing` stays `3px`
   until a reload.
3. **Not a bug, but worth knowing.**
   - A single-route build ships no client router, so a link is a full page load (by design).
   - `brain run` always uses the dev client's own History router, never the Navigation API or View Transitions.
     Those paths are only exercised in build.
   - The 1000-row fill took under 5 ms, so the chunked-render slicing never had to yield at this size.
   - The style edit in the HMR test uses `letter-spacing`, because a literal color is a check error (B0304) and
     would block the HMR update.
