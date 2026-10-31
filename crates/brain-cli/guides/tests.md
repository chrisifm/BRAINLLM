# guide: tests — colocated *.test.brain in real Chrome

## When
Every stateful component (`comp/`) and every view with logic: a form, a filter, a list with states. `brain add comp`
already creates the test file; fill it before calling the task done.

## Steps
1. File next to the component: `comp/user-card.brain` → `comp/user-card.test.brain` (`brain add test user-card`).
2. Write one `test(name, async () => {…})` per behavior, named as a sentence a person would read in a report.
3. Arrange: `await mount(Comp, props)`; swap services with `await mount(Comp, props, { provide: [[Api, fakeApi]] })`
   (`Api` is the key exported from `channels/` or `services/`). Pass slot content as text:
   `await mount(LinkButton, { href: '/next' }, { slot: 'Continue', slots: { hint: 'Opens the next step' } })`.
4. Act: `await click(sel)`, `await type(sel, text)`.
5. Assert: `text(sel)`, `attr(sel, name)`, `hasClass(sel, c)`, `count(sel)` with
   `expect(x).toBe(y) | toEqual(y) | toBeTruthy()`.
6. Cover the states, not only the happy path: empty list, validation errors, server error, disabled while busy.
7. Run `brain test` (or `brain test comp/user-card.test.brain`). Each test also gets an accessibility audit of the
   mounted tree; `--heap` adds the leak watch that `brain doctor` always runs.

## What to test (in this order)
1. What the user sees for given props (rendering and text).
2. What happens on interaction (click, type, submit).
3. Every state the component promises: loading, empty, error, disabled.
4. Events to the parent: render a tiny parent in the test only if the event's effect is visible; otherwise test the
   visible result.

## Verify
```bash
brain test                  # all files; non-zero exit on any failure
brain doctor                # tests + check + over-engineering + leaks; the done gate
```
Over MCP: `brain_test`, then `brain_doctor`.

## Common mistakes
| Mistake | Code | Fix |
|---|---|---|
| test reaches into `document` | B0201 | use `text`/`attr`/`count`/`click` helpers with selectors |
| component leaks after unmount | B0601 | return a cleanup from `fx`/`mounted` |
| global listener or timer without cleanup | B0602 · B0906 | create it inside `fx` and return its cleanup |
| list grows from a timer with no input | B0907 | cap the list or drive it by user input |
| hitting the real network in a test | — | `provide: [[Api, fakeApi]]`, or `brain run --mock` for manual checks |
| control whose text comes from its `<slot />` mounted without it | B0406 | `mount(Comp, props, { slot: 'Text' })` |
| testing internals (signal values) | — | assert what the DOM shows |
| one giant test | — | one behavior per `test()` |

## Minimal correct example
```html
<!-- comp/todo-list.test.brain -->
<script>
  import TodoList from './todo-list.brain'

  test('shows the empty message when there are no tasks', async () => {
    await mount(TodoList, { items: [] })
    expect(text('li')).toBe('No tasks.')
  })

  test('adds a task typed by the user', async () => {
    await mount(TodoList, { items: [] })
    await type('#new', 'Buy milk')
    await click('button[type=submit]')
    expect(count('li')).toBe(1)
    expect(text('li')).toBe('Buy milk')
  })
</script>
```
