// BRAINLLM inspector (SPEC §9 and §9.1). Dev only; absent from builds. Emits data, does not draw the debugger.
// Usage (brain-serve HMR client): inspect({ send: s => ws.send(s), inspectPort }) → every 100 ms, if there is data,
// sends `{"t":"m","v":2,"seq":n,"at":ms,"d":[records…]}`. Record format: see FORMAT at the end.
// Does not touch the core: uses the brain-dev.js registry and patches the signal prototype in dev (`set v`, `r`).
import { live, inst, bus, owner } from './brain-dev.js'
import { capture, sub } from './brain-err.js'
import { seen } from './brain-ch.js'

const G = globalThis
const RT = new URL('.', import.meta.url).href // runtime frames: do not count as an app line
const O = {
  si: G.setInterval, ci: G.clearInterval, st: G.setTimeout, ct: G.clearTimeout,
  raf: G.requestAnimationFrame, caf: G.cancelAnimationFrame, fetch: G.fetch,
}
const now = () => Date.now()
const pnow = () => (G.performance ? performance.now() : now())
const syncHeap = () => (G.performance && performance.memory ? performance.memory.usedJSHeapSize : G.process && process.memoryUsage ? process.memoryUsage().heapUsed : null)

let out = [], seq = 0, opts = {}, started = false
const push = r => out.push(r)

// ---------- instance identity and current owner ----------
const ids = new WeakMap()
let nextId = 1
const idOf = i => ids.get(i) || (ids.set(i, nextId++), ids.get(i))
// the component's own name: under guard() (every compiled component) i.C is its wrapper `W`, the component is i.C.C
const nameOf = i => (i.C && ((i.C.C && i.C.C.name) || i.C.name)) || i.h
function instOf(n) {
  for (; n; n = n.p) {
    const i = inst.get(n)
    if (i) return i
  }
  return null
}
let handler = null // { i, o:'on:click' } while a delegated handler runs
// Current owner: component + origin (fx · setup · on:<event>). load runs on the server: brain-serve attributes it.
function ctx() {
  const n = owner()
  if (n && n.e) {
    const i = instOf(n)
    if (i) return { i, o: 'fx', n }
  }
  if (handler) return handler
  const i = n && instOf(n)
  return i ? { i, o: 'setup', n } : null
}
const own = c => (c ? { id: idOf(c.i), h: c.i.h, name: nameOf(c.i), o: c.o } : null)
// First frame outside the runtime: "url:line:col". No app frame → foreign (extension, third party, browser).
function site() {
  const lim = Error.stackTraceLimit
  Error.stackTraceLimit = 50 // with 10, the runtime stack hides the app frame
  const st = String(new Error().stack || '').split('\n')
  Error.stackTraceLimit = lim
  for (const l of st) {
    const m = l.match(/((?:https?|file|chrome-extension|moz-extension):\/\/[^\s)]+):(\d+):(\d+)/)
    if (!m || (m[1].startsWith(RT) && !m[1].slice(RT.length).includes('/'))) continue // only root-level brain*.js
    const app = G.location && /^https?:/.test(G.location.href) ? m[1].startsWith(G.location.origin) && !m[1].includes('/@brain/') : m[1].startsWith('file:')
    return { line: m[1].replace(G.location ? G.location.origin : '', '') + ':' + m[2], foreign: !app }
  }
  return { line: null, foreign: true }
}
// Safe preview of a value (depth and length limits).
function prev(v, d = 0) {
  if (v == null || typeof v === 'number' || typeof v === 'boolean') return v
  if (typeof v === 'string') return v.length > 120 ? v.slice(0, 117) + '…' : v
  if (typeof v === 'function') return 'ƒ'
  if (v.nodeType) return '<' + (v.nodeName || 'node').toLowerCase() + '>'
  if (d > 1) return Array.isArray(v) ? `[… ${v.length}]` : '{…}'
  if (Array.isArray(v)) return v.slice(0, 10).map(x => prev(x, d + 1)).concat(v.length > 10 ? [`… +${v.length - 10}`] : [])
  const o = {}
  let k = 0
  for (const key in v) if (k++ < 12) o[key] = prev(v[key], d + 1)
  return o
}

