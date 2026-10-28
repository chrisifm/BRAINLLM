// BRAINLLM channels (INTERNALS §12): context (provide/use/key), store, bus and exposed API. Props, the `~` model,
// events (`emit` → `$p["on:x"]`) and slots are resolved by the codegen with no extra runtime.
// Every channel use notifies `seen` (fn(kind, name, role, owner)): the inspector builds the Graph view without this
// module retaining owners.
import { _rt, sig, fx, batch, untrack } from './brain.js'

const G = globalThis
const I = _rt((pairs, f) => {
  for (const [k, v] of pairs) provide(k, v)
  return f()
})
export const seen = new Set()
const note = (k, n, role) => {
  if (seen.size) for (const f of seen) f(k, n, role, I.owner())
}

// ---------- context ----------
// The key is an object exported from channels/*.jsbrain (`export const Theme = key('theme')`): no collisions.
export const key = (name, d) => ({ name, d })
const CTX = new WeakMap() // owner N → Map(key → value)
// Components built afterwards under the same owner see it. To isolate siblings, the codegen wraps the component that
// calls provide with `scope(C)`: that way the value stays only in its subtree.
export function provide(k, v) {
  const o = I.owner()
  if (!o) throw Error('provide outside a component')
  const m = CTX.get(o) || (CTX.set(o, new Map()), CTX.get(o))
  m.set(k, v)
  note('ctx', k.name, 'provide')
  return v
}
export function use(k) {
  note('ctx', k.name, 'use')
  for (let o = I.owner(); o; o = o.p) {
    const m = CTX.get(o)
    if (m && m.has(k)) return m.get(k)
  }
  if (k.d !== undefined) return k.d
  throw Object.assign(Error(`B0807: use(${k.name}) without provide(${k.name}) in the ancestors`), { code: 'B0807' })
}
// Own owner for a component (the codegen emits it if the component calls provide).
export const scope = C => Object.assign($p => I.root(() => C($p), I.owner()), { C, h: C.h, f: C.f, p: C.p, form: C.form })
// Tests and app.brain: mount with provided values (`withProvide([[Api, fakeApi]], () => Comp(props))`).
export const withProvide = (pairs, f) =>
  I.root(() => {
    for (const [k, v] of pairs) provide(k, v)
    return f()
  }, I.owner())

// Reactive context read (codegen): `let theme = use(Theme)` → `drv(((u) => () => val(u))(use(Theme)))`. A provided
// sig/drv is read (tracked); any other value, including objects with a `v` field, is returned as is.
const N = sig(0).constructor
export const val = x => (x instanceof N ? x.v : x)

// state from another tab, the server or storage is untrusted JSON: prototype keys are dropped (§31)
const safe = (k, v) => (k === '__proto__' || k === 'constructor' || k === 'prototype' ? undefined : v)

// ---------- store ----------
// `store({ items: [], total: 0 })` → object with a reactive getter/setter per field (`cart.items = [...]`).
// `$set(patch)` writes several in one batch; `$snap()` flat copy; `$reset()` restores init. Valid in channels/ (one
// global instance) and inside `provide(K, store(…))` (one instance per subtree, §41.2). sync: 'tab' (BroadcastChannel) | 'storage'
// (localStorage + storage event) | 'ws' (WebSocket at `url`, see ws() below). `id` is set by the codegen (channel
// path); if missing, it is derived from init.
export function store(init, o = {}, id) {
  const name = id || o.name || 'store:' + Object.keys(init).join(',')
  const S = {}, s = {}
  let last = '' // JSON of the last state sent or received: cuts the echo between tabs
  for (const k of Object.keys(init)) {
    S[k] = sig(init[k], name + '.' + k) // named: dev tools (brain_why, HMR) see `cart.total`
    Object.defineProperty(s, k, {
      enumerable: true,
      get: () => (note('store', name, 'read'), S[k].v),
      set: v => (note('store', name, 'write'), (S[k].v = v)),
    })
  }
  const snap = () => {
    const r = {}
    for (const k in S) r[k] = untrack(() => S[k].v)
    return r
  }
  const apply = d => {
    const n = snap()
    for (const k in d) if (k in S) n[k] = d[k]
    last = JSON.stringify(n) // before the batch: the sync fx runs when it closes and must see the echo
    batch(() => {
      for (const k in n) S[k].v = n[k]
    })
  }
  Object.defineProperties(s, {
    $set: { value: d => batch(() => { for (const k in d) if (k in S) s[k] = d[k] }) },
    $snap: { value: snap },
    $reset: { value: () => apply({ ...init }) }, // back to the initial state (persisted and synced like any write)
  })
  const K = 'brain:' + name
  if (o.persist) {
    // Persistence (INTERNALS §35.4), versioned by the shape of init. Small states live in localStorage and are restored
    // here, synchronously, before the first render; brain-persist.js (loaded on demand) handles IndexedDB for large ones
    // and the writes. `ready()` resolves once every persisted store is restored.
    const P = 'brain:p:' + name, v = shape(init)
    let big = false
    try {
      const r = JSON.parse(localStorage.getItem(P), safe)
      if (r && r.v === v) r.idb ? (big = true) : apply(r.d)
      else if (r) localStorage.removeItem(P) // another shape: the old data is discarded, never applied
    } catch {}
    const base = JSON.stringify(snap()) // the state as created: changes made before the module loads are still saved
    READY.push(import('./brain-persist.js').then(m => m.persist({ key: P, v, big, apply, snap, S, base })))
  }
  if (o.sync === 'storage' && G.localStorage) {
    try {
      const v = localStorage.getItem(K)
      if (v) apply(JSON.parse(v, safe))
    } catch {}
    G.addEventListener && G.addEventListener('storage', e => e.key === K && e.newValue && apply(JSON.parse(e.newValue, safe)))
  }
  const bc = o.sync === 'tab' && G.BroadcastChannel ? new BroadcastChannel(K) : null
  const w = o.sync === 'ws' && G.WebSocket ? ws(o.url || '/_ws', K, apply) : null
  if (w) Object.defineProperty(s, '$close', { value: w.close })
  if (bc) {
    bc.onmessage = e => (e.data === '?' ? bc.postMessage(snap()) : apply(e.data))
    bc.postMessage('?') // a new tab asks the open ones for the state
  }
  last = JSON.stringify(snap()) // the initial state is not broadcast: a new tab asks for it with '?'
  // a store created inside provide() lives in that subtree: its sync effect is disposed with it
  if (o.sync)
    I.root(() =>
      fx(() => {
        const d = {}
        for (const k in S) d[k] = S[k].v // reads everything: re-runs on any change
        const j = JSON.stringify(d)
        if (j === last) return
        last = j
        if (bc) bc.postMessage(d)
        else if (w) w.send(d)
        else
          try {
            if (j.length > 2e6) throw Error('over 2 MB') // localStorage is ~5 MB per origin for the whole app
            localStorage.setItem(K, j)
          } catch (e) {
            // full or too big: the state stays in memory, the page keeps working, and the author is told why (§31)
            console.warn(`store ${name}: not saved to localStorage (${e && e.message}); it is kept in memory only`)
          }
      }),
    I.owner())
  return s
}

