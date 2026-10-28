<script>
  import Badge from '../ui/Badge.svelte'
  import UserForm from '../comp/UserForm.svelte'
  import api from '../lib/api.js'
  import fmt from '../lib/format.js'

  let users = $state([])
  let query = $state('')
  let editing = $state(null)
  let busy = $state(false)
  let error = $state('')

  // Equivalent of `load()`: there is no server in a static Vite build, so it runs on mount.
  $effect(() => { api.list().then((list) => { users = list }) })

  let shown = $derived.by(() => {
    const q = query.trim().toLowerCase()
    if (q === '') return users
    return users.filter(u => u.name.toLowerCase().includes(q) || u.email.toLowerCase().includes(q))
  })
  let activeCount = $derived(users.filter(u => u.active).length)

  const save = async (data) => {
    busy = true
    error = ''
    try {
      const saved = data.id === null ? await api.create(data) : await api.update(data)
      users = data.id === null
        ? [...users, saved]
        : users.map(u => u.id === saved.id ? saved : u)
      editing = null
    } catch (e) {
      error = e.message
    }
    busy = false
  }

  const remove = async (id) => {
    error = ''
    try {
      await api.remove(id)
      users = users.filter(u => u.id !== id)
    } catch (e) {
      error = e.message
    }
  }
</script>

<main class="dash">
  <header>
    <h1>Users</h1>
    <span>{activeCount} active of {fmt.plural(users.length, 'user', 'users')}</span>
    <input bind:value={query} placeholder="Search by name or email" aria-label="Search users" />
    <button onclick={() => editing = {}}>New</button>
  </header>

  {#if error !== ''}
    <p class="error" role="alert">{error}</p>
  {/if}

  {#if editing !== null}
    <UserForm user={editing.id ? editing : null} {busy} onsave={save} oncancel={() => editing = null} />
  {/if}

  <table>
    <thead><tr><th>Name</th><th>Email</th><th>Status</th><th></th></tr></thead>
    <tbody>
      {#each shown as u (u.id)}
        <tr>
          <td>{u.name}</td>
          <td>{u.email}</td>
          <td><Badge tone={u.active ? 'ok' : 'bad'} text={u.active ? 'Active' : 'Inactive'} /></td>
          <td>
            <button onclick={() => editing = u}>Edit</button>
            <button onclick={() => remove(u.id)}>Delete</button>
          </td>
        </tr>
      {:else}
        <tr><td colspan="4">{query === '' ? 'No users yet.' : 'No results for this search.'}</td></tr>
      {/each}
    </tbody>
  </table>
</main>

<style>
  .dash { display: grid; gap: var(--space-3); padding: var(--space-3); background: var(--bg); color: var(--fg); }
  header { display: flex; gap: var(--space-2); align-items: center; }
  .error { color: var(--bad); }
  table { width: 100%; border-collapse: collapse; }
  td, th { padding: var(--space-1) var(--space-2); text-align: left; }
</style>