// ---------- signals: last writer, count and update latency ----------
const writes = new WeakMap() // N → { w, c }
let upd = null // update in progress: { at, t0, sig, w, fx, ms }
let mo = null, touched = 0
function patchSignals(sample) {
  const P = Object.getPrototypeOf(sample)
  if (P.$bi) return
  P.$bi = 1
  const d = Object.getOwnPropertyDescriptor(P, 'v')
  Object.defineProperty(P, 'v', {
    configurable: true,
    get: d.get,
    set(x) {
      const before = this._
      d.set.call(this, x)
      if (this._ === before) return
      const c = ctx(), s = site(), w = c ? `${nameOf(c.i)} · ${c.o}` : s.line || '?'
      const r = writes.get(this) || (writes.set(this, { w, c: 0 }), writes.get(this))
      r.w = w
      r.c++
      if (!upd) {
        upd = { at: now(), t0: pnow(), sig: sigName(this), w, fx: 0, ms: 0 }
        if (mo) mo.takeRecords()
        touched = 0
        queueMicrotask(closeUpd) // queued after the core flush
      }
    },
  })
  const run = P.r
  P.r = function () {
    if (!this.e) return run.call(this)
    const t = pnow()
    try {
      return run.call(this)
    } finally {
      if (upd) upd.fx++
      const i = instOf(this)
      if (i) (i.$fx = (i.$fx || 0) + 1), (i.$ms = (i.$ms || 0) + pnow() - t)
    }
  }
}
function closeUpd() {
  if (!upd) return
  const u = upd
  upd = null
  if (mo) touched += mo.takeRecords().length
  push({ k: 'upd', at: u.at, sig: u.sig, w: u.w, fx: u.fx, nodes: touched, ms: +(pnow() - u.t0).toFixed(3) })
}
function sigName(n) {
  for (const set of live.values()) for (const i of set) if (i.m) for (const [k, v] of i.m) if (v === n) return `${nameOf(i)}.${k}`
  return 'sig'
}

// ---------- delegated handlers: capture on document, close on window ----------
function patchHandlers() {
  const D = G.document, W = G.window
  if (!D || !W || D.$bi) return
  D.$bi = 1
  let t0 = 0
  const close = t => {
    if (handler) push({ k: 'handler', at: now(), id: idOf(handler.i), h: handler.i.h, name: nameOf(handler.i), type: t, ms: +(pnow() - t0).toFixed(3) })
    handler = null
  }
  const types = ['click', 'input', 'change', 'submit', 'keydown', 'keyup', 'pointerdown', 'pointerup', 'focusin', 'focusout']
  for (const t of types) {
    D.addEventListener.call(D, t, e => {
      let n = e.composedPath ? e.composedPath()[0] : e.target
      for (; n && !n.$i; n = n.parentNode || n.host);
      handler = n ? { i: n.$i, o: 'on:' + t } : null
      t0 = pnow()
      if (handler) O.st.call(G, () => close(t), 0) // fallback if a handler stops propagation
    }, true)
    W.addEventListener.call(W, t, () => close(t))
  }
}

// ---------- timers ----------
const timers = new Map() // id → active public record
const tmeta = new Map() // id → { h: handle, n: owning fx } (internal: never serialized)
let tid = 1
function timer(kind, ms) {
  const c = ctx(), s = site(), id = tid++
  const r = { k: 'timer', id, kind, ms: ms || 0, own: own(c), line: s.line, foreign: s.foreign, cleanup: null, st: 'on' }
  timers.set(id, r)
  tmeta.set(id, { h: null, n: c && c.o === 'fx' ? c.n : null })
  push(r)
  return r
}
function patchTimers() {
  const mk = (name, kind, orig, once) =>
    function (cb, ms, ...a) {
      const r = timer(kind, kind === 'raf' ? 16 : ms)
      const h = orig.call(G, typeof cb === 'function' && once ? (...x) => (end(r), cb(...x)) : cb, ms, ...a)
      tmeta.get(r.id).h = h
      byHandle.set(h, r)
      if (tmeta.get(r.id).n) O.st.call(G, () => fxCleanup(r), 0)
      return h
    }
  const byHandle = new Map()
  const end = r => {
    if (r.st === 'off') return
    r.st = 'off'
    timers.delete(r.id)
    const m = tmeta.get(r.id)
    if (m) byHandle.delete(m.h), tmeta.delete(r.id)
    push({ k: 'timer', id: r.id, st: 'off' })
  }
  const clr = orig =>
    function (h) {
      const r = byHandle.get(h)
      if (r) end(r)
      return orig.call(G, h)
    }
  if (G.setTimeout.$bi) return
  G.setTimeout = mk('setTimeout', 'timeout', O.st, true)
  G.setInterval = mk('setInterval', 'interval', O.si, false)
  G.clearTimeout = clr(O.ct)
  G.clearInterval = clr(O.ci)
  if (O.raf) (G.requestAnimationFrame = mk('requestAnimationFrame', 'raf', O.raf, true)), (G.cancelAnimationFrame = clr(O.caf))
  G.setTimeout.$bi = 1
}
// Timer created in an fx: cleanup = the fx returned a function. An interval without cleanup is B0602.
function fxCleanup(r) {
  const m = tmeta.get(r.id), n = m && m.n
  if (m) m.n = null
  r.cleanup = !!(n && n.k)
  push({ k: 'timer', id: r.id, cleanup: r.cleanup })
  if (!r.cleanup && r.kind === 'interval' && r.st === 'on') b602(n, 'setInterval', r.line)
}

