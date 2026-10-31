// BRAINLLM runtime (SPEC §6). Push-pull signals with coloring: a write marks, a read recomputes only what is dirty.
// N: _ value · f fn · e effect · s state (0 clean 1 check 2 dirty 3 dead) · S sources · o observers
// S and o hold pairs [node, index of the link on the other side] (O(1) unlink) · c children · x index in p.c
// k cleanup · p owner. S/o/c start null (RAM). Methods: m mark · u update
// · r run · q clean · n unlink · d dispose. Short names: brain-min does not rename properties.

// L tracker · O owner · Q pending effects · B batch depth · P flush scheduled · H dev hook · E error hook · V provider (brain-ch)
let L = null, O = null, Q = [], B = 0, P = 0, H = null, E = null, V, A = null, U
// Errors (INTERNALS §18): fx, drv and handlers are caught and the app keeps running. E (brain-err.js, _err) records
// them and applies per-component fallbacks; without it, console.error.
// Errors go to the rich catcher (brain-err) or the console, then to the nearest component boundary (§42, brain-guard.js):
// up the owner chain for fx/drv, up the DOM for handlers.
const oops = (e, x, w) => {
  if (e && e.kind == 'aborted') return // a cancelled request (brain-data): never an error to report or show
  E ? E(e, x, w) : H ? globalThis.reportError ? reportError(e) : console.error(e) : console.error('BRAIN_ERROR: Operation failed. Try again.')
  for (let o = x; o; o = o.nodeType ? o.parentNode : o.p) {
    const g = o.$g
    if (g) return g(e, w)
  }
}

const adopt = (p, n) => {
  n.p = p
  if (p) n.x = (p.c || (p.c = [])).push(n) - 1
}

class N {
  constructor(v, f, e) {
    this._ = v
    this.f = f
    this.e = e
    this.s = f ? 2 : 0
    this.S = null
    this.o = null
    this.c = null
    this.k = null
    this.p = null
    this.x = 0
    if (f) adopt(O, this)
  }
  get v() {
    const l = L
    if (l && l != this) {
      const s = l.S || (l.S = [])
      if (s[s.length - 2] != this && !s.includes(this)) {
        const o = this.o || (this.o = [])
        s.push(this, o.length)
        o.push(l, s.length - 2)
      }
    }
    if (this.f) this.u()
    return this._
  }
  set v(x) {
    if (this.f) throw Error('drv is read-only')
    if (!Object.is(x, this._)) {
      this._ = x
      const o = this.o
      if (o) for (let i = 0; i < o.length; i += 2) o[i].m(2)
      if (!B && !P) (P = 1), queueMicrotask(flush)
    }
  }
  m(s) {
    if (this.s < s) {
      if (this.e && !this.s) Q.push(this)
      this.s = s
      const o = this.o
      if (o) for (let i = 0; i < o.length; i += 2) o[i].m(1)
    }
  }
  u() {
    const S = this.S
    if (this.s == 1 && S) for (let i = 0; i < S.length; i += 2) if ((S[i].u(), this.s == 2)) break
    if (this.s == 2) this.r()
    if (this.s < 3) this.s = 0
  }
  r() {
    const pl = L, po = O, old = this._
    this.q()
    this.n()
    L = O = this
    let r, x = 0
    try {
      r = this.f()
    } catch (e) {
      ;(x = 1), (r = old), oops(e, this, this.e ? 'fx' : 'drv')
    } finally {
      L = pl
      O = po
    }
    if (x) return
    if (this.e) {
      if (typeof r == 'function') this.k = r
    } else if (r !== old) {
      this._ = r
      const o = this.o
      if (o) for (let i = 0; i < o.length; i += 2) if (o[i].s < 3) o[i].s = 2
    }
  }
  q() {
    const c = this.c
    if (c) {
      this.c = null // children see c=null and skip removing themselves one by one
      for (const n of c) n.d()
    }
    const f = this.k
    if (f) (this.k = null), f()
  }
  n() {
    const S = this.S
    if (S) {
      // swap-remove: the last pair of s.o fills the gap and its index in l.S is fixed
      for (let j = 0; j < S.length; j += 2) {
        const o = S[j].o, k = S[j + 1], ly = o.pop(), l = o.pop()
        if (k < o.length) (o[k] = l), (o[k + 1] = ly), (l.S[ly + 1] = k)
      }
      S.length = 0 // reused on the next run
    }
  }
  d() {
    if (this.s < 3) {
      this.s = 3
      const c = this.p && this.p.c
      if (c) {
        const l = c.pop()
        if (l != this) (c[this.x] = l), (l.x = this.x)
      }
      this.q()
      this.n()
    }
  }
}

