// BRAINLLM runaway probe (INTERNALS §30.2, #91). Runs after the prerender, in Node on the DOM shim: each view or island
// is mounted with fake timers and stepped for N seconds of virtual time (default 30 s, ~200 ms real) with no user
// input. It samples, per component (`data-b` hash): DOM nodes; per signal: the length of the lists it holds; and the
// animation frames requested. Sustained growth with no input, or a requestAnimationFrame loop re-armed in every frame
// for the whole run, is a runaway: B0907 findings with what grew and how fast. Repeated bounded work (a clock or a
// stopwatch updating its text) is not: it does not grow and it is what such a component is for. brain-pack (BUILD) calls it and fails the
// build, doctor and test on any finding.
//   const findings = await probe({ core, mount: root => core.mount(View, root, props), seconds: 30 })
//   → [{ code: 'B0907', comp: '<hash>' | null, what: 'nodes' | 'list' | 'raf', growth: '+40 nodes/s', msg, fix }]
const TICK = 100 // ms of virtual time per step
const FIX = 'bound the growth (slice the list, stop the interval, clean up the timer in the fx) or move the work behind a user action'

export async function probe({ core, mount, seconds = 30, document: doc = globalThis.document }) {
  const G = globalThis, N = core.sig(0).constructor, P = N.prototype
  // ---- fake clock: timers, animation frames and Date.now move only when the probe steps ----
  let now = 0, seq = 0
  const timers = new Map() // id → { at, f, every }
  const saved = { st: G.setTimeout, ct: G.clearTimeout, si: G.setInterval, ci: G.clearInterval, raf: G.requestAnimationFrame, caf: G.cancelAnimationFrame, dn: Date.now }
  const add = (f, ms, every) => (timers.set(++seq, { at: now + Math.max(0, +ms || 0), f, every: every ? Math.max(1, +ms || 1) : 0 }), seq)
  G.setTimeout = (f, ms, ...a) => add(() => f(...a), ms, false)
  G.setInterval = (f, ms, ...a) => add(() => f(...a), ms, true)
  G.clearTimeout = G.clearInterval = id => timers.delete(id)
  G.requestAnimationFrame = f => add(() => (rafTicks.add(tickNo), f(now)), 16, false)
  G.cancelAnimationFrame = id => timers.delete(id)
  const t0 = saved.dn.call(Date)
  Date.now = () => t0 + now
  // ---- instrumentation: list lengths per signal, ticks with an animation frame ----
  const lists = new Map() // signal → { last: length }
  const rafTicks = new Set()
  let tickNo = 0
  const vd = Object.getOwnPropertyDescriptor(P, 'v')
  Object.defineProperty(P, 'v', {
    configurable: true,
    get: vd.get,
    set(x) {
      vd.set.call(this, x)
      if (Array.isArray(x)) (lists.get(this) || lists.set(this, []).get(this)).last = x.length
    },
  })
  const flush = async () => {
    for (let i = 0; i < 3; i++) await new Promise(r => queueMicrotask(r))
  }
  // nodes per component: each `data-b` root counts its elements, nested components count apart
  const perComp = () => {
    const out = {}
    const walk = (el, h) => {
      const own = el.getAttribute && el.getAttribute('data-b')
      const at = own || h
      if (at) out[at] = (out[at] || 0) + 1
      for (const c of el.childNodes || []) if (c.nodeType === 1) walk(c, at)
    }
    walk(root, null)
    return out
  }

  const root = doc.createElement('div')
  doc.body.appendChild(root)
  const findings = []
  let stop = null
  try {
    stop = mount(root)
    await flush()
    const samples = [] // per virtual second: { nodes: {hash: n}, lists: Map(signal → length) }
    const steps = Math.round((seconds * 1000) / TICK)
    for (let s = 0; s <= steps; s++) {
      tickNo = s
      now = s * TICK
      // run every timer due by now, in time order (an interval re-arms itself)
      for (;;) {
        let next = null
        for (const [id, t] of timers) if (t.at <= now && (!next || t.at < next[1].at)) next = [id, t]
        if (!next) break
        const [id, t] = next
        if (t.every) t.at += t.every
        else timers.delete(id)
        try {
          t.f()
        } catch {} // a timer that throws is the error capture's business, not the probe's
      }
      await flush()
      if (s % Math.round(1000 / TICK) === 0) samples.push({ nodes: perComp(), lists: new Map([...lists].map(([k, v]) => [k, v.last])) })
    }
    // ---- verdicts: growth in every second of the second half, and well above the start ----
    const half = samples.slice(Math.floor(samples.length / 2))
    const rising = vals => vals.length > 3 && vals.every((v, i) => i === 0 || v > vals[i - 1])
    const hashes = new Set(samples.flatMap(x => Object.keys(x.nodes)))
    for (const h of hashes) {
      const v = half.map(x => x.nodes[h] || 0)
      const all = samples.map(x => x.nodes[h] || 0)
      if (rising(v) && all.at(-1) - all[0] >= 20) {
        const rate = Math.round((all.at(-1) - all[0]) / seconds)
        findings.push({ code: 'B0907', comp: h, what: 'nodes', growth: `+${rate} nodes/s`, msg: `component ${h} grows with no input: +${rate} nodes/s over ${seconds} s of virtual time`, fix: FIX })
      }
    }
    for (const sig of lists.keys()) {
      const v = half.map(x => x.lists.get(sig) ?? 0), all = samples.map(x => x.lists.get(sig) ?? 0)
      if (rising(v) && all.at(-1) - all[0] >= 10) {
        const rate = +((all.at(-1) - all[0]) / steps).toFixed(2)
        findings.push({ code: 'B0907', comp: null, what: 'list', growth: `+${rate} item/tick`, msg: `a list grows with no input: +${rate} item/tick (${all[0]} → ${all.at(-1)} items) over ${seconds} s of virtual time`, fix: FIX })
      }
    }
    if (rafTicks.size >= 0.95 * steps) findings.push({ code: 'B0907', comp: null, what: 'raf', growth: 'a frame every tick', msg: `a requestAnimationFrame loop re-arms itself in every frame with no input for ${seconds} s of virtual time`, fix: 'stop the loop when there is nothing to animate (or use a CSS / Web Animations transition)' })
  } finally {
    if (stop) stop()
    root.remove()
    Object.defineProperty(P, 'v', vd)
    Object.assign(G, { setTimeout: saved.st, clearTimeout: saved.ct, setInterval: saved.si, clearInterval: saved.ci, requestAnimationFrame: saved.raf, cancelAnimationFrame: saved.caf })
    Date.now = saved.dn
  }
  return findings
}