// Real-time sync (INTERNALS §22). One socket per url, shared by its stores. Protocol (JSON text frames):
//   client → server  {s, d}  local change (full state) · {s, q:1}  asks for the current state (after each connect)
//   server → client  {s, d}  state of store s (the server keeps the last one per s and relays it to the others)
// While offline, the latest local state waits and is sent on reconnect BEFORE asking (the server's reply then matches it).
// Reconnection: exponential backoff 0.5 s → 30 s with ±50 % jitter, reset on open; `online` and returning to a
// visible tab retry at once. `$close()` stops it.
const WS = new Map()
function ws(url, s, apply) {
  let c = WS.get(url)
  if (!c) {
    const subs = new Map(), wait = new Map()
    let sock, n = 0, t, off = false
    const u = new URL(url, G.location && location.href).href.replace(/^http/, 'ws')
    const put = (k, d) => sock && sock.readyState === 1 && (sock.send(JSON.stringify(d ? { s: k, d } : { s: k, q: 1 })), true)
    const open = () => {
      clearTimeout(t)
      if (off || (sock && sock.readyState < 2)) return
      sock = new WebSocket(u)
      sock.onopen = () => {
        n = 0
        for (const [k, d] of wait) put(k, d)
        wait.clear()
        for (const k of subs.keys()) put(k)
      }
      sock.onmessage = e => {
        let m
        try {
          m = JSON.parse(e.data, safe)
        } catch {
          return
        }
        const f = m && subs.get(m.s)
        if (f && m.d && typeof m.d === 'object') f(m.d)
      }
      sock.onclose = () => {
        if (off) return
        const ms = Math.min(30000, 500 * 2 ** n++)
        t = setTimeout(open, ms * (0.5 + Math.random()))
      }
    }
    const now = () => ((n = 0), open())
    G.addEventListener && G.addEventListener('online', now)
    G.document && document.addEventListener && document.addEventListener('visibilitychange', () => document.visibilityState === 'visible' && open())
    c = {
      subs,
      sub(k, f) {
        subs.set(k, f)
        if (!put(k)) open()
      },
      send: (k, d) => put(k, d) || (wait.set(k, d), open()),
      close() {
        off = true
        clearTimeout(t)
        sock && sock.close()
        WS.delete(url)
        G.removeEventListener && G.removeEventListener('online', now)
      },
    }
    WS.set(url, c)
  }
  c.sub(s, apply)
  return { send: d => c.send(s, d), close: () => (c.subs.delete(s), c.subs.size || c.close()) }
}

// Shape of a store: field names + kind of each initial value. A persisted state with another shape is discarded.
const kind = x => (Array.isArray(x) ? 'list' : x === null ? 'null' : typeof x)
const shape = init => Object.keys(init).sort().map(k => k + ':' + kind(init[k])).join(',')
const READY = []
export const ready = () => Promise.all(READY)

// ---------- bus ----------
// Stateless notification: `toast.send(msg)` · `toast.on(fn)` → off. Inside a component it unsubscribes itself on
// unmount. A failing handler does not stop the others (reportError → onerror → brain-err).
export function bus(name) {
  const subs = new Set()
  return {
    name,
    send(m) {
      note('bus', name, 'send')
      for (const f of [...subs])
        try {
          f(m)
        } catch (e) {
          G.reportError ? reportError(e) : queueMicrotask(() => { throw e })
        }
    },
    on(f) {
      note('bus', name, 'on')
      subs.add(f)
      const off = () => subs.delete(f)
      if (I.owner()) fx(() => off)
      return off
    },
  }
}

// ---------- exposed API ----------
// Child with `<meta expose="open close">`: the codegen emits `expose($p, { open, close })` at the end of setup; the parent
// `<Modal bind:api={m} />` passes `"bind:api": a => (m = a)`. When the child unmounts, the parent receives null.
export function expose($p, api) {
  const f = $p['bind:api']
  note('api', Object.keys(api).join(' '), 'expose')
  if (f) {
    f(Object.freeze(api))
    if (I.owner()) fx(() => () => f(null))
  }
  return api
}
