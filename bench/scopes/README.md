# Scopes bench — component encapsulation (12 rules, dev and prod)

A component must behave like a sealed box in light DOM: its state, styles, ids, events, errors and names stay inside
it unless the author opens a documented door (props, `emit`, `<slot>`, `provide`/`use`, a store in `channels/`, a
custom property). This bench checks 12 such rules in real headless Chrome, against the same app served two ways:

- **dev**: `brain run --port 5196` (per-module dev server, HMR runtime, `brain-err.js`);
- **prod**: `brain build` + a static host for `dist/` (the app has no server functions, so `dist/` is static; the host
  serves clean URLs and never serves `dist/_brain/`).

Background: `docs/INTERNALS.md` §6 (runtime, hydration), §7 (CSS: `@scope ([data-b]) to ([data-b])`), §8
(encapsulation in 3 layers, encapsulated IDs), §41 (state scopes, precise updates), §42 (every component is isolated
by default); `SPEC.md` "Communication" and the error paragraph (a failing click handler keeps the component).

## Run

```sh
cargo build -p brain-cli
node bench/scopes/run.mjs                 # both modes; 10–14 s
node bench/scopes/run.mjs --modes dev     # one mode
node bench/scopes/run.mjs --no-build      # prod against the existing dist/
```

`BRAIN` overrides the binary (default `target/debug/brain`), `BRAIN_CHROME` the Chrome binary (see `bench/lib/cdp.mjs`).
The runner first requires `brain check --json` on the app to report no errors, then starts the dev server and Chrome
as its own children and kills them by their own handles on exit (nothing is left running).

**Output:** a table on stdout, `results/summary.json` and `results/summary.md` (committed), and
`results/raw/<mode>.json` with every computed style, mutation record and console line (gitignored).
**Exit code:** `0` every rule passes in every mode · `1` any rule fails (use it as a regression gate) · `2` the bench
could not run (binary missing, `brain check` errors, build failed, a view did not render).

## The app

`app/` is a regular BRAINLLM app (`brain check` clean). `/scopes` hosts one section per rule; `/errors` hosts a
component that throws while it is created. Colors come from `tokens.cssbrain`: accent `rgb(0, 98, 196)` (view),
ok `rgb(22, 121, 74)` (CardA), bad `rgb(196, 43, 28)` (CardB, SlotHost, escape probe), muted `rgb(85, 89, 94)`
(MultiRoot). Each probed element carries `data-probe="<owner>-<what>"`.

## The 12 rules

