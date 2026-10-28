// BRAINLLM update provenance (INTERNALS §43.2-§43.1, #198). Development only: the dev client loads it next to
// brain-dev.js; nothing ships to production and the core carries no hook for it.
// - why(selector | node) → for a node that updated: which signal write caused each change (component, field,
//   `file.brain:line` of the write) and the chain of drv between them. MCP `brain_why` (TOOLCHAIN #200) calls it
//   through globalThis.__brainWhy.
// - interactions() → per user interaction (click, input, change, keydown, submit): how many DOM mutations it caused
//   and on how many nodes, for the impact report (`brain_impact`, "updates per interaction").
// How: it wraps, in dev only, the signal setter (records the write and its stack), the node run (an effect run
// knows, before it re-runs, which of its sources were written: those are its causes) and the DOM write methods
// (each mutation made while an effect runs is attributed to that effect's causes).
import { sig } from './brain.js'
import { names, mounts as M } from './brain-dev.js'

const G = globalThis
const N = sig(0).constructor, P = N.prototype
const KEEP = 10, LAST = 50
const WR = new Map() // sig → write record, for the current flush (cleared once it settles)
const BY = new WeakMap() // node → [mutation records], newest first
const INT = [] // last interactions
let RUN = null // causes of the effect running now
let cur = null // interaction being measured
let clearing = 0

// ---------- signal writes ----------
const vd = Object.getOwnPropertyDescriptor(P, 'v')
Object.defineProperty(P, 'v', {
  configurable: true,
  get: vd.get,
  set(x) {
    if (!this.f && !Object.is(x, this._)) {
      const info = names.get(this)
      WR.set(this, { sig: this, name: (info && info.name) || null, comp: (info && info.comp) || null, from: this._, to: x, stack: new Error().stack || '' })
      if (!clearing) (clearing = 1), setTimeout(() => ((clearing = 0), WR.clear()), 0) // after the flush microtasks
    }
    vd.set.call(this, x)
  },
})

// causes of a node's next run: its sources that were written, or reached through a drv chain
function causesOf(n, via = [], seen = new Set()) {
  const out = [], S = n.S
  if (!S) return out
  for (let i = 0; i < S.length; i += 2) {
    const s = S[i]
    if (seen.has(s)) continue
    seen.add(s)
    const w = WR.get(s)
    if (w) out.push({ w, via })
    else if (s.f && !s.e) out.push(...causesOf(s, [...via, (names.get(s) && names.get(s).name) || 'drv'], seen))
  }
  return out
}
const r0 = P.r
P.r = function () {
  if (!this.e || !WR.size) return r0.call(this)
  const prev = RUN
  RUN = causesOf(this)
  try {
    return r0.call(this)
  } finally {
    RUN = prev
  }
}

