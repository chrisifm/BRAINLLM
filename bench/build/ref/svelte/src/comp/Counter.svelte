<script>
  let { start = 0, label = 'Clicks' } = $props()
  let n = $state(start)
  let dbl = $derived(n * 2)
  $effect(() => { if (import.meta.env.DEV) console.log(n) })
</script>

<button onclick={() => n++} class:hot={n > 10}>{label}: {n} × 2 = {dbl}</button>

<style>
  button { padding: var(--space-2); &.hot { color: var(--bad); } }
</style>
