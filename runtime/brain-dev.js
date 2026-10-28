// BRAINLLM dev (SPEC §6.2): HMR that keeps sig state by name. Development only.
// Served next to the core under /@brain/ and imports ./brain.js relatively: a single instance of the signal graph.
// The dev codegen wraps each component: `export default dev(Counter)` and emits `sig(v, 'name')`.
// v0 limit: if after an hmr a parent removes that row or branch, it removes the old nodes; then do a full reload.
// Registry for brain-inspect (§9): `live` (hash → instances), `inst` (root node N → instance) and the event `bus`.
import { _dev, fx } from './brain.js'

let cur = null // instance under construction: { m: Map name→sig, r: Map name→value to restore | null }
let quiet = 0 // >0 during hmr: the bus emits `hmr` instead of mount/dispose
export const live = new Map()
export const inst = new WeakMap()
export const bus = new Set() // fn(kind, data): 'mount' | 'dispose' | 'hmr' | 'hydrate'
// sig → { name, comp } for every named signal (dev codegen `sig(v, 'name')`, store fields): brain-why reads it
export const names = new WeakMap()
// last component mounts with their mount time, HMR remounts included (brain_impact: Δ mount time per save)
export const mounts = []
const emit = (k, d) => {
  for (const f of bus) f(k, d)
}
const { root, owner } = _dev((n, k) => {
  if (!n) return emit('hydrate', { url: k })
  names.set(n, { name: k, comp: (cur && cur.f) || null })
  if (cur) {
    if (cur.r && cur.r.has(k)) n._ = cur.r.get(k)
    cur.m.set(k, n)
  }
  return n
})
export { owner }

function build(i, restore) {
  const pc = cur
  cur = { m: new Map(), r: restore, f: i.C.f || i.h || null }
  try {
    return root(d => {
      i.r = owner() // register before C(): fx from the first run already have an owner (inspector)
      inst.set(i.r, i)
      const t = performance.now()
      const out = i.C(i.$p)
      i.$mt = performance.now() - t
      mounts.unshift({ comp: i.C.f || i.h || null, ms: Math.round(i.$mt * 100) / 100, at: Date.now(), hmr: quiet > 0 })
      if (mounts.length > 50) mounts.length = 50
      // a component may return a node, a fragment or an array (guard() returns its nodes plus an anchor)
      i.n = Array.isArray(out) ? out.flat(Infinity) : out.nodeType === 11 ? [...out.childNodes] : [out]
      for (const x of i.n) if (x.nodeType === 1) x.$i = i
      i.d = d
      i.m = cur.m
      const set = live.get(i.h) || live.set(i.h, new Set()).get(i.h)
      set.add(i)
      // on dispose the nodes stop pointing at the instance: a browser may keep a detached node alive for a while
      // (last hovered or focused element, accessibility tree) and must not keep the whole instance with it
      fx(() => () => {
        set.delete(i)
        for (const x of i.n || []) if (x.nodeType === 1 && x.$i === i) x.$i = null
        quiet || emit('dispose', i)
      })
      if (!quiet) emit('mount', i)
      return out
    }, i.p)
  } finally {
    cur = pc
  }
}

export function dev(C) {
  const W = $p => build({ C, $p, h: C.h, p: owner(), n: null, d: null, m: null, r: null }, null)
  Object.assign(W, C) // h, p, form, seo, head… whatever the compiler (or guard) put on it
  W.C = C
  return W
}

// Remounts in place every live instance of `hash` with the new component from `mod`.
export function hmr(hash, mod) {
  const W = Object.values(mod).find(c => c && c.h === hash)
  if (!W) return 0
  const C = W.C || W
  let k = 0
  quiet++
  try {
    for (const i of [...(live.get(hash) || [])]) {
      const old = i.n, pa = old[0].parentNode
      // an anchor that survives the dispose (guard() removes its own nodes when its owner is disposed)
      const first = document.createComment('')
      if (pa) pa.insertBefore(first, old[0])
      const vals = new Map([...i.m].map(([name, n]) => [name, n._]))
      i.d()
      emit('dispose', { h: i.h, C: i.C, n: old, r: null, hmr: 1 })
      i.C = C
      build(i, vals)
      if (pa) for (const x of i.n) pa.insertBefore(x, first)
      for (const x of old) x.remove()
      first.remove()
      k++
    }
  } finally {
    quiet--
  }
  emit('hmr', { h: hash, n: k })
  return k
}
