<script>
  let { title = 'To do' } = $props()
  let items = $state([])
  let text = $state('')
  let filter = $state('all')
  let nextId = 1

  let visible = $derived(
    filter === 'done' ? items.filter(it => it.done)
    : filter === 'open' ? items.filter(it => !it.done)
    : items
  )
  let left = $derived(items.filter(it => !it.done).length)

  const add = (e) => {
    e.preventDefault()
    const t = text.trim()
    if (t === '') return
    items = [...items, { id: nextId++, text: t, done: false }]
    text = ''
  }
  const toggle = (id) => {
    items = items.map(it => it.id === id ? { ...it, done: !it.done } : it)
  }
  const remove = (id) => {
    items = items.filter(it => it.id !== id)
  }
</script>

<section class="todo">
  <h2>{title}</h2>
  <form onsubmit={add}>
    <input bind:value={text} placeholder="What needs doing?" aria-label="New task" />
    <button disabled={text.trim() === ''}>Add</button>
  </form>

  <ul>
    {#each visible as it (it.id)}
      <li class:done={it.done}>
        <input type="checkbox" checked={it.done} onchange={() => toggle(it.id)} aria-label={`Mark ${it.text}`} />
        <span>{it.text}</span>
        <button onclick={() => remove(it.id)} aria-label="Delete">×</button>
      </li>
    {:else}
      <li class="empty">Nothing here.</li>
    {/each}
  </ul>

  <footer>
    <span>{left} left</span>
    <button class:on={filter === 'all'} onclick={() => filter = 'all'}>All</button>
    <button class:on={filter === 'open'} onclick={() => filter = 'open'}>Open</button>
    <button class:on={filter === 'done'} onclick={() => filter = 'done'}>Done</button>
  </footer>
</section>

<style>
  .todo { display: grid; gap: var(--space-2); }
  form { display: flex; gap: var(--space-1); }
  li { display: flex; gap: var(--space-2); align-items: center; &.done span { text-decoration: line-through; color: var(--muted); } }
  .empty { color: var(--muted); }
  footer button.on { color: var(--accent); }
</style>