// Owners first (guard-first, as Solid and Svelte): an {#if} re-evaluates its condition, and disposes its block, before
// an effect inside the block runs, even when the inner effect was queued first.
const up = n => (n.p && up(n.p), n.u())
function flush() {
  P = 0
  B++
  let c = null
  try {
    for (let i = 0; i < Q.length; i++) {
      const n = Q[i]
      if (i > 100) {
        // B0902: an effect re-run >100 times in one flush is cut off (stays dirty, not requeued)
        const k = (c || (c = new Map())).get(n) + 1 || 1
        c.set(n, k)
        if (k > 100) {
          if (k < 102) oops(Object.assign(Error('B0902'), { code: 'B0902', fx: n }), n, 'B0902')
          continue
        }
      }
      up(n)
    }
  } finally {
    Q = []
    B--
  }
}

export const sig = (v, k) => (U && U('signals', O, 1), k && H ? H(new N(v, null, 0), k) : new N(v, null, 0))
export const drv = f => new N(undefined, f, 0)
export const fx = f => {
  const n = new N(undefined, f, 1)
  n.u()
  return () => n.d()
}
export const batch = f => {
  B++
  try {
    return f()
  } finally {
    if (!--B) flush()
  }
}
export const untrack = f => {
  const p = L
  L = null
  try {
    return f()
  } finally {
    L = p
  }
}

// Root owner: isolates from tracking and returns what f builds; f receives its dispose.
function root(f, p) {
  const r = new N(), pl = L, po = O
  adopt(p, r)
  L = null
  O = r
  try {
    return f(() => r.d())
  } finally {
    L = pl
    O = po
  }
}
// Internals for brain-dev.js: installs the named-sig hook; returns root and the current owner.
export const _dev = h => ((H = h), _rt())
// RPC (INTERNALS §11.1): codegen stub `(...a) => rpc("<hash>/save", a)`; the client is brain-rpc.js, loaded on the
// first call (#226: only pages that call the server pay for it).
export const rpc = (id, a) => import('./brain-rpc.js').then(m => m.rpc(id, a, oops, base()))
// Internals for sibling modules (brain-err, brain-ch): root, current owner and node helpers; _err installs the hook;
// brain-ch passes its provider for `mount(C, t, props, { provide })`.
// Optimistic writes (brain-data) and the circuit breaker (brain-guard) wrap N's prototype instead of a core hook (#288).
export const _rt = p => ((V = V || p), { root, owner: () => O, nodes, rm, oops, guard: (o, g) => (o.$g = g), adopt: a => (A = a), dev: (a, b) => H && H(a, b), development: () => !!H, budget: f => (U = f) })
export const _err = h => ((E = h), _rt())

// ---------- DOM ----------

const mk = p => p.appendChild(document.createComment(''))
// Yield to the browser: scheduler.yield (Chrome 129+), else a MessageChannel task (no 4 ms setTimeout clamp)
const later = f => {
  if (globalThis.scheduler?.yield) return scheduler.yield().then(f)
  const c = new MessageChannel()
  c.port1.onmessage = () => (c.port1.close(), f())
  c.port2.postMessage(0)
}
// a node with an exit transition (brain-anim.js `$out` → Promise) leaves once it has played
const rm = a => {
  for (const n of a) n.$out ? n.$out().then(() => n.remove()) : n.remove()
}
function nodes(v, a = []) {
  if (v == null || v === !!v) return a // null, undefined, true, false
  if (Array.isArray(v)) for (const x of v) nodes(x, a)
  else if (typeof v == 'function') nodes(v(), a)
  else if (v.nodeType == 11) a.push(...v.childNodes)
  else a.push(v.nodeType ? v : document.createTextNode(v))
  return a
}
function place(end, v) {
  const a = nodes(v), p = end.parentNode
  if (A) return A.p(end, a)
  for (const n of a) p.insertBefore(n, end)
  return a
}
// Writes reach the DOM only when the value changed (Object.is): a row whose object changed rewrites only its new fields.
const bind = (f, s) => {
  if (typeof f != 'function') return s(f)
  let last = bind
  fx(() => {
    const v = f()
    if (!Object.is(v, last)) s((last = v))
  })
}

