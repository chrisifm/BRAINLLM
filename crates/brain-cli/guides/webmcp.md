# guide: webmcp — expose what a page can do to browser AI agents

## When to expose a tool
Expose an action when an agent acting for the user would otherwise have to guess clicks: search, filter, add to cart,
book, subscribe, create a record. Do not expose:
- navigation (agents follow real `<a href>` links already) or reading (they read the HTML, the Markdown twin and llms.txt);
- admin, payment confirmation or deletion without a UI confirmation step — tools run with the page's permissions;
- the same action twice under different names.

Support is feature-detected (`document.modelContext`, or Chrome's origin-trial `navigator.modelContext`); where the
browser has none, nothing is registered and nothing breaks. WebMCP is not a standard yet.

## Steps (declarative: ships today)
1. Start from a real `<form>` that already works for people (`brain guide forms`).
2. Add `tool="snake_case_name"` and `tool-description="…"` to the `<form>`. The compiler emits the attribute names the
   current WebMCP draft uses.
3. Every field keeps its `<label>` (or kit `label=`): the label becomes the parameter description agents read.
   `tool-param-description="…"` on a field overrides it when the label is too short for an agent.
4. Names: snake_case verb + object (`search_products`, `add_to_cart`, `book_table`), unique per page.
5. Descriptions: one sentence that says what it does, what it needs and what it returns or changes, in the user's terms:
   "Search the catalog by text and return up to 20 matching products with price and stock."
   Say when NOT to use it if a near tool exists ("Use add_to_cart to buy; this only searches").
6. Keep the form usable without JS and without an agent: the agent fills the same fields a person does.

## Naming and description checklist
- Verb first, lower snake_case, no app prefix, no version numbers.
- Required inputs are required fields in the form (`!` in the `form()` schema); bounds go in the schema, not in prose.
- Mention units, formats and limits ("date as YYYY-MM-DD", "max 50").
- One tool = one user intent; split "search_or_create" into two.
- Destructive or paying actions: the description says it asks for confirmation, and the UI does ask.

## Verify
```bash
brain check --json                # []
brain build && brain preview      # inspect the built <form>: toolname / tooldescription attributes present
brain shot /shop --text           # every field has a name in the accessibility outline (that is what agents read)
```

## Common mistakes
| Mistake | Code | Fix |
|---|---|---|
| field without a label (no parameter description) | B0403 | `<Input label="Search text" …>` |
| CamelCase or duplicated tool name | — | snake_case, unique per page (SPEC rule; not machine-checked on `<form tool>`) |
| description that repeats the name ("Search tool") | — | say what it does, needs and returns |
| exposing delete/pay without a confirmation | — | keep the confirmation step in the UI |
| `<meta tools>` in a component today | B0128 | planned (#106): use a `<form tool>` |

## Planned (#106, not available yet)
Imperative tools in a component: `<meta tools="add_task: Add a task (text:str!)">` plus a function of the same name,
registered while the component is mounted (checks B1701 no function, B1702 bad or duplicate name, B1703 parameter
without a type; B0128 until it ships). The runtime registration already passes WebMCP's optional `title` and the
`annotations` hints (`readOnlyHint`, `consequentialHint`, `untrustedContentHint`) when given.

## Minimal correct example
```html
<script>
  let f = form({ q: 'str!<=80', max: 'num<=50' }, { submit: async (v) => navigate('/shop?q=' + encodeURIComponent(v.q)) })
</script>
<form tool="search_products" on:submit={f.submit}
  tool-description="Search the catalog by text and list matching products with price and stock.">
  <Input label="Product name or keyword" bind:value={f.v.q} error={f.err.q} />
  <Input label="Maximum results (up to 50)" type="number" bind:value={f.v.max} error={f.err.max} />
  <Button type="submit" busy={f.busy}>Search</Button>
</form>
```