// ---------- B0602 and B0601 ----------
const leaks = []
const flagged = new WeakSet()
const b602ByInst = new WeakMap()
function leak(r) {
  if (leaks.some(l => l.code === r.code && l.h === r.h && l.msg === r.msg)) return
  r.k = 'leak'
  r.at = now()
  leaks.push(r)
  push(r)
}
function b602(n, what, line) {
  if (!n || flagged.has(n) || n.s >= 3) return
  flagged.add(n)
  const i = instOf(n), name = i ? nameOf(i) : '?'
  const fx = `fx in ${name} (${what})${line ? ' ' + line : ''}`
  if (i) (b602ByInst.get(i.r) || b602ByInst.set(i.r, []).get(i.r)).push(fx)
  leak({ code: 'B0602', h: i ? i.h : null, name, file: i && i.C.f, line, msg: `global ${what} inside an fx without cleanup`, fix: 'return a cleanup function from the fx: `fx(() => { …; return () => remove() })`', fx: [fx] })
}
function patchGlobalListeners() {
  for (const t of [G.window, G.document].filter(Boolean)) {
    const add = t.addEventListener
    if (!add || add.$bi) continue
    t.addEventListener = function (type, ...a) {
      const c = ctx()
      if (c && c.o === 'fx') {
        const line = site().line
        O.st.call(G, () => !c.n.k && b602(c.n, `addEventListener('${type}')`, line), 0)
      }
      return add.call(this, type, ...a)
    }
    t.addEventListener.$bi = 1
  }
}
// GC cycles that can collect an old object (#295). A fresh object dies in any GC, a minor one included, and minor GCs
// never touch the promoted nodes an unmounted instance leaves behind: counting them flagged clean instances as leaks
// (B0601) whenever a page allocated between two checks. So each GC (seen by a fresh probe) ages a few strongly held
// sentinels; one that survived 2 GCs is promoted (V8 promotes on the second) and is then released: only a major GC collects it. Every release
// dying in the same GC counts once.
let gcs = 0, counted = false
const held = [[{}, 0]] // [object, GCs survived]; one from the start, so the first GCs already count
const major = G.FinalizationRegistry
  ? new FinalizationRegistry(() => {
      if (!counted) (counted = true), queueMicrotask(() => ((counted = false), gcs++))
    })
  : null
const probe = G.FinalizationRegistry
  ? new FinalizationRegistry(() => {
      for (let i = held.length - 1; i >= 0; i--) if (++held[i][1] >= 2) major.register(held.splice(i, 1)[0][0], 0)
      held.push([{}, 0])
      arm()
    })
  : null
const arm = () => probe && probe.register({}, 0)
const watched = []
// No deref(): reading a WeakRef keeps the target alive for the job (KeepDuringJob) and, checking every
// second, the inspector would cause the leak it measures. The FinalizationRegistry marks the record on collection.
const fin = G.FinalizationRegistry ? new FinalizationRegistry(w => (w.dead = 1)) : null
function watch(d) {
  if (!fin) return
  // its first DOM node: an app that keeps a detached node (a module array, a cache) is the leak to catch. The browser
  // also keeps the last hovered or focused node for a while: `brain test --heap` moves the pointer away and blurs before
  // collecting. Unmounted nodes no longer point at their instance (brain-dev, brain-guard), so a kept node is small.
  const target = (d.n && d.n.find(x => x.nodeType === 1)) || d.r
  if (!target) return
  const w = { dead: 0, h: d.h, name: nameOf(d), file: (d.C && d.C.f) || null, at: gcs, fx: (d.r && b602ByInst.get(d.r)) || [] }
  fin.register(target, w)
  watched.push(w)
}
function checkLeaks() {
  for (let j = watched.length - 1; j >= 0; j--) {
    const w = watched[j]
    if (w.dead) watched.splice(j, 1)
    else if (gcs - w.at >= 2) {
      watched.splice(j, 1)
      leak({ code: 'B0601', h: w.h, name: w.name, file: w.file, msg: `unmounted ${w.name} instance still alive after 2 GC cycles`, fix: w.fx.length ? 'add cleanup to the listed fx: they retain the instance' : 'look for external references to its nodes or signals (global listeners, caches, module closures)', fx: w.fx })
    }
  }
}

