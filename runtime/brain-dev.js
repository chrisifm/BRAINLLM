// BRAINLLM dev (SPEC §6.2): HMR that keeps sig state by name. Development only.
// Served next to the core under /@brain/ and imports ./brain.js relatively: a single instance of the signal graph.
// The dev codegen wraps each component: `export default dev(Counter)` and emits `sig(v, 'name')`.
// Static proof retains node identity; incompatible nested updates rebuild a proven owner boundary before parent ranges can become stale.
// Registry for brain-inspect (§9): `live` (hash → instances), `inst` (root node N → instance) and the event `bus`.
import { _dev, fx, tpl } from './brain.js'

let cur = null // instance under construction: { m: Map name→sig, r: Map name→value to restore | null }
let quiet = 0 // >0 during hmr: the bus emits `hmr` instead of mount/dispose
const definitions = new Map() // bounded current implementations for future mounts through an older import
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
  cur = { m: new Map(), r: restore, f: i.C.f || i.h || null, i }
  try {
    return root(d => {
      i.r = owner() // register before C(): fx from the first run already have an owner (inspector)
      inst.set(i.r, i)
      const t = performance.now()
      i.fragments = new Set()
      i.fragmentRefs = 0
      i.fragmentOverflow = false
      let out = i.C(i.$p)
      i.$mt = performance.now() - t
      mounts.unshift({ comp: i.C.f || i.h || null, ms: Math.round(i.$mt * 100) / 100, at: Date.now(), hmr: quiet > 0 })
      if (mounts.length > 50) mounts.length = 50
      // a component may return a node, a fragment or an array (guard() returns its nodes plus an anchor)
      i.n = out == null ? [] : Array.isArray(out) ? out.flat(Infinity) : out.nodeType === 11 ? [...out.childNodes] : [out]
      if (!i.n.length) { out = document.createComment(''); i.n = [out] }
      for (const x of i.n) if (x.nodeType === 1) x.$i = i
      i.d = d
      i.m = cur.m
      const set = live.get(i.h) || live.set(i.h, new Set()).get(i.h)
      set.add(i)
      // on dispose the nodes stop pointing at the instance: a browser may keep a detached node alive for a while
      // (last hovered or focused element, accessibility tree) and must not keep the whole instance with it
      fx(() => () => {
        set.delete(i)
        if (!set.size) live.delete(i.h)
        for (const x of i.n || []) if (x.nodeType === 1 && x.$i === i) x.$i = null
        if (!quiet) for (const node of i.n || []) node.remove()
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
  const registered = typeof C.h === 'string' && (definitions.has(C.h) || definitions.size < 1024)
  if (registered) definitions.set(C.h, C)
  const W = $p => build({ C: definitions.get(C.h) || C, $p, h: C.h, p: owner(), n: null, d: null, m: null, r: null }, null)
  Object.assign(W, C) // h, p, form, seo, head… whatever the compiler (or guard) put on it
  W.C = C
  if (typeof C.h === 'string' && !registered) W.$hmrUnsupportedReason = 'definition-registry-budget'
  return W
}

// Compiler-only fragment factories record pristine references before dynamic holes insert/move siblings.
// Per-owner cleanup prevents inactive branches/rows from retaining detached nodes.
export function fragment(create, hash, index) {
  let cached = null, factory = create
  return () => {
    let i = cur?.i?.h === hash ? cur.i : null
    for (let o = owner(); !i && o; o = o.p) { const found = inst.get(o); if (found?.h === hash) i = found }
    const html = i?.C?.$hmr?.templates?.[index]
    if (typeof html === 'string' && html !== cached) { cached = html; factory = tpl(html) }
    const out = factory()
    if (!i || i.fragmentOverflow) return out
    const nodes = [], roots = out.nodeType === 11 ? [...out.childNodes] : [out]
    const rootSet = new Set(roots)
    const visit = (node, depth) => {
      if (depth > 64 || nodes.length >= 4096) return false
      nodes.push({ node, parent: node.parentNode, root: rootSet.has(node) })
      for (const child of node.childNodes || []) if (!visit(child, depth + 1)) return false
      return true
    }
    if (!roots.every(node => visit(node, 0)) || i.fragmentRefs + nodes.length > 8192 || i.fragments.size >= 2048) { i.fragmentOverflow = true; return out }
    const record = { index, nodes, owner: owner() }
    i.fragments.add(record)
    i.fragmentRefs += nodes.length
    fx(() => () => { if (i.fragments.delete(record)) i.fragmentRefs -= nodes.length })
    return out
  }
}

export let lastUpdate = { mode: 'none', patched: 0, remounted: 0, reason: 'not-run' }
const safeAttribute = name => ['class', 'title', 'alt', 'placeholder', 'style'].includes(name) || name.startsWith('aria-') || name.startsWith('data-') && !['data-b', 'data-cv-ref'].includes(name)
const attributes = node => new Map([...node.attributes].map(attribute => [attribute.name, attribute.value]))
function metadata(value) {
  return value?.version === 1 && typeof value.wiring === 'string' && value.wiring.length <= 65536 && Array.isArray(value.templates) && value.templates.length <= 64 && value.templates.every(html => typeof html === 'string') && value.templates.reduce((n, html) => n + html.length, 0) <= 65536
}
function templateChanges(before, after) {
  const changes = [], shapes = []
  let visits = 0
  for (let index = 0; index < before.length; index++) {
    const old = tpl(before[index])(), next = tpl(after[index])(), flat = []
    const walk = (a, b, depth) => {
      if (++visits > 8192 || depth > 64) throw Error('template-budget')
      if (!a || !b || a.nodeType !== b.nodeType || a.nodeName !== b.nodeName || a.namespaceURI !== b.namespaceURI) throw Error('template-topology')
      const position = flat.length
      flat.push(a)
      if (a.nodeType === 3 && a.data !== b.data) changes.push({ index, position, kind: 'text', before: a.data, after: b.data })
      else if (a.nodeType === 8 && a.data !== b.data) throw Error('anchor-topology')
      if (a.nodeType === 1) {
        const previous = attributes(a), current = attributes(b)
        for (const name of new Set([...previous.keys(), ...current.keys()])) if (previous.get(name) !== current.get(name)) {
          if (!safeAttribute(name)) throw Error('stateful-or-unsupported-attribute')
          changes.push({ index, position, kind: 'attribute', name, before: previous.get(name) ?? null, after: current.get(name) ?? null })
        }
      }
      if (a.childNodes.length !== b.childNodes.length) throw Error('template-topology')
      for (let n = 0; n < a.childNodes.length; n++) walk(a.childNodes[n], b.childNodes[n], depth + 1)
    }
    const a = old.nodeType === 11 ? [...old.childNodes] : [old], b = next.nodeType === 11 ? [...next.childNodes] : [next]
    if (a.length !== b.length) throw Error('template-topology')
    for (let n = 0; n < a.length; n++) walk(a[n], b[n], 0)
    shapes.push(flat)
  }
  return { changes, shapes }
}
function staticPlan(i, C) {
  const before = i.C.$hmr, after = C.$hmr
  if (!metadata(before) || !metadata(after)) return 'metadata-unavailable-or-budget'
  if (before.wiring !== after.wiring) return 'compiled-wiring-changed'
  if (before.templates.length !== after.templates.length) return 'template-topology'
  if (i.fragmentOverflow) return 'instance-reference-budget'
  let analysis
  try { analysis = templateChanges(before.templates, after.templates) } catch (error) { return ['template-budget', 'template-topology', 'anchor-topology', 'stateful-or-unsupported-attribute'].includes(error.message) ? error.message : 'template-proof-failed' }
  const operations = []
  for (const record of i.fragments || []) {
    const shape = analysis.shapes[record.index]
    if (!shape || shape.length !== record.nodes.length) return 'instance-shape-conflict'
    for (const change of analysis.changes) if (change.index === record.index) {
      const entry = record.nodes[change.position], node = entry?.node
      if (!node || node.nodeType !== shape[change.position].nodeType || !entry.root && node.parentNode !== entry.parent) return 'instance-node-conflict'
      if (change.kind === 'text' ? node.data !== change.before : node.getAttribute(change.name) !== change.before) return 'runtime-owned-content-conflict'
      operations.push({ node, ...change })
      if (operations.length > 8192) return 'instance-operation-budget'
    }
  }
  return { i, C, operations }
}
function applyStaticPlan({ i, C, operations }) {
  // Plan first; preserve live handlers/effects/owners and target the original nodes only.
  const selection = document.getSelection?.()
  const saved = selection?.rangeCount ? { anchor: selection.anchorNode, anchorOffset: selection.anchorOffset, focus: selection.focusNode, focusOffset: selection.focusOffset } : null
  try {
    for (const operation of operations) {
      if (operation.kind === 'text') operation.node.data = operation.after
      else if (operation.after === null) operation.node.removeAttribute(operation.name)
      else operation.node.setAttribute(operation.name, operation.after)
    }
  } catch { return 'native-dom-patch-failed' }
  if (saved && selection.setBaseAndExtent) {
    const length = node => node?.nodeType === 3 ? node.data.length : node?.childNodes.length || 0
    try { selection.setBaseAndExtent(saved.anchor, Math.min(saved.anchorOffset, length(saved.anchor)), saved.focus, Math.min(saved.focusOffset, length(saved.focus))) } catch {}
  }
  i.C = C
  return null
}

// A parent list/branch keeps its own node range. Replacing a child range directly would leave those
// captured roots stale, so incompatible nested updates rebuild the outermost owned component once.
function boundary(i) {
  let target = i, count = 0
  for (let owner = i.p; owner; owner = owner.p) {
    if (++count > 256) return null
    const ancestor = inst.get(owner)
    if (ancestor) target = ancestor
  }
  return !target.p || !target.p.f && !target.p.p ? target : null
}
function under(i, ancestor) {
  if (i === ancestor) return true
  let count = 0
  for (let owner = i.p; owner && ++count <= 256; owner = owner.p) if (owner === ancestor.r) return true
  return false
}
function remount(i, C) {
  const old = i.n, parent = old[0]?.parentNode
  const anchor = document.createComment('')
  if (parent) parent.insertBefore(anchor, old[0])
  const values = new Map([...i.m].map(([name, node]) => [name, node._]))
  i.d()
  emit('dispose', { h: i.h, C: i.C, n: old, r: null, hmr: 1 })
  i.C = C
  build(i, values)
  if (parent) for (const node of i.n) parent.insertBefore(node, anchor)
  for (const node of old) node.remove()
  anchor.remove()
}
// Safe static fragments retain node identity. Other updates rebuild a proven owner boundary.
export function hmr(hash, mod) {
  const W = Object.values(mod).find(component => component && component.h === hash)
  if (!W) return 0
  const C = W.C || W
  if (W.$hmrUnsupportedReason) { lastUpdate = { mode: 'reload-required', patched: 0, remounted: 0, reason: W.$hmrUnsupportedReason }; return 0 }
  const instances = [...(live.get(hash) || [])].filter(i => i.r?.s !== 3)
  const plans = [], rebuild = new Map(), reasons = new Set(), patched = new Set()
  for (const i of instances) {
    let plan
    try { plan = staticPlan(i, C) } catch { plan = 'native-proof-operation-failed' }
    if (typeof plan !== 'string') { plans.push(plan); continue }
    const target = boundary(i)
    if (!target) { lastUpdate = { mode: 'reload-required', patched: 0, remounted: 0, reason: 'parent-range-owner-unavailable' }; return 0 }
    reasons.add(plan)
    if (target !== i) reasons.add('ancestor-remount-for-parent-owned-range')
    rebuild.set(target, definitions.get(target.h) || target.C)
  }
  let affected = 0, remounted = 0, ancestors = 0
  quiet++
  try {
    for (const plan of plans) {
      if ([...rebuild.keys()].some(target => under(plan.i, target))) continue
      const failure = applyStaticPlan(plan)
      if (!failure) patched.add(plan.i)
      else {
        const target = boundary(plan.i)
        if (!target) { lastUpdate = { mode: 'reload-required', patched: patched.size, remounted, reason: 'native-patch-failed-without-owned-range' }; return 0 }
        rebuild.set(target, definitions.get(target.h) || target.C)
        reasons.add(failure)
        if (target !== plan.i) reasons.add('ancestor-remount-for-parent-owned-range')
      }
    }
    for (const [target, implementation] of rebuild) {
      if (target.r?.s === 3) continue
      const covered = instances.filter(i => under(i, target)).length
      for (const i of patched) if (under(i, target)) patched.delete(i)
      if (target.h !== hash) ancestors++
      remount(target, implementation)
      affected += covered
      remounted++
    }
  } finally { quiet-- }
  const count = patched.size + affected
  lastUpdate = { mode: !count ? 'future-mount-updated' : remounted ? patched.size ? 'mixed' : ancestors ? 'ancestor-remount' : 'component-remount' : 'static-fragment', patched: patched.size, remounted, ancestorRemounted: ancestors, reason: [...reasons].join(',') || (!count ? 'no-live-instance; current-definition-retained' : 'exact-compiled-wiring-and-template-shape'), stateRestoration: ancestors ? 'ancestor-named-state; descendant-instances-recreated' : 'named-state-or-static-identity' }
  emit('hmr', { h: hash, n: count, ...lastUpdate })
  return count
}