| # | Rule | What it means | How it is measured |
|---|---|---|---|
| 1 | Local state per instance | Each instance of a component owns its `sig` values. | Three `Counter`s; click the first 3 times; expect `3/0/0`. |
| 2 | Context per subtree | `use(Key)` reads the nearest `provide(Key)` above it, including through slots; outside any provider it gets the key's default. | `ToneProvider` red with a nested blue provider, a green provider and a bare reader. Read every `.tone`, click "Shift red", read again: only the red subtree becomes `red+`; the inner stays `blue-inner`, green stays `green`, the bare reader shows `none`. |
| 3 | Global store shared, field-precise | A `channels/` store is one instance for the whole app, and writing one field updates only the nodes bound to that field. | `StoreA` (+1) and `StoreB` (+100), far apart in the tree, both show `101`; a `MutationObserver` around the `other` field write must only see nodes inside `#far-other`. |
| 4 | Precise updates | A write mutates only the nodes bound to it. | One counter click mutates ≤ 3 nodes, all inside `#c2`. In a keyed list of 200 rows, bumping row 57 mutates nodes of row 57 only. |
| 5 | CSS same-name isolation | Equal selectors (`.title`, `.btn`, `h2`, `p`, `span`, `em`) in a view, a `comp/`, a `ui/` and a multi-root component never affect each other, in either direction. | `getComputedStyle` color, font-size and border-top of 17 probes match their own author's rule; the view's `span { border-top: 7px }` reaches none of the ~430 component spans; two `CardB` instances compute the same. |
| 6 | IDs per instance | Static `id`s and their references (`for`, `aria-describedby`, `href="#…"`) are rewritten per instance; the view's own ids stay as written. | Two `LabeledField`s: no duplicate ids in the document, each `<label>.control` is its own input, `aria-describedby` points at its own hint, the anchor targets its own input; `#r1`, `#c1`, `#anchor-target`, `#lf1` still exist. |
| 7 | Events reach only the direct parent | `emit` calls the direct parent's `on:` handler only; it does not bubble to grandparents or through the DOM. | `MidRelay` hosts leaves `a` and `b`; the view hosts a sibling leaf. Leaf `a` → only the relay counts; leaf `b` → the relay counts and re-emits `b-relayed` to the view; the sibling leaf → only the view's `onSib`. A capture listener for a DOM `ping` event must see 0. |
| 8 | Slots keep the author's scope | Content passed through `<slot>` is styled and bound by the component that wrote it, not by the host. | CardA puts a `p.title` into CardB and a `p.slotted` into SlotHost; the view puts a `p.title` into CardB. Each must compute its author's styles (CardA ok 17px 3px / underline; view accent 31px) and show its author's state (`parent-state`), while the host's own span shows `child-state`. |
| 9 | Error isolation | A failure stays in the component where it happened (§42). A throwing click handler keeps the component (SPEC: it is reported and a toast is shown). A throwing `drv` or a throw during setup replaces only that component with its `<meta fallback>`. Neighbours keep working. | `Boom`: click "Throw in handler" → `#boom1` must still be the component with `clicks = 1`; click "Throw in drv" on `#boom2` → it shows `Boom failed`, `<main>` keeps its 11 children and the neighbour counters keep counting. `/errors`: `SetupThrow` shows `Setup failed` between two counters that both count. |
| 10 | CSS cannot escape its component | Selectors that try to leave the component (`:scope ~ p`, `main`, `body` inside `@media`, `:where(html) p`, `.a body`, `::selection`) only match inside it. | `EscapeProbe` declares all of them at 41px / bad color; `body`, the view `<main>` and the view paragraph after the probe must keep their own values (16px, 16px, 31px accent). |
| 11 | JS same names stay module-local | Two components with the same top-level names (`count`, `items`, `toggle`, `handleClick`, `open`) do not share them, and nothing leaks to `window`. | `JsA` ×2 and `JsB`: after one click each, counts are `1/0/1010` and `items` are each component's own; none of 26 app names appear on `window` at load or after all interactions. |
| 12 | Custom property per instance | `style:--x={v}` sets a token for that instance's subtree only. | Two `TintBox`es: the plain one computes the global accent, the custom one `rgb(10, 20, 30)`; the view title keeps the accent. |

Owners when a rule fails: COMPILER (scoping and codegen: `crates/brain-codegen`, `brain-css`) for 5, 6, 8, 10, 11;
RUNTIME (`runtime/`) for 1, 2, 3, 4, 7, 9, 12; BUILD (`brain-pack`, `brain-min`) when a rule passes in dev and fails
in prod.

## Current results

Binary sha256 `6b2f316e6c56` (2026-10-13): **dev 10/12 · prod 10/12 — FAIL** (exit 1). Total runtime 10–14 s over 3 runs
(build 0.7–2.9 s, dev 4.9–6.2 s, prod 4.4–6.5 s); the verdicts were identical in all 3 runs. The full table is in `results/summary.md`.

| # | Rule | dev | prod |
|---|---|---|---|
| 1 | Local state per instance | pass | pass |
| 2 | Context per subtree | pass | pass |
| 3 | Global store shared, field-precise | pass | pass |
| 4 | Precise updates | pass | pass |
| 5 | CSS same-name isolation | pass | pass |
| 6 | IDs per instance | pass | pass |
| 7 | Events reach only the direct parent | pass | pass |
| 8 | Slots keep the author's scope | FAIL | FAIL |
| 9 | Error isolation | FAIL | FAIL |
| 10 | CSS cannot escape its component | pass | pass |
| 11 | JS same names stay module-local | pass | pass |
| 12 | Custom property per instance | pass | pass |

- **8 (COMPILER):** every slotted node computes the host's styles: CardA's `p.title` inside CardB is
  `rgb(196, 43, 28)` 23px 5px (want CardA's `rgb(22, 121, 74)` 17px 3px); CardA's `p.slotted` inside SlotHost is red
  13px without underline; the view's `p.title` inside CardB is red 23px. Cause: slotted nodes carry no marker of
  their author, so the host's `@scope ([data-b="host"]) to ([data-b])` reaches them and the author's scope is cut at
  the host's `data-b`. Slot text and state are correct (`parent-state`). The expected semantics (author's scope,
  as in Svelte, Vue and Shadow DOM) are not yet written in INTERNALS §7/§8.
- **9 (RUNTIME):** a throw in a click handler replaces the whole `Boom` with its fallback `Boom failed` instead of
  keeping it and showing a toast (SPEC error paragraph, INTERNALS §42 "Event handlers do not destroy the
  component"). The `drv` throw, the setup throw and every neighbour behave as specified.