// ---------- network: fetch and XHR ----------
let rid = 1
function patchNet() {
  if (O.fetch && !G.fetch.$bi) {
    G.fetch = function (input, init) {
      const c = ctx(), s = site(), t0 = pnow()
      const r = { k: 'net', id: rid++, kind: 'fetch', m: (init && init.method) || (input && input.method) || 'GET', url: String((input && input.url) || input), own: own(c), line: s.line, foreign: s.foreign, at: now(), st: null, ms: null, b: null }
      push(r)
      return O.fetch.call(this, input, init).then(
        res => {
          const len = res.headers && res.headers.get && res.headers.get('content-length')
          push({ k: 'net', id: r.id, st: res.status, ms: +(pnow() - t0).toFixed(1), b: len == null ? null : +len })
          return res
        },
        err => (push({ k: 'net', id: r.id, st: 0, ms: +(pnow() - t0).toFixed(1), err: String(err) }), Promise.reject(err))
      )
    }
    G.fetch.$bi = 1
  }
  const X = G.XMLHttpRequest && G.XMLHttpRequest.prototype
  if (X && !X.$bi) {
    X.$bi = 1
    const open = X.open, send = X.send
    X.open = function (m, url, ...a) {
      this.$r = { m, url: String(url) }
      return open.call(this, m, url, ...a)
    }
    X.send = function (body) {
      const c = ctx(), s = site(), t0 = pnow(), r = { k: 'net', id: rid++, kind: 'xhr', ...(this.$r || {}), own: own(c), line: s.line, foreign: s.foreign, at: now(), st: null, ms: null, b: null }
      push(r)
      this.addEventListener('loadend', () => push({ k: 'net', id: r.id, st: this.status, ms: +(pnow() - t0).toFixed(1), b: typeof this.responseText === 'string' ? this.responseText.length : null }))
      return send.call(this, body)
    }
  }
}

// ---------- storage: cookie, Storage, IndexedDB, Cache Storage ----------
const storeKey = new Map() // `${area}:${key}` → last record (avoids repeating reads)
function store(area, key, op, extra) {
  const c = ctx(), s = site(), id = area + ':' + key
  if (op === 'get' && storeKey.has(id) && storeKey.get(id).own && storeKey.get(id).own.id === (c && idOf(c.i))) return
  const r = { k: 'store', area, key, op, own: own(c), line: s.line, foreign: s.foreign, at: now(), ...extra }
  storeKey.set(id, r)
  push(r)
}
function patchStorage() {
  const D = G.Document && G.Document.prototype
  const cd = D && Object.getOwnPropertyDescriptor(D, 'cookie')
  if (cd && !cd.set.$bi) {
    const set = function (v) {
      const [pair, ...attrs] = String(v).split(';')
      const eq = pair.indexOf('='), name = pair.slice(0, eq).trim(), val = pair.slice(eq + 1).trim()
      const fl = {}
      for (const a of attrs) {
        const [k, x] = a.split('=').map(s => s.trim())
        fl[k.toLowerCase()] = x ?? true
      }
      store('cookie', name, 'set', { v: val, flags: { secure: !!fl.secure, sameSite: fl.samesite || null, path: fl.path || null }, exp: fl.expires || (fl['max-age'] != null ? `max-age=${fl['max-age']}` : 'session') })
      cd.set.call(this, v)
    }
    set.$bi = 1
    Object.defineProperty(D, 'cookie', {
      configurable: true,
      get() {
        const v = cd.get.call(this)
        for (const p of v ? v.split('; ') : []) store('cookie', p.slice(0, p.indexOf('=')), 'get')
        return v
      },
      set,
    })
  }
  const S = G.Storage && G.Storage.prototype
  if (S && !S.$bi) {
    S.$bi = 1
    const area = st => (st === G.localStorage ? 'local' : 'session')
    for (const [m, op] of [['setItem', 'set'], ['getItem', 'get'], ['removeItem', 'remove']]) {
      const f = S[m]
      S[m] = function (k, v) {
        store(area(this), String(k), op, op === 'set' ? { v: prev(String(v)) } : {})
        return f.apply(this, arguments)
      }
    }
    const clear = S.clear
    S.clear = function () {
      store(area(this), '*', 'remove')
      return clear.call(this)
    }
  }
  if (G.indexedDB && !G.indexedDB.$bi) {
    G.indexedDB.$bi = 1
    const open = G.indexedDB.open
    G.indexedDB.open = function (name, ver) {
      store('idb', String(name), 'open', { ver: ver ?? null })
      dbs.add(String(name))
      return open.apply(this, arguments)
    }
  }
  if (G.caches && !G.caches.$bi) {
    G.caches.$bi = 1
    const open = G.caches.open
    G.caches.open = function (name) {
      store('cache', String(name), 'open')
      cacheNames.add(String(name))
      return open.apply(this, arguments)
    }
  }
}
const dbs = new Set(), cacheNames = new Set()
// Every 5 s: stores, count and a sample of records from the databases and caches the app opened.
async function scanStores() {
  for (const name of dbs) {
    const req = G.indexedDB.open.call(G.indexedDB, name) // no version: does not trigger upgrade
    const db = await new Promise(r => ((req.onsuccess = () => r(req.result)), (req.onerror = () => r(null))))
    if (!db) continue
    for (const sname of db.objectStoreNames) {
      const os = db.transaction(sname, 'readonly').objectStore(sname)
      const [count, sample] = await Promise.all([
        new Promise(r => (os.count().onsuccess = e => r(e.target.result))),
        new Promise(r => (os.getAll(undefined, 3).onsuccess = e => r(e.target.result))),
      ])
      push({ k: 'idb', db: name, store: sname, count, sample: prev(sample) })
    }
    db.close()
  }
  for (const name of cacheNames) {
    const c = await G.caches.open.call(G.caches, name)
    push({ k: 'cachestore', name, count: (await c.keys()).length })
  }
}

