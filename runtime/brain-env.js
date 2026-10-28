// BRAINLLM lifecycle and environment helpers (INTERNALS §41, #169). Each one cleans up with the component that called
// it (the owner's effects are disposed on unmount), and none of them runs timers, observers or listeners while the
// server renders (server.mjs SSR or the build's prerender: globalThis.__brainSSR / __brainRoute are set), so a render
// never keeps Node alive. Values meant for markup (`media`, `visible`, `debounce`) are read-only nodes read with `.v`.
import { _rt, sig, fx, untrack } from './brain.js'

const G = globalThis, I = _rt()
const onServer = () => !!(G.__brainSSR || G.__brainRoute)
const ro = n => ({ get v() { return n.v } })
const ms = t => {
  if (typeof t === 'number') return t
  const m = /^(\d+(?:\.\d+)?)(ms|s|m|h)$/.exec(String(t).trim())
  if (!m) throw Error(`brain-env: "${t}" is not a duration (300ms, 30s, 5m, 1h)`)
  return m[1] * { ms: 1, s: 1e3, m: 6e4, h: 36e5 }[m[2]]
}
// runs `f` when the component unmounts (an fx with no reads: its cleanup is the owner's disposal)
const onDispose = f => fx(() => f)

// mounted(f): after the component's nodes are in the document. A returned function runs on unmount.
export function mounted(f) {
  if (onServer()) return
  let alive = true, c
  queueMicrotask(() => alive && (c = untrack(f)))
  onDispose(() => ((alive = false), typeof c === 'function' && c()))
}

// watch(() => src, (v, old) => …): runs the callback when the source changes, not on the first read; the callback is
// not tracked. A returned function is its cleanup (before the next run and on unmount).
export function watch(src, cb) {
  let first = true, old
  fx(() => {
    const v = src()
    if (first) return void ((first = false), (old = v))
    const o = old
    old = v
    return untrack(() => cb(v, o))
  })
}

// tick(): resolves once pending updates have reached the DOM
export const tick = () => new Promise(r => setTimeout(r, 0))

// every('30s', f): an interval, paused while the tab is hidden
export function every(t, f) {
  if (onServer()) return
  const n = ms(t), d = G.document
  let id = 0
  const sync = () => (clearInterval(id), (id = d && d.hidden ? 0 : setInterval(() => untrack(f), n)))
  sync()
  if (d && d.addEventListener) d.addEventListener('visibilitychange', sync)
  onDispose(() => (clearInterval(id), d && d.removeEventListener && d.removeEventListener('visibilitychange', sync)))
}

// media('(max-width: 600px)') → .v true/false, following the viewport
export function media(q) {
  const mq = !onServer() && G.matchMedia ? G.matchMedia(q) : null
  const n = sig(!!(mq && mq.matches))
  if (mq) {
    const on = e => (n.v = e.matches)
    mq.addEventListener('change', on)
    onDispose(() => mq.removeEventListener('change', on))
  }
  return ro(n)
}

// visible(ref) → .v true while the element is in the viewport (IntersectionObserver); a ref may still be empty
export function visible(r, o) {
  const n = sig(false)
  if (onServer() || !G.IntersectionObserver) return ro(n)
  fx(() => {
    const el = r && r.nodeType ? r : r && r.v
    if (!el) return
    const io = new IntersectionObserver(es => (n.v = es[es.length - 1].isIntersecting), o)
    io.observe(el)
    return () => io.disconnect()
  })
  return ro(n)
}

// debounce(() => value, '300ms') → .v follows the value once it has been still for that long
export function debounce(f, t) {
  const n = sig(untrack(f)), wait = ms(t)
  let first = true
  fx(() => {
    const v = f()
    if (first) return void (first = false)
    if (onServer()) return void (n.v = v)
    const id = setTimeout(() => (n.v = v), wait)
    return () => clearTimeout(id)
  })
  return ro(n)
}

// <component is={C} …props> → dyn(parent, () => C, $p, anchor): mounts C before the anchor and remounts only when the
// component itself changes (a null C renders nothing). The mounted component belongs to the caller's owner.
export function dyn(p, c, $p, end) {
  const own = I.owner()
  let cur, nodes = [], d = null
  const clear = () => (d && d(), (d = null), I.rm(nodes), (nodes = []))
  fx(() => {
    const C = c()
    if (C === cur) return
    cur = C
    untrack(() => {
      clear()
      if (C) I.root(x => ((d = x), (nodes = I.nodes(C($p || {})))), own)
      const par = (end && end.parentNode) || p
      for (const n of nodes) par.insertBefore(n, end || null)
    })
  })
  onDispose(clear)
}

// <Portal to="#layer"> → portal(target, () => children): renders the children into another element (a selector or an
// element; document.body by default) and removes them on unmount. Returns what to insert in place: nothing in the
// browser; while the server renders, the children themselves (they stay where they are written).
export function portal(target, f) {
  if (onServer() || !G.document) return f()
  const t = (typeof target === 'string' ? document.querySelector(target) : target) || document.body
  let nodes = [], d = null
  I.root(x => ((d = x), (nodes = I.nodes(f()))), I.owner())
  for (const n of nodes) t.appendChild(n)
  onDispose(() => (d && d(), I.rm(nodes)))
  return []
}

// route() → { path, params, query }, read-only and reactive (each a signal read): path without the base path
// (<meta name="brain-base">), query from the URL, params from the page's <meta name="brain-params" content='{"id":"7"}'>
// (the server knows the route's pattern; the client does not). brain-router calls globalThis.__brainRouted after every
// client navigation. While the server renders, the values come from globalThis.__brainRoute (server.mjs, prerender).
let cur = null
const paramsOf = () => {
  try {
    const m = G.document && document.querySelector('meta[name=brain-params]')
    return (m && JSON.parse(m.getAttribute('content'))) || {}
  } catch {
    return {}
  }
}
const strip = p => {
  const m = G.document && document.querySelector('meta[name=brain-base]'), b = (m && m.getAttribute('content')) || '/'
  return b !== '/' && (p + '/').startsWith(b) ? '/' + p.slice(b.length) : p
}
const read = () => {
  const u = new URL(location.href)
  return [strip(u.pathname), paramsOf(), Object.fromEntries(u.searchParams)]
}
export function route() {
  const s = G.__brainRoute
  if (s) return { path: s.path, params: s.params || {}, query: s.query || {} }
  if (!cur) {
    const S = read().map(v => sig(v))
    cur = { get path() { return S[0].v }, get params() { return S[1].v }, get query() { return S[2].v } }
    G.__brainRouted = () => {
      const v = read()
      S.forEach((s, i) => JSON.stringify(untrack(() => s.v)) !== JSON.stringify(v[i]) && (s.v = v[i])) // only what changed
    }
  }
  return cur
}
