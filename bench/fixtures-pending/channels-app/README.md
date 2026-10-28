# channels-app — one case per channel (INTERNALS §12)

Valid app: once #30 (COMPILER) and #31 (RUNTIME) are done, `brain check --json` must give `[]` and `brain dev` must
show it working. It lives outside `examples/` because today the checker does not know `~`, `emits`, `expose`, `provide`,
`use`, `key`, `store` or `bus` yet, and it would break the "examples/ clean" test of `brain-check`.

| # | Channel | Where |
|---|---|---|
| 1 | Props | `ui/price.brain` (`amount`, `currency`) ← `comp/product-card.brain` |
| 2 | `~` model | `comp/qty-input.brain` (`value:num~`) ← `bind:value={qty}` in `product-card` |
| 3 | Events | `product-card` `emits="add:obj"` → `on:add={add}` in `views/shop.brain` |
| 4 | Slots | `ui/panel.brain` (`<slot />` and `<slot name="footer" />`) |
| 5 | Exposed API | `comp/confirm.brain` `expose="open"` → `bind:api={confirm}` + `confirm.open()` in `cart-summary` |
| 6 | Context | `provide(Theme, theme)` in `shop` → `use(Theme)` in `cart-summary`; key in `channels/theme.jsbrain` |
| 7 | Store | `channels/cart.jsbrain` (`sync: 'storage'`) ← `shop` writes, `cart-summary` reads and empties it |
| 8 | Bus | `channels/toast.jsbrain` ← `cart-summary` sends, `comp/toaster.brain` listens (cleanup in `fx`) |

The invalid cases B0801–B0807 are in `invalid/` (table at the end).

## Invalid cases B0801–B0807 (`invalid/`)

| Code | Case | Proposed level |
|---|---|---|
| B0801 | `invalid/channels/b0801-module-state.jsbrain` (mutable `let count` at module level) | error |
| B0802 | `invalid/comp/b0802-emit-undeclared.brain` (`emit('select')` with `emits="close"`) | error |
| B0803 | `invalid/comp/b0803-on-undeclared.brain` (`on:remove` on `ProductCard`, which declares `add`) | error |
| B0804 | `invalid/comp/b0804-api-undeclared.brain` (`dlg.close()` while `Confirm` exposes `open`) | error |
| B0805 | `invalid/comp/b0805-import-internal.brain` (`import { qty } from …product-card.brain`) | error |
| B0806 | `invalid/comp/b0806-store-outside-channels.brain` (`store()` in a component) | error |
| B0807 | `invalid/comp/b0807-use-without-provide.brain` (`use(Lang)` without `provide`) | warning (static) |

The `file` of each `.expected.json` is relative to `bench/fixtures-pending/channels-app/` as the app root
(`channels-app/invalid/…`); it must be adjusted when they move to `examples/invalid/`.

Status with the current checker (2026-10-13, before #30/#31): `brain check --json bench/fixtures-pending/channels-app`
only gives diagnostics for primitives not implemented yet (`use`, `provide`, `store` → B0111; `bind:api`, `bind:value`
on a component → B0205; `value:num~` → B0004 and its consequence B0206). None comes from another rule: zero real
collateral hits. Definition of done for #30/#31: the valid part gives `[]` and each invalid case gives exactly its
`.expected.json`.