// ---------- connections: WebSocket, EventSource, BroadcastChannel, Worker ----------
let cid = 1
function wrapCtor(name, kind, urlOf) {
  const C = G[name]
  if (!C || C.$bi) return
  const W = new Proxy(C, {
    construct(T, args, NT) {
      const o = Reflect.construct(T, args, NT)
      const c = ctx(), s = site()
      const r = { k: 'conn', id: cid++, kind, url: urlOf(args), own: own(c), line: s.line, foreign: s.foreign, st: 'opening', in: 0, out: 0, bin: 0, bout: 0 }
      push(r)
      const upd = patch => push({ k: 'conn', id: r.id, ...patch })
      const size = d => (typeof d === 'string' ? d.length : d && (d.byteLength ?? d.size)) || 0
      o.addEventListener('open', () => upd({ st: 'open' }))
      o.addEventListener('message', e => ((r.in++), (r.bin += size(e.data)), upd({ in: r.in, bin: r.bin })))
      o.addEventListener('error', () => upd({ st: 'error' }))
      o.addEventListener('close', () => upd({ st: 'closed' }))
      const send = o.send || o.postMessage
      const m = o.send ? 'send' : 'postMessage'
      if (send) o[m] = function (d, ...a) {
        r.out++
        r.bout += size(d)
        upd({ out: r.out, bout: r.bout })
        return send.call(this, d, ...a)
      }
      const close = o.close || o.terminate
      const cm = o.close ? 'close' : 'terminate'
      if (close) o[cm] = function () {
        upd({ st: 'closed' })
        return close.apply(this, arguments)
      }
      if (kind === 'bc' || kind === 'worker') upd({ st: 'open' })
      return o
    },
  })
  W.$bi = 1
  G[name] = W
}
function patchConns() {
  wrapCtor('WebSocket', 'ws', a => String(a[0]))
  wrapCtor('EventSource', 'es', a => String(a[0]))
  wrapCtor('BroadcastChannel', 'bc', a => String(a[0]))
  wrapCtor('Worker', 'worker', a => String(a[0]))
}

// ---------- tree, state, counts and graph per component ----------
const sent = new WeakMap() // instance → signature of the last sent state
function walk(r, self, f) {
  for (const c of r.c || []) {
    if (inst.has(c) && inst.get(c) !== self) continue
    f(c)
    walk(c, self, f)
  }
}
function countDom(n, acc, self) {
  if (n.nodeType === 1 && n.$i && n.$i !== self) return
  acc.nodes++
  if (n.nodeType === 1) for (const k in n) if (k[0] === '$' && k.length > 2 && typeof n[k] === 'function') acc.ev++
  for (let c = n.firstChild; c; c = c.nextSibling) countDom(c, acc, self)
}
// Estimated bytes from runtime/perf/heap.js measurements (sig 112 B, child fx 170 B) plus an approximate DOM cost.
const EST = { sig: 112, drv: 230, fx: 170, nodes: 160, ev: 40 }
// ---------- Jank (§19): Long Animation Frames (Chrome 123+) and Event Timing (INP per component) ----------
const inpBy = new Map() // component hash → worst interaction latency (ms)
const pathOf = u => {
  try {
    return new URL(u, G.location && G.location.href).pathname
  } catch {
    return String(u || '')
  }
}
// Component whose source file is served at this URL (dev serves `.brain` files at their path).
function compOfUrl(u) {
  const p = pathOf(u)
  for (const set of live.values())
    for (const i of set) {
      const f = i.C && (i.C.f || (i.C.C && i.C.C.f))
      if (f && p.endsWith(f)) return { h: i.h, name: nameOf(i) }
    }
  return null
}
export function onLoaf(list) {
  for (const e of list)
    push({
      k: 'jank', at: now(), ms: +e.duration.toFixed(1), block: +(e.blockingDuration || 0).toFixed(1),
      scripts: (e.scripts || []).map(x => ({ src: pathOf(x.sourceURL), fn: x.sourceFunctionName || null, inv: x.invoker || null, ms: +x.duration.toFixed(1), comp: compOfUrl(x.sourceURL) })),
    })
}
export function onEvents(list) {
  for (const e of list) {
    let i = null
    for (let x = e.target; x && !i; x = x.parentNode) i = x.$i || null
    const ms = Math.round(e.duration)
    if (i) inpBy.set(i.h, Math.max(inpBy.get(i.h) || 0, ms))
    push({ k: 'inp', at: now(), type: e.name, ms, delay: Math.round(e.processingStart - e.startTime), proc: Math.round(e.processingEnd - e.processingStart), comp: i ? { id: idOf(i), h: i.h, name: nameOf(i) } : null })
  }
}