export function tpl(h) {
  let n
  return () => {
    if (!n) {
      const t = document.createElement('template')
      t.innerHTML = h
      const c = t.content
      n = c.childNodes.length == 1 ? c.firstChild : c
    }
    // A: SSR stamps the clone, hydration adopts the node the server rendered (#285)
    return A ? A.t(n) : n.cloneNode(true)
  }
}
export function insert(p, f, end) {
  end = end || mk(p)
  if (typeof f != 'function') return void place(end, f)
  let cur = [], last = insert
  fx(() => {
    const v = f()
    if (Object.is(v, last)) return
    last = v
    if (cur.length == 1 && cur[0].nodeType == 3 && (typeof v == 'string' || typeof v == 'number')) cur[0].data = v
    else
      untrack(() => {
        rm(cur)
        cur = place(end, v)
      })
  })
}

// aria-* and data-* are enumerated, not boolean: true and false render as text (k[4] is their dash)
export function attr(el, k, f) {
  bind(
    f,
    k.startsWith('style:')
      ? ((k = k.slice(6)), v => (v == null || v === false ? el.style.removeProperty(k) : el.style.setProperty(k, v)))
      : /^(value|checked|selected)$/.test(k) && !(k == 'value' && el.tagName == 'OPTION') // an option's value is its attribute (SSR, value="")
        ? v => {
            if (el[k] !== v) el[k] = v
          }
        : v => (v == null || (v === false && k[4] != '-') ? el.removeAttribute(k) : el.setAttribute(k, v === true && k[4] != '-' ? '' : /^(href|src|action|formaction|poster)$/.test(k) && typeof v == 'string' ? inBase(v) : v))
  )
}
// Base path (§23 #148): <meta name="brain-base"> in the browser, globalThis.__brainBase while server.mjs renders.
// A computed app URL ('/posts/7') gets it; literal ones already have it from the build.
const base = (m = document.querySelector('meta[name=brain-base]')) => globalThis.__brainBase || (m ? m.getAttribute('content') : '/')
// javascript:, vbscript: and data: (other than data:image/) in a bound URL are dropped and reported (B0213, §31)
const inBase = (v, b = base()) =>
  /^\s*(javascript|vbscript|data(?!:image\/)):/i.test(v)
    ? (oops(Error('B0213 blocked URL scheme')), '')
    : v[0] == '/' && v[1] != '/' && !(v + '/').startsWith(b) ? b + v.slice(1) : v

export const cls = (el, k, f) => bind(f, v => el.classList.toggle(k, !!v))

// Delegated: walks composedPath (crosses shadow roots). brain-wc.js reuses it in shadow roots (_ev).
export function _ev(e) {
  const k = '$' + e.type
  let n
  batch(() => {
    for (n of e.composedPath()) {
      const h = n[k]
      if (h) {
        // a data property, not a getter: V8 keeps accessors in the event's shared hidden class, so a getter closure
        // would keep the first event (and its component) alive for the life of the page
        Object.defineProperty(e, 'currentTarget', { configurable: true, value: n })
        try {
          // n moves on with the loop: keep the element for a later rejection
          const r = h.call(n, e), el = n
          if (r && r.then) r.then(null, x => oops(x, el, 'on:' + e.type)) // async handler: a rejected await too
        } catch (x) {
          oops(x, n, 'on:' + e.type)
        }
        if (e.cancelBubble) break
      }
    }
  })
}
export function on(el, t, h) {
  if (/^(focus|blur|mouseenter|mouseleave|load|error|scroll)$/.test(t)) return el.addEventListener(t, e => batch(() => h(e)))
  el['$' + t] = h
  document.addEventListener(t, _ev) // once per type: the DOM ignores the same listener added again
}

