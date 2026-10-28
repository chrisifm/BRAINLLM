// BRAINLLM component boundaries (INTERNALS §42). The compiler wraps every comp/, views/ and app.brain component as
// `guard(Comp, '<meta fallback text>')`; nothing for the author to write. The core routes each error to the nearest
// boundary (owner chain for fx/drv/list rows, DOM ancestors for handlers):
//  - render, fx, drv, list rows: the component's effects stop and its nodes are replaced by the fallback — a short
//    notice with a Retry button that remounts only this component (3 failures in 10 s → no button, no retry loop);
//  - event handlers (sync, or an async one whose await rejects): the component stays as it was and the user sees a
//    short toast (the error's message for HTTP errors, a generic one otherwise).
// The error itself is reported first by the core (brain-err when present, else the console). During SSR the failing
// component renders its fallback into the HTML and the error is collected in globalThis.__brainSSR for the server log.
import { _rt, fx, sig } from './brain.js'

const G = globalThis, I = _rt(), LIMIT = 3, WINDOW = 10000
const TEXT = 'This section could not load.'

// ---------- circuit breaker (§30.3) ----------
// Per instance budgets, checked as its effects run: runs of one effect per second, continuous work per flush (over budget in
// 3 flushes in a row), and, once per flush that touched it, its DOM nodes (a native count) and list rows (a walk, only when it has
// more nodes than the rows budget, at most once a second unless it grew by a quarter: QA #302, counting every 64 runs cost 250 ms per
// jsfb operation). Crossing one trips THAT instance: its effects stop, a neutral notice replaces it
// (no Retry: it would trip again), B0908 is reported with the budget, the value and the component; the rest of the
// page keeps running. Defaults below; `guard(C, fb, { rows: 200000 })` (from `<meta budget="rows:200000">`) overrides.
// nodes 200 000 and rows 100 000 (QA #302/#303): a legitimate 10,000-row table is ~80 000 nodes, measured without the
// rows walk
const BUDGET = { nodes: 200000, rows: 100000, runs: 1000, ms: 200 }
const STOPPED = 'This part stopped to protect the page.'
const OWN = new WeakMap() // root owner → instance
const now = () => (G.performance ? performance.now() : Date.now())
// work per flush: the time between two effect runs belongs to the earlier one (the hook fires as a run starts); the
// last run of a flush is settled by a microtask queued at the flush's first run
let LX = null, LT = 0, inFlush = false
const touched = new Set()
// elements under a root, and the most element children any one of them has (the rows of a list)
const count = (e, c) => {
  if (e.nodeType !== 1) return
  c.nodes++
  let k = 0
  for (const n of e.childNodes) if (n.nodeType === 1) k++, count(n, c)
  if (k > c.rows) c.rows = k
}
// elements under a root: native when the DOM has it (the shim walks)
const elems = r => {
  if (r.nodeType !== 1) return 0
  if (r.getElementsByTagName) return 1 + r.getElementsByTagName('*').length
  const c = { nodes: 0, rows: 0 }
  count(r, c)
  return c.nodes
}
const size = (x, t) => {
  if (x.off) return
  let n = 0
  for (const r of x.nodes()) n += elems(r)
  if (n > x.b.nodes) return x.trip('nodes', n)
  if (n > x.b.rows && (t - x.rt > 1000 || n > 1.25 * x.rn)) {
    x.rt = t
    x.rn = n
    const c = { nodes: 0, rows: 0 }
    for (const r of x.nodes()) count(r, c)
    if (c.rows > x.b.rows) x.trip('rows', c.rows)
  }
}
const settle = t => {
  if (LX && !LX.off) LX.work += t - LT
}
// a busy device can make one flush slow: only an instance over budget in 3 flushes in a row is runaway work
const close = x => {
  if (x.off) return
  x.slow = x.work > x.b.ms ? (x.slow || 0) + 1 : 0
  if (x.slow >= 3) x.trip('ms', Math.round(x.work))
}
// the hook fires as an effect starts running: N.prototype.r is wrapped (the core has no hook for it, #288). A first run
// (no sources yet) cannot be a loop: it only closes the work of the run before it (QA #302: creating 1,000 rows is 3,000
// first runs, and the full check on each one was the slowdown).
const NP = sig(0).constructor.prototype, run = NP.r
NP.r = function () {
  if (this.e)
    try {
      this.S ? check(this) : born()
    } catch {} // the breaker never breaks a page
  return run.call(this)
}
function born() {
  const t = now()
  settle(t)
  LX = null
  LT = t
}
function check(n) {
  let o = n
  while (o && !OWN.has(o)) o = o.p
  const x = o && OWN.get(o), t = now()
  settle(t)
  LX = x && !x.off ? x : null
  LT = t
  if (!inFlush) {
    inFlush = true
    queueMicrotask(() => {
      settle(now())
      LX = null
      inFlush = false
      const t = now()
      for (const y of touched) close(y), size(y, t), (y.work = 0)
      touched.clear()
    })
  }
  if (!LX) return
  touched.add(x)
  // runs of ONE effect per second (QA #290): creating or updating 1,000 rows runs 1,000 different effects once each,
  // which is legitimate; a runaway loop re-runs the same effect. Runaway = over budget in 2 consecutive seconds, or 10×
  // the budget inside one second.
  if (t - x.t0 > 1000) (x.hot = x.max > x.b.runs && t - x.t0 < 2000 ? (x.hot || 0) + 1 : 0), (x.t0 = t), (x.max = 0), (x.per = new WeakMap())
  const k = (x.per.get(n) || 0) + 1
  x.per.set(n, k)
  if (k > x.max) x.max = k
  if (k > x.b.runs && (x.hot >= 1 || k > 10 * x.b.runs)) return x.trip('runs', k)
}