export function comps() {
  const res = []
  for (const [h, set] of live) {
    if (!set.size) continue
    const a = { k: 'comp', h, name: null, f: null, inst: 0, sig: 0, drv: 0, fx: 0, ev: 0, nodes: 0, est: 0, runs: 0, ms: 0, inp: inpBy.get(h) || null }
    for (const i of set) {
      a.name = a.name || nameOf(i)
      a.f = a.f || i.C.f || null
      a.inst++
      a.sig += i.m ? i.m.size : 0
      if (i.r) walk(i.r, i, c => c.f && (c.e ? a.fx++ : a.drv++))
      for (const n of i.n || []) countDom(n, a, i)
      a.runs += i.$fx || 0
      a.ms += i.$ms || 0
    }
    a.fx = Math.max(0, a.fx - a.inst) // brain-dev internal fx (unregistration)
    a.ms = +a.ms.toFixed(2)
    a.est = a.sig * EST.sig + a.drv * EST.drv + a.fx * EST.fx + a.nodes * EST.nodes + a.ev * EST.ev
    res.push(a)
  }
  return res
}
function states() {
  for (const set of live.values())
    for (const i of set) {
      const sigs = [...(i.m || [])].map(([n, s]) => ({ n, v: prev(s._), w: (writes.get(s) || {}).w || null, c: (writes.get(s) || {}).c || 0 }))
      const drv = []
      if (i.r) walk(i.r, i, c => c.f && !c.e && drv.push(prev(c._)))
      let props = {}
      try {
        for (const k of Object.keys(i.$p || {})) props[k] = prev(i.$p[k])
      } catch {
        props = '?'
      }
      const r = { k: 'state', id: idOf(i), sigs, drv, props }
      const sig = JSON.stringify(r)
      if (sent.get(i) !== sig) sent.set(i, sig), push(r)
    }
}
function graphs() {
  for (const set of live.values()) {
    const i = set.values().next().value
    if (!i || !i.r) continue
    const nodes = [], idx = new Map(), edges = []
    const add = (n, k, name) => (idx.has(n) || (idx.set(n, nodes.length), nodes.push({ k, name })), idx.get(n))
    for (const [name, s] of i.m || []) add(s, 'sig', name)
    walk(i.r, i, c => c.f && add(c, c.e ? 'fx' : 'drv', null))
    for (const [n, j] of idx) {
      const S = n.S || []
      for (let x = 0; x < S.length; x += 2) if (idx.has(S[x])) edges.push([idx.get(S[x]), j])
      if (nodes.length > 200) break
    }
    push({ k: 'graph', h: i.h, nodes: nodes.slice(0, 200), edges })
  }
}

// ---------- heap ----------
let measuring = false, lastHeap = null
function sample() {
  let used = syncHeap(), src = used == null ? 'none' : G.performance && performance.memory ? 'memory' : 'node'
  if (!measuring && G.crossOriginIsolated && G.performance && performance.measureUserAgentSpecificMemory) {
    measuring = true
    performance.measureUserAgentSpecificMemory().then(
      m => ((measuring = false), push({ k: 'heap', at: now(), used: m.bytes, src: 'uasm' })),
      () => (measuring = false)
    )
  }
  if (used != null) push({ k: 'heap', at: now(), used, src })
  return used
}
function event(e, data) {
  const u = syncHeap()
  push({ k: 'ev', at: now(), e, dh: u != null && lastHeap != null ? u - lastHeap : null, ...data })
  if (u != null) lastHeap = u
}

// ---------- cycle ----------
let ticks = 0, lastTop = null
// SSR islands still inert (no first event): count per hash.
function islands() {
  if (!G.document || !G.document.querySelectorAll) return
  const inert = {}
  for (const s of G.document.querySelectorAll('script[type="b/state"]')) {
    const el = s.previousSibling
    const h = el && el.getAttribute && el.getAttribute('data-b')
    if (h) inert[h] = (inert[h] || 0) + 1
  }
  push({ k: 'islands', inert })
}
export function tick() {
  ticks++
  lastHeap = sample() ?? lastHeap
  checkLeaks()
  for (const c of comps()) push(c)
  states()
  if (ticks % 5 === 1) graphs(), islands(), dbs.size + cacheNames.size && scanStores().catch(() => {})
  return drain()
}
export function drain() {
  if (!out.length) return null
  const m = { t: 'm', v: 2, seq: seq++, at: now(), d: out }
  out = []
  if (opts.send) opts.send(JSON.stringify(m))
  return m
}