// Indices of array `a` (−1 = new) that form the longest increasing subsequence.
function lis(a) {
  const p = a.slice(), r = []
  for (let i = 0; i < a.length; i++) {
    const x = a[i]
    if (x < 0) continue
    let lo = 0, hi = r.length
    while (lo < hi) {
      const m = (lo + hi) >> 1
      a[r[m]] < x ? (lo = m + 1) : (hi = m)
    }
    p[i] = lo ? r[lo - 1] : -1
    r[lo] = i
  }
  const f = new Uint8Array(a.length)
  for (let i = r.length ? r[r.length - 1] : -1; i >= 0; i = p[i]) f[i] = 1
  return f
}

// render(item, i) receives signals (`item.v`, `i.v`): same key with a new object updates in place.
export function list(p, f, key, render, end, empty) {
  end = end || mk(p)
  const own = O
  let rows = new Map(), order = [], emp = null
  fx(() => {
    const items = f() || [], me = O
    if (U) U('rows', me, items.length)
    untrack(() => {
      // Large updates (>500 rows, some already on screen) create new rows in ≤16 ms slices (one frame; QA #302: 5 ms
      // split a 1,000-row update into extra frames): the rest is skipped and this fx re-runs after a yield (INP). A first render (no rows yet) is built at once: slicing it only re-walks the list.
      const first = !order.length, next = [], map = new Map(), pa = end.parentNode, big = items.length > 500 && !first, t0 = big && performance.now()
      let cut = 0
      order.forEach((r, j) => (r.o = j))
      for (let i = 0; i < items.length; i++) {
        const it = items[i], k = key(it, i)
        if (map.has(k)) throw Error('duplicate key in each: ' + k)
        let r = rows.get(k)
        if (r) rows.delete(k), (r.s.v = it), (r.i.v = i)
        else if (cut || (big && performance.now() - t0 > 16)) {
          cut = 1
          continue
        } else r = root(d => {
          const s = sig(it), x = sig(i)
          return { s, i: x, n: nodes(render(s, x)), d, o: -1 }
        }, own)
        next.push(r)
        map.set(k, r)
      }
      if (cut) later(() => (me.m(2), P || B || ((P = 1), queueMicrotask(flush))))
      for (const r of rows.values()) r.d(), rm(r.n)
      if (first && next.length && A) A.p(end, next.flatMap(r => r.n))
      else if (first && next.length) {
        // nothing to keep or move: one fragment, one insertion
        const fr = document.createDocumentFragment()
        for (const r of next) for (const n of r.n) fr.appendChild(n)
        pa.insertBefore(fr, end)
      } else {
      const keep = lis(next.map(r => r.o)), mv = pa.moveBefore && pa.isConnected
      for (let i = next.length - 1, ref = end; i >= 0; i--) {
        const r = next[i]
        // moveBefore keeps focus, iframes, animations and state of moved rows (Chrome 133+); new rows use insertBefore
        if (!keep[i]) for (const n of r.n) mv && r.o >= 0 ? pa.moveBefore(n, ref) : pa.insertBefore(n, ref)
        ref = r.n[0] || ref
      }
      }
      rows = map
      order = next
      if (!items.length && empty && !emp) emp = root(d => ({ n: place(end, empty()), d }), own)
      else if (items.length && emp) emp.d(), rm(emp.n), (emp = null)
    })
  })
}

export function when(p, c, a, b, end) {
  end = end || mk(p)
  const t = drv(() => !!c())
  fx(() => {
    const g = t.v ? a : b
    const n = g ? untrack(() => place(end, g())) : []
    return () => rm(n)
  })
}

// Tests replace services/context: `mount(C, el, props, { provide: [[Api, fake]] })` (needs brain-ch.js).
export function mount(C, t, props = {}, o) {
  return root(d => {
    const n = nodes(o && o.provide ? V(o.provide, () => C(props)) : C(props))
    for (const x of n) t.appendChild(x)
    return () => (d(), rm(n))
  }, null)
}

// Islands (SPEC §6.1) live in brain-island.js: only pages with islands load it. It receives what it needs from the
// core instead of importing it (hashed chunks cannot import each other in a cycle).
export const hydrate = r => import('./brain-island.js').then(m => m.hydrate(r, { mount, _rt }))
