# guide: start — the working loop for any BRAINLLM task
Read this first in a new session. Every other guide assumes this loop.

## When
Any task in a BRAINLLM app: a new view, a bug, a refactor, a deploy. Run `brain guide <topic>` for the specific kind.

## The loop (which command, when)
1. **Learn the contract.** `brain spec` (or `brain spec --topic <files|script|markup|channels|css|a11y|seo|server|tests|cli>`).
   What is not in the spec does not exist; do not invent syntax from other frameworks.
2. **Learn the project.** `brain map --json` lists routes, components with props, modules, tests and tokens. Import only
   what it lists. To use a component read `brain card <name>` (its contract), never its body.
3. **Generate, do not hand-create.** `brain add view|comp|ui|layout|lib|channel|service|api|type|guard|server|collection|locale <name>`.
   The file arrives with the right `<meta>`, a11y-correct markup and, for components, a colocated test.
4. **Edit** the generated file. Prefer AST edits for structural changes:
   `brain edit add-prop|remove-prop|rename|move|extract …` and `brain split <comp> "<selector>" <NewName>`.
5. **Check and fix.** `brain fix` (applies every exact `edits`), then `brain check --json`. For each remaining
   diagnostic apply its `fix` literally. Repeat until the output is `[]`.
6. **Stuck?** A diagnostic with `"stuck": true` failed the same way three checks in a row: stop patching and run
   `brain explain <code>` (why the rule exists + a minimal correct example), then rewrite that part.
7. **Test.** `brain test` runs every `*.test.brain` in headless Chrome with an accessibility audit.
8. **Done gate.** `brain doctor` = check + tests + over-engineering + leaks. A task is done only when it exits 0.
   `brain doctor --fix` applies the mechanical fixes first.
9. **Look at it.** `brain run` (dev server, hot reload, port 5180) prints one `impact {…}` line per save.
   `brain shot <route> --text` gives the accessibility outline; without `--text` it writes PNGs (Unix + Chrome).
10. **Ship.** `brain build` (production: minify, CSP, SRI, sitemap, llms.txt) then `brain start` (`brain guide server`).

## Verify
```bash
brain check --json      # must print []
brain test              # all tests pass
brain doctor            # exit 0 = done
brain build             # production build succeeds
```

## Common mistakes
| Symptom | Code | Fix |
|---|---|---|
| importing a file that does not exist | B0119 | read `brain map --json`, import only listed files |
| creating a folder outside the layout | B0131 | use the folders `brain new` created; `brain add` picks the folder |
| using a feature the runtime does not ship yet | B0128 | use the alternative the `fix` names; `brain spec` hides planned parts |
| `class`, `this`, `while`, `switch` | B0101 B0102 B0110 | BrainScript is a closed subset: functions, signals, for-of |
| `items.push(x)` on a signal | B0113 | assign a new value: `items = [...items, x]` |
| `document`/`window` access | B0201 | `ref()` + `bind:ref={r}`, or `media()`/`visible()`/`online()` |
| `location.assign('/x')` | B0129 | `navigate('/x')` |
| a component over 150 lines | B1010 | run the `brain split` it suggests |

## Minimal correct example
```bash
brain new shop && cd shop
brain add view products
brain add comp product-card --props "product:obj"
brain fix && brain check --json   # → []
brain doctor                      # exit 0
```

## App documentation
`brain docs` writes `docs/app/` from the source: `index.md` (summary and links), `routes.md`, `components.md` (props,
events, slots, stores, a usage example), `data.md`, `backend.md`, `env.md` (key names only) and `seo.md`. Output is
deterministic, so diffs show real changes. Never edit those pages: write prose in `docs/app/notes/<page>.md` and it is
merged under "Notes". New apps set `"docs": true` in `brain.json`, so `brain build` refreshes the pages; `brain docs
--check` exits 1 when committed pages are stale and `brain doctor` warns. Over MCP: `brain_docs {check?}`.
```bash
brain docs                 # docs/app: 7 pages, N updated
brain docs --check         # exit 1 lists stale pages
brain docs --json          # the documentation model for an agent
```

## Other guides
`views` routing, layouts, guards · `data` load, server functions, resources, channels, caching · `forms` ·
`server` sessions, errors, env, deploy · `seo` search and AI answers · `a11y` · `webmcp` agent tools · `tests` ·
`i18n` languages · `performance`.
Over MCP (`brain mcp`) the same steps are `brain_spec`, `brain_map`, `brain_add`, `brain_check`, `brain_fix`,
`brain_test`, `brain_doctor`, and `brain_guide` for these playbooks.