export function inspect(o) {
  if (started) return api // inspect() without args is read-only: does not overwrite send/options of the first call
  opts = o = o || {}
  started = true
  // Errors (§18): brain-err with dev attribution (instance → name, file and parent chain).
  capture({
    who(x) {
      let i = null
      if (x && x.nodeType) for (; x && !i; x = x.parentNode) i = x.$i || null
      else i = instOf(x)
      if (!i) return null
      const chain = []
      for (let j = i; j; j = instOf(j.p)) chain.push(nameOf(j))
      return { id: idOf(i), h: i.h, name: nameOf(i), file: (i.C && (i.C.f || (i.C.C && i.C.C.f))) || null, chain }
    },
    sigName,
    maps: true, // dev: errors point at file.brain:line through each module's inline source map
  })
  // Channels (§12): one record per new (component, kind, name, role) edge, for the Graph view. No owner is retained.
  const chans = new WeakMap()
  seen.add((kind, name, role, o) => {
    const i = instOf(o), key = `${kind}|${name}|${role}`, done = i && (chans.get(i) || (chans.set(i, new Set()), chans.get(i)))
    if (done && done.has(key)) return
    if (done) done.add(key)
    push({ k: 'chan', kind, name, role, id: i ? idOf(i) : null, h: i ? i.h : null, comp: i ? nameOf(i) : null })
  })
  sub.add(r => push({ k: 'err', fp: r.fp, code: r.code, msg: r.msg, where: r.where, comp: r.comp, line: r.line, src: r.src || null, stack: r.brainStack || r.stack, crumbs: r.crumbs, n: r.n, at: r.last }))
  patchTimers()
  patchGlobalListeners()
  patchHandlers()
  patchNet()
  patchStorage()
  patchConns()
  arm()
  if (G.MutationObserver && G.document) (mo = new MutationObserver(() => {})).observe(G.document, { subtree: true, childList: true, attributes: true, characterData: true })
  bus.add((k, d) => {
    if (k === 'mount') {
      if (d.m && d.m.size) patchSignals(d.m.values().next().value)
      const pi = instOf(d.p)
      if (!pi) lastTop = d
      push({ k: 'inst', id: idOf(d), h: d.h, name: nameOf(d), f: d.C.f || null, parent: pi ? idOf(pi) : null, island: null })
      event('mount', { id: idOf(d), h: d.h, ms: d.$mt == null ? null : +d.$mt.toFixed(2) })
      if (O.raf) {
        const t = pnow(), id = idOf(d)
        O.raf.call(G, () => event('paint', { id, h: d.h, ms: +(pnow() - t).toFixed(1) }))
      }
    } else if (k === 'dispose') {
      if (lastTop === d) lastTop = null // the inspector must not retain what it watches
      if (handler && handler.i === d) handler = null
      watch(d)
      if (ids.has(d)) push({ k: 'gone', id: idOf(d) })
      event('dispose', { h: d.h })
    } else if (k === 'hydrate') {
      if (lastTop) push({ k: 'inst', id: idOf(lastTop), island: 'hydrated' })
      event('hydrate', { url: d.url, id: lastTop ? idOf(lastTop) : null })
    }
    else event('hmr', { h: d.h, n: d.n })
  })
  if (G.PerformanceObserver)
    try {
      new PerformanceObserver(l => {
        for (const e of l.getEntries())
          if (/\/(_b|@brain)\//.test(e.name)) event('module', { url: new URL(e.name).pathname, ms: +e.duration.toFixed(1), b: e.encodedBodySize || null })
      }).observe({ type: 'resource', buffered: true })
    } catch {}
  const types = (G.PerformanceObserver && PerformanceObserver.supportedEntryTypes) || []
  if (types.includes('long-animation-frame')) new PerformanceObserver(l => onLoaf(l.getEntries())).observe({ type: 'long-animation-frame', buffered: true })
  if (types.includes('event')) new PerformanceObserver(l => onEvents(l.getEntries())).observe({ type: 'event', durationThreshold: 16, buffered: true })
  const ms = (v, d) => Math.min(v || d, 2147483647) // >int32: Node turns it into 1 ms
  const t1 = O.si.call(G, tick, ms(o.sample, 1000))
  const t2 = O.si.call(G, drain, ms(o.batch, 100))
  if (t1.unref) t1.unref(), t2.unref()
  if (G.document && G.document.addEventListener && !o.noPanel)
    G.document.addEventListener.call(G.document, 'keydown', e => e.altKey && e.shiftKey && e.code === 'KeyB' && toggle())
  return api
}
const api = { tick, drain, comps, onLoaf, onEvents, leaks: () => leaks.slice(), timers: () => [...timers.values()], toggle: () => toggle() }

// ---------- mini panel (Alt+Shift+B): heap, leaks and link to the full debugger (§9.1) ----------
let panel = null
const CSS = `:host{all:initial;position:fixed;right:16px;bottom:16px;z-index:2147483647;font:12px/1.45 system-ui,-apple-system,sans-serif;color:#1a1a1a}
.p{width:min(340px,calc(100vw - 32px));border-radius:16px;padding:12px 14px;background:rgba(255,255,255,.72);
-webkit-backdrop-filter:blur(20px) saturate(1.6);backdrop-filter:blur(20px) saturate(1.6);border:1px solid rgba(0,0,0,.08);
box-shadow:0 1px 3px rgba(16,24,40,.08),0 12px 32px rgba(16,24,40,.14)}@supports not (backdrop-filter:blur(1px)){.p{background:#fff}}
@media (prefers-color-scheme:dark){:host{color:#f2f2f2}.p{background:rgba(28,28,30,.8);border-color:rgba(255,255,255,.1)}}
.h{display:flex;justify-content:space-between;font-weight:600}.k{font-variant-numeric:tabular-nums;font-weight:400;opacity:.65}
.l{margin-top:8px;padding:6px 8px;border-radius:10px;background:rgba(255,59,48,.1)}.l b{color:#d70015}.ok{margin-top:8px;opacity:.6}
a{display:inline-block;margin-top:10px;color:#0071e3;text-decoration:none;font-weight:500}`
function toggle() {
  if (!panel) {
    const host = document.createElement('brain-inspect')
    const sr = host.attachShadow({ mode: 'open' })
    sr.innerHTML = `<style>${CSS}</style><div class="p"></div>`
    panel = { host, box: sr.lastChild, on: false }
  }
  panel.on = !panel.on
  if (!panel.on) return panel.host.remove()
  document.body.appendChild(panel.host)
  const u = syncHeap(), port = opts.inspectPort || (location.port ? +location.port + 1 : 5181)
  const esc = s => String(s).replace(/[&<>"]/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' })[c])
  panel.box.innerHTML = `<div class="h">brain-inspect <span class="k">${u == null ? '—' : (u / 1048576).toFixed(1) + ' MB'} · GC ${gcs}</span></div>` +
    (leaks.length ? leaks.map(l => `<div class="l"><b>${l.code}</b> ${esc(l.msg)}</div>`).join('') : '<div class="ok">No leaks detected.</div>') +
    `<a href="${location.protocol}//${location.hostname}:${port}/" target="_blank" rel="noopener">Open full debugger →</a>`
}

/* FORMAT v2 (every record in `d` has `k`; records with `id` are updated by patches with the same id):
 heap  {at, used, src:'uasm'|'memory'|'node'|'none'}
 ev    {at, e:'mount'|'dispose'|'hmr'|'hydrate'|'module', h?, id?, url?, ms?, b?, dh}
 inst  {id, h, name, f, parent, island:null|'hydrated'}   gone {id}   islands {inert:{hash: n}} (every 5 s)
 ev.e also 'paint' (mount → first frame, ms)
 comp  {h, name, f, inst, sig, drv, fx, ev, nodes, est, runs, ms, inp}  (every 1 s; inp = worst interaction ms or null)
 jank  {at, ms, block, scripts:[{src, fn, inv, ms, comp:{h,name}|null}]}  (Long Animation Frame ≥50 ms)
 inp   {at, type, ms, delay, proc, comp:{id,h,name}|null}               (Event Timing, interactions ≥16 ms)
 state {id, sigs:[{n, v, w, c}], drv:[v], props}                        (only if changed)
 graph {h, nodes:[{k:'sig'|'drv'|'fx', name}], edges:[[from, to]]}     (every 5 s)
 upd   {at, sig, w, fx, nodes, ms}                                       (signal written → effects → nodes touched)
 handler {at, id, h, name, type, ms}
 net   {id, kind:'fetch'|'xhr', m, url, own, line, foreign, at, st, ms, b}
 store {area:'cookie'|'local'|'session'|'idb'|'cache', key, op:'set'|'get'|'remove'|'open', v?, flags?, exp?:'session'|'max-age=N'|<expires>, own, line, foreign, at}
 idb   {db, store, count, sample}          cachestore {name, count}
 conn  {id, kind:'ws'|'es'|'bc'|'worker', url, own, line, foreign, st:'opening'|'open'|'error'|'closed', in, out, bin, bout}
 timer {id, kind:'timeout'|'interval'|'raf', ms, own, line, foreign, cleanup:true|false|null, st:'on'|'off'}
 leak  {code:'B0601'|'B0602', h, name, file, line?, msg, fix, fx:[…], at}
 chan  {kind:'ctx'|'store'|'bus'|'api', name, role:'provide'|'use'|'read'|'write'|'send'|'on'|'expose', id, h, comp}  (once per component edge; id null = module scope)
 err   {fp, code, msg, where:'fx'|'drv'|'on:<ev>'|'render'|'B0902'|'onerror'|'promise'|'rpc', comp:{id,h,name,file,chain}, line, stack, crumbs, n, at}
 own = {id, h, name, o:'fx'|'setup'|'on:<event>'} | null · line = "url:line" of the first app frame · foreign = no app frame */
