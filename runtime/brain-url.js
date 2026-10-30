// Shareable public navigation state. Credentials, form contents and transient busy/error state do not belong in URLs.
import { sig, fx, _rt, untrack } from './brain.js'
const G = globalThis, I = _rt(), bindings = new Map(), known = new Set()
const privateKey = /password|secret|token|authorization|session|csrf|jwt|email|credit/i
let listening = false, reports = 0
const report = fix => { if (reports++ < 32) { const e = Object.assign(new Error(fix), { code: 'B0915', fix }); G.reportError ? G.reportError(e) : console.warn(e.message) } }
const scalar = v => typeof v === 'string' || typeof v === 'boolean' || (typeof v === 'number' && Number.isFinite(v))
const safe = v => scalar(v) && String(v).length <= 512 && !/[\u0000-\u001f\u007f]/.test(String(v)) && !/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(String(v))
const encode = v => Array.isArray(v) ? (v.length <= 32 && v.every(safe) ? JSON.stringify(v) : null) : safe(v) ? String(v) : null
function decode(raw, base) {
  if (raw == null) return base
  let v = raw
  if (typeof base === 'boolean') { if (!/^(true|false)$/.test(raw)) return base; v = raw === 'true' }
  else if (typeof base === 'number') { if (!/^-?(?:\d+\.?\d*|\.\d+)(?:e[+-]?\d+)?$/i.test(raw)) return base; v = Number(raw) }
  else if (Array.isArray(base)) { try { v = JSON.parse(raw) } catch { return base }; if (!Array.isArray(v) || (base.length && v.some(x => typeof x !== typeof base[0]))) return base }
  return encode(v) == null ? base : v
}
function restore() {
  if (!G.location) return
  const url = new URL(G.location.href)
  for (const [key, b] of bindings) if (b.path === url.pathname) {
    const value = decode(url.searchParams.get(key), b.base)
    b.last = encode(value)
    b.cell.v = value
  }
  G.__brainRouted?.()
}
G.__brainURLOnly = (from, to) => {
  try {
    const a = new URL(from, G.location.href), b = new URL(to, G.location.href)
    if (a.origin !== b.origin || a.pathname !== b.pathname || a.hash !== b.hash) return false
    if (a.href === b.href) return true
    for (const key of known) { a.searchParams.delete(key); b.searchParams.delete(key) }
    return a.href === b.href
  } catch { return false }
}
G.__brainURLRestore = restore

export function urlState(initial, options) {
  const cell = sig(initial)
  if (!options?.disabled) urlBind(cell, options?.key, options)
  return cell
}

// A native UI primitive stays stateless in author source; its DOM adapter owns navigation state and cleanup.
export function urlDialog(element, key) {
  if (!element || !key || G.__brainSSR || G.__brainRoute || !G.location) return
  return I.root(dispose => {
    let alive = true
    const opened = urlState(!!element.open, {key})
    const read = () => { if (alive) opened.v = !!element.open }
    const apply = value => {
      if (!alive || !element.isConnected) return
      if (value && !element.open) element.showModal()
      else if (!value && element.open) element.close()
    }
    fx(() => {
      const value = opened.v
      if (element.isConnected) apply(value)
      else queueMicrotask(() => apply(value))
    })
    const observer = G.MutationObserver ? new G.MutationObserver(read) : null
    observer?.observe(element, {attributes:true,attributeFilter:['open']})
    element.addEventListener('close',read)
    element.addEventListener('toggle',read)
    fx(() => () => {
      alive = false
      observer?.disconnect()
      element.removeEventListener('close',read)
      element.removeEventListener('toggle',read)
    })
    return dispose
  }, I.owner())
}

// Internal compiler binding for public select/tab/modal signals, once during component setup.
export function urlBind(cell, key, options = {}) {
  if (G.__brainRoute || G.__brainSSR) {
    if (typeof key === 'string' && key.length <= 200 && !privateKey.test(key) && encode(untrack(() => cell.v)) != null) {
      const raw = G.__brainRoute?.query?.[key]
      cell.v = decode(typeof raw === 'string' ? raw : null, untrack(() => cell.v))
    }
    return cell
  }
  if (!G.location || !G.history) return cell
  if (typeof key !== 'string' || !key.length || key.length > 200 || privateKey.test(key) || encode(untrack(() => cell.v)) == null) {
    report('URL state needs a stable public key and bounded scalar value; keep credentials and form contents in private state')
    return cell
  }
  const owner = I.owner()
  if (!owner) { report('URL state must belong to a component so its listeners can be disposed'); return cell }
  const previous = bindings.get(key)
  if (previous) {
    if (previous.cell !== cell) report('Repeated URL state key; give repeated components distinct stable urlState keys')
    return cell
  }
  if (bindings.size >= 128 || known.size >= 512) { report('URL state key capacity exceeded; reduce public state or reuse stable keys'); return cell }
  const base = untrack(() => cell.v), url = new URL(G.location.href)
  if (url.search.length > 8192) { report('URL state exceeds 8 KiB; shorten public selections'); return cell }
  cell.v = decode(url.searchParams.get(key), base)
  const b = { cell, base, path: url.pathname, last: encode(cell.v) }
  bindings.set(key, b); known.add(key)
  if (!listening) { G.addEventListener('popstate', restore); listening = true }
  fx(() => {
    const value = encode(cell.v)
    if (value === b.last) return
    b.last = value
    if (value == null) { report('URL state value is private, unsupported or too large; keep it out of navigation state'); return }
    const next = new URL(G.location.href)
    if (next.pathname !== b.path) return
    if (value === encode(base)) next.searchParams.delete(key)
    else next.searchParams.set(key, value)
    if (next.search.length > 8192) { report('URL state exceeds 8 KiB; shorten public selections'); return }
    if (next.href === G.location.href) return
    try {
      G.history[options.history === 'replace' ? 'replaceState' : 'pushState'](G.history.state, '', next.href)
      G.__brainRouted?.()
    } catch { report('URL state history write failed; reduce the frequency of changes or use history: replace') }
  })
  fx(() => () => {
    if (bindings.get(key) === b) bindings.delete(key)
    if (!bindings.size && listening) { G.removeEventListener('popstate', restore); listening = false }
  })
  return cell
}
