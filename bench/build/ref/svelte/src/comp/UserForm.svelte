<script>
  let { user = null, busy = false, onsave, oncancel } = $props()
  let name = $state(user ? user.name : '')
  let email = $state(user ? user.email : '')
  let active = $state(user ? user.active : true)

  let valid = $derived(name.trim() !== '' && email.includes('@'))

  const save = (e) => {
    e.preventDefault()
    if (!valid || busy) return
    onsave?.({ id: user ? user.id : null, name: name.trim(), email: email.trim(), active })
  }
</script>

<form onsubmit={save}>
  <label>Name <input bind:value={name} /></label>
  <label>Email <input type="email" bind:value={email} /></label>
  <label><input type="checkbox" bind:checked={active} /> Active</label>
  <button disabled={!valid || busy}>{busy ? 'Saving…' : 'Save'}</button>
  <button type="button" onclick={() => oncancel?.()}>Cancel</button>
</form>

<style>
  form { display: grid; gap: var(--space-2); }
</style>
