# Precision bench — one write, exactly its nodes (QA #172, INTERNALS §41.3)

There is no virtual DOM: the unit of update is a DOM node. This bench checks that one write mutates **exactly** the
text and attribute nodes bound to the written field, on pages with 1 000 components. It runs in real headless Chrome
against `app/`, in dev (`brain run --port 5199`) and prod (`brain build` + a static host: the fault server from
`bench/http/server.mjs` in static mode).

## Method

A `MutationObserver` on the whole document (`childList`, `characterData`, `attributes`, subtree) records one action:
the click, two animation frames and 80 ms. Each mutation becomes a label:

- `cell-500>span.v#text`: a text node changed (owner = nearest `data-k`, then the parent tag and class);
- `cell-500>span.v@data-n`: an attribute changed;
- `+row-2` / `-row-2`: a keyed node added or removed (a move is one of each); other added or removed nodes are
  labelled `owner>tag+child`.

The label multiset must equal the expected one: any **extra** label (a node touched that is not bound to the field)
or **missing** label fails the case.

Production hydrates an island on its first interaction by rendering it again and swapping it in (§38). Before every
measured write, the runner clicks the inert `<h1>` to trigger that, waits for the island's `b/state` script to go,
and records it apart as **first interaction** (informational, not gated). Those 4 recorded swap mutations also show
that the observer catches mutations outside the bound set.

## Cases

| # | Case | Expected |
|---|---|---|
| P1 | local `sig` in one of 1 000 `Cell` components (text + `data-n` attribute) | 2 |
| P2 | global store (`channels/shop.jsbrain`): write `a`; 10 readers of `a`, 10 of `b`, 1 000 cells | 10 (the `a` readers) |
| P3 | write an equal value (`shop.a = shop.a`; `Object.is`) | 0 |
| P4 | write `b` | 10 (the `b` readers) |
| P5 | group state: two `GroupRoot`s each `provide(Cart, store({ x, y }))`; write `x` in one (5 `x` + 5 `y` readers per group) | 5 |
| P6 | 1 000-row keyed list: change one row's `hits` | 1 |
| P7 | swap rows 2 and 999 | 4 (two moves), and the resulting order |
| P8 | remove row 500 | 1 |

P5 depends on `store()` inside `provide()` (RUNTIME #169 / COMPILER #170). If its files do not compile, they are
left out of the working copy (`.work/app`) and the case reports **BLOCKED** with the diagnostic.

## Run

```sh
cargo build -p brain-cli
node bench/precision/run.mjs               # both modes, 60–125 s
node bench/precision/run.mjs --modes dev
```

`BRAIN` / `BRAIN_CHROME` as in the other benches. **Output:** stdout table, `results/summary.{json,md}` (committed),
`results/raw/<mode>.json` with every label (gitignored). **Exit:** `0` every runnable case exact · `1` any extra or
missing mutation (regression gate) · `2` the bench could not run.

## Current results

Binary sha256 `d26b4bd21f64` (2026-10-13): **8/8 exact in dev and prod — PASS**. That includes P5: `store()` in
`provide()` already compiles and is field-precise per subtree.

First interaction in production: 4 mutations per view (the swap of `<main>` and its state script, ~100 ms on a
1 000-component view); 0 in dev.