let tEl = null, tT = 0
function toast(msg) {
  const d = G.document
  if (!d || !d.body) return
  if (!tEl) {
    tEl = d.createElement('div')
    tEl.setAttribute('role', 'status')
    tEl.setAttribute('aria-live', 'polite')
    tEl.setAttribute('class', 'brain-toast')
    tEl.setAttribute('style',
      'position:fixed;left:50%;bottom:16px;transform:translateX(-50%);max-width:90vw;z-index:2147483647;padding:10px 14px;' +
      'border-radius:var(--radius-sm);border:1px solid var(--line);background:var(--surface);color:var(--fg);box-shadow:var(--shadow);font:14px/1.4 var(--font)')
    d.body.appendChild(tEl)
  }
  tEl.textContent = msg
  tEl.removeAttribute('hidden')
  clearTimeout(tT)
  tT = setTimeout(() => tEl.setAttribute('hidden', ''), 4000)
}
// HTTP/API errors carry a message meant for people (ApiError, rpc); anything else gets a generic line
const human = e => (e && (e.kind || e.status || e.code === 'rpc' || e.code === 'api') && e.message) || 'Something went wrong. Try again.'

export function guard(C, fb, budget) {
  const W = $p => {
    const own = I.owner(), end = G.document.createComment('')
    let nodes = [], dispose = null, fails = []
    const notice = (retry, text) => {
      if (typeof fb === 'function' && !text) return I.nodes(fb())
      const box = G.document.createElement('div')
      box.setAttribute('role', 'alert')
      box.setAttribute('class', 'brain-fallback')
      box.textContent = text || fb || TEXT
      if (retry) {
        const b = G.document.createElement('button')
        b.setAttribute('type', 'button')
        b.textContent = 'Retry'
        b.addEventListener('click', () => again())
        box.appendChild(G.document.createTextNode(' '))
        box.appendChild(b)
      }
      return [box]
    }
    const report = (e, w) => {
      if (G.__brainSSR) G.__brainSSR.push({ e, w, comp: W.f || C.f || C.name || null })
    }
    let building = false, pending = null
    const fall = (e, w, text) => {
      if (building) return void (pending = pending || [e, w]) // a child effect failed while this builds: settled by build()
      const now = Date.now()
      fails = [...fails.filter(t => now - t < WINDOW), now]
      if (dispose) dispose(), (dispose = null) // stops its fx, drv and handlers
      const p = end.parentNode
      I.rm(nodes)
      nodes = notice(!text && fails.length < LIMIT, text)
      if (p) for (const n of nodes) p.insertBefore(n, end)
      report(e, w)
    }
    // breaker state of this instance (see NP.r above)
    const x = {
      b: budget ? { ...BUDGET, ...budget } : BUDGET,
      t0: 0,
      rt: -1e9, // last rows walk (performance.now() starts near 0 with the page)
      rn: 0, // nodes at that walk: a list that grew by a quarter is walked again sooner
      max: 0, // most runs of one effect in this second
      per: new WeakMap(), // effect → its runs in this second
      work: 0,
      off: false,
      nodes: () => nodes,
      trip(k, v) {
        x.off = true
        const e = Object.assign(Error(`B0908: ${W.f || C.f || C.name || 'a component'} crossed its ${k} budget (${v} > ${x.b[k]}); it was stopped to protect the page`), { code: 'B0908', budget: k, value: v, limit: x.b[k], comp: W.f || C.f || null })
        I.oops(e, null, 'B0908') // reported (brain-err, dev overlay, doctor); the boundary handles it below
        queueMicrotask(() => fall(e, 'B0908', STOPPED)) // after the run that tripped it
      },
    }
    const handler = (e, w) => (String(w).startsWith('on:') ? (toast(human(e)), report(e, w)) : fall(e, w))
    // requests started from this component's handlers (brain-data registers them in `$c`): cancelled with it
    const reqs = new Set()
    const build = () =>
      I.root(d => {
        dispose = d
        if (!G.__brainSSR) OWN.set(I.owner(), x)
        x.off = false
        I.guard(I.owner(), handler)
        fx(() => () => reqs.forEach(c => c.abort()))
        building = true
        pending = null
        try {
          nodes = I.nodes(C($p))
        } catch (e) {
          I.oops(e, null, 'render') // report only (no owner: the boundary itself handles it below)
          pending = [e, 'render']
        } finally {
          building = false
        }
        // the render threw, or one of its effects failed during it: the half-built nodes never reach the page
        if (pending) {
          d()
          dispose = null
          const now = Date.now()
          fails = [...fails.filter(t => now - t < WINDOW), now]
          nodes = notice(fails.length < LIMIT)
          report(pending[0], pending[1])
          pending = null
        }
        for (const n of nodes) if (n.nodeType === 1) (n.$g = handler), (n.$c = reqs)
      }, own)
    function again() {
      const p = end.parentNode
      I.rm(nodes)
      build()
      if (p) for (const n of nodes) p.insertBefore(n, end)
    }
    build()
    // when the parent unmounts this component, whatever it shows now (content or fallback) goes with it
    if (own)
      I.root(() => fx(() => () => {
        if (dispose) dispose()
        for (const n of nodes) if (n.nodeType === 1) (n.$g = null), (n.$c = null) // see brain-dev: no node → instance links
        I.rm(nodes)
      }), own)
    return [...nodes, end]
  }
  // every static the compiler puts on the component (h, f, p, form, seo, head…) stays visible on the wrapper
  return Object.assign(W, C, { C })
}
