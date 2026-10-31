# guide: webmcp — expose what a page can do to browser AI agents

## When to expose a tool
Expose an action when an agent acting for the user would otherwise have to guess clicks: search, filter, add to cart,
book, subscribe, create a record. Do not expose:
- navigation (agents follow real `<a href>` links already) or reading (they read the HTML, the Markdown twin and llms.txt);
- admin, payment confirmation or deletion without a UI confirmation step — tools run with the page's permissions;
- the same action twice under different names.

Support is feature-detected (`document.modelContext`, or Chrome's origin-trial `navigator.modelContext`); where the
browser has none, nothing is registered and nothing breaks. WebMCP is not a standard yet.

## Steps (declarative: a form people already use)
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

## Steps (imperative: an action that is not a form)
Use `<meta tools>` when the action has no form a person fills (clear completed tasks, add the current item to the
cart) or needs typed values a form does not hold.
1. Declare the tools in the component: `<meta tools="add_task: Add a task (text:str!, due:date); clear_done: Remove
   completed tasks">`. Same naming and description rules as above; `!` marks a required parameter.
2. Write a function with the same name that destructures exactly those parameters and returns what the agent should
   read: `const add_task = async ({ text, due }) => { … return items.length }`.
3. The tools are registered while the component is mounted and removed when it unmounts. Arguments are checked
   against the declared types first: a missing required one or a wrong type answers `text: Required` (or the type's
   message) and the function does not run. A thrown error answers `Error: <message>`.
4. Mount a component with tools once per page: the browser refuses a second registration of the same name, so a
   second instance exposes nothing (it still renders).

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
| `<meta tools>` entry without a function of that name | B1701 | write `const add_task = async ({ text }) => …` |
| tool parameter without a type | B1703 | `text:str` (types `str num bool list obj email url date any`) |
| function destructures other parameters than the tool declares | B1704 | make `(text:str!, due:date)` and `({ text, due })` list the same names |

## Imperative example
```html
<meta tools="add_task: Add a task to the list and return how many there are (text:str!, due:date); clear_done: Remove completed tasks">
<script>
  let items = sig([])
  const add_task = async ({ text, due }) => { items = [...items, { id: items.length + 1, text, due, done: false }]; return items.length }
  const clear_done = () => { items = items.filter(i => !i.done); return 'Removed completed tasks' }
</script>
<p>{items.length} tasks</p>
```

## Minimal correct example (declarative)
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