// ---------- DOM writes ----------
function note(node, kind) {
  if (!node || !RUN || !RUN.length) return
  const rec = { kind, t: Date.now(), causes: RUN }
  for (const n of node.nodeType === 3 && node.parentNode ? [node, node.parentNode] : [node]) {
    const a = BY.get(n) || (BY.set(n, []), BY.get(n))
    a.unshift(rec)
    a.length = Math.min(a.length, KEEP)
  }
  if (cur) cur.mutations++, cur.nodes.add(node)
}
const owner = (o, k) => {
  for (; o; o = Object.getPrototypeOf(o)) {
    const d = Object.getOwnPropertyDescriptor(o, k)
    if (d) return [o, d]
  }
  return []
}
function wrap(sample, k, kind) {
  const [o, d] = owner(sample, k)
  if (!o || (d.value && d.value.$why) || (d.set && d.set.$why)) return
  if (typeof d.value === 'function') {
    const f = d.value
    const g = function (...a) {
      note(kind.node ? kind.node(this, a) : this, typeof kind === 'function' ? kind(a) : kind.k(a))
      return f.apply(this, a)
    }
    g.$why = 1
    Object.defineProperty(o, k, { ...d, value: g })
  } else if (d.set) {
    const set = function (v) {
      note(this, kind)
      d.set.call(this, v)
    }
    set.$why = 1
    Object.defineProperty(o, k, { ...d, set })
  }
}
function install() {
  const d = G.document
  if (!d || G.__brainWhy) return
  const el = d.createElement('div'), tx = d.createTextNode(''), cm = d.createComment('')
  wrap(el, 'setAttribute', a => 'attr:' + a[0])
  wrap(el, 'removeAttribute', a => 'attr:' + a[0])
  wrap(el, 'textContent', 'text')
  wrap(tx, 'data', 'text')
  wrap(tx, 'nodeValue', 'text')
  wrap(el, 'insertBefore', { node: (_, a) => a[0], k: () => 'insert' })
  wrap(el, 'appendChild', { node: (_, a) => a[0], k: () => 'insert' })
  wrap(el, 'removeChild', { node: (_, a) => a[0], k: () => 'remove' })
  for (const n of [el, tx, cm]) wrap(n, 'remove', () => 'remove')
  for (const k of ['value', 'checked', 'selected']) wrap(d.createElement('input'), k, 'prop:' + k)
  // classList / style have no link back to their element: map them through the element's getter (browsers)
  for (const [k, m] of [['classList', 'toggle'], ['style', 'setProperty']]) {
    const [o, dd] = owner(el, k)
    if (!o || !dd.get) continue
    const back = new WeakMap()
    Object.defineProperty(o, k, { ...dd, get() { const v = dd.get.call(this); back.set(v, this); return v } })
    const [po, pd] = owner(el[k], m)
    if (po && typeof pd.value === 'function') {
      const f = pd.value
      Object.defineProperty(po, m, { ...pd, value(...a) { note(back.get(this), (k === 'style' ? 'style:' : 'class:') + a[0]); return f.apply(this, a) } })
    }
  }
  // interactions: everything a user event causes until the updates settle
  for (const t of ['click', 'input', 'change', 'keydown', 'submit'])
    d.addEventListener(t, e => {
      if (cur) return
      cur = { type: t, target: describe(e.target), mutations: 0, nodes: new Set(), t0: G.performance ? performance.now() : Date.now() }
      setTimeout(() => {
        const c = cur
        cur = null
        INT.unshift({ type: c.type, target: c.target, mutations: c.mutations, nodes: c.nodes.size, ms: Math.round(((G.performance ? performance.now() : Date.now()) - c.t0) * 10) / 10 })
        INT.length = Math.min(INT.length, LAST)
      }, 0)
    }, true)
  G.__brainWhy = { why, interactions, mounts, clear }
}
const describe = n => {
  if (!n || n.nodeType !== 1) return n && n.parentNode ? describe(n.parentNode) : null
  const id = n.getAttribute('id'), c = n.getAttribute('class')
  return n.tagName.toLowerCase() + (id ? '#' + id : '') + (c ? '.' + c.trim().split(/\s+/).join('.') : '')
}

// ---------- answers ----------
const RT = /\/(@brain|_b)\/brain[\w-]*\.js|\/runtime\/brain[\w-]*\.js|\/brain-why\.js/
const MAPS = new Map()
// the first app frame of the write's stack, as file.brain:line when the dev module carries its source map
async function where(stack) {
  for (const l of String(stack).split('\n').slice(1)) {
    const m = /\(?((?:https?|file):\/\/[^\s)]+?):(\d+):\d+\)?\s*$/.exec(l)
    if (!m || RT.test(m[1])) continue
    const url = m[1], line = +m[2]
    if (!MAPS.has(url)) MAPS.set(url, load(url))
    const lines = await MAPS.get(url)
    const path = url.replace(/^\w+:\/\/[^/]*/, '').replace(/^\//, '').replace(/\?.*$/, '')
    return lines && lines[line - 1] ? `${path}:${lines[line - 1]}` : `${path}:${line}`
  }
  return null
}
async function load(url) {
  try {
    if (!/^https?:/.test(url) || !G.fetch) return null
    const t = await (await fetch(url)).text()
    const m = /sourceMappingURL=data:application\/json;base64,([A-Za-z0-9+/=]+)/.exec(t)
    if (!m) return null
    const { decode } = await import('./brain-err.js')
    return decode(JSON.parse(atob(m[1])).mappings)
  } catch {
    return null
  }
}
const val = v => (typeof v === 'function' ? 'fn' : v && typeof v === 'object' ? JSON.stringify(v).slice(0, 80) : v)

// why('#total') → [{ kind: 'text', at, causes: [{ signal: 'cart.total', comp: 'views/cart.brain', src:
// 'views/cart.brain:12', from, to, via: ['subtotal'] }] }], newest first; [] when that node never updated
export async function why(target) {
  const n = typeof target === 'string' ? G.document && document.querySelector(target) : target
  if (!n) return { error: `no element matches ${target}` }
  const recs = [...(BY.get(n) || [])]
  return Promise.all(
    recs.map(async r => ({
      kind: r.kind,
      at: r.t,
      causes: await Promise.all(r.causes.map(async c => ({ signal: c.w.name, comp: c.w.comp, src: await where(c.w.stack), from: val(c.w.from), to: val(c.w.to), via: c.via }))),
    }))
  )
}
export const interactions = () => INT.slice()
// last mounts, newest first: [{ comp: 'comp/x.brain', ms, at, hmr }] (brain-dev measures each one)
export const mounts = () => M.slice()
export const clear = () => (INT.length = 0)

install()
