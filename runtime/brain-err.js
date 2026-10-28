// BRAINLLM native error capture (INTERNALS §18), no third parties. The core already captures fx, drv, handlers and render
// (and cuts B0902 loops); this module gives each error a fingerprint, component and parent chain, stack, breadcrumbs
// (last 30 actions), build and browser; it dedupes and sends via sendBeacon with a cap.
// Usage: prod `capture({ report: '/path', build })` (brain build --report) · in dev brain-inspect installs it with `who`.
// `fallback(C, fb)` (codegen of `<meta fallback="…">`: `dev(fallback(C, fb))`) lives here: importing the module
// installs the core hook, so an app without fallbacks or --report does not pay for this code.
// Never includes input values, cookies, query strings or signal values.
import { _err, sig } from './brain.js'

const G = globalThis
const RT = new URL('.', import.meta.url).href
// Core hook: records (capture active) or console.error, and falls back to the nearest boundary's fallback (not for
// event handlers).
const I = _err((e, x, w) => {
  on ? record(e, x, w) : G.console && console.error(e)
  // §42: a failing event handler never destroys its component (it is reported; brain-guard shows the toast)
  if (String(w).startsWith('on:')) return
  for (; x; x = x.p || x.parentNode) {
    const b = BW.get(x)
    if (b) return fall(b)
  }
})
// Boundaries: owner N and its root elements → { fb, n: nodes, r: owner, f: already fell }.
const BW = new WeakMap()
const fbn = fb => {
  if (typeof fb === 'function') return I.nodes(fb())
  const p = document.createElement('p')
  p.setAttribute('role', 'alert')
  p.textContent = fb
  return [p]
}
function fall(b) {
  if (b.f) return
  b.f = 1
  const a = fbn(b.fb), f = b.n[0], p = f && f.parentNode
  if (p) for (const x of a) p.insertBefore(x, f)
  b.r.q() // stops fx and handlers of the fallen component
  I.rm(b.n)
  b.n = a
}
// An error in the component's render, fx or handler swaps its nodes for the fallback (text → <p role=alert>,
// function → nodes); the rest of the app keeps running.
export const fallback = (C, fb) =>
  Object.assign(
    $p =>
      I.root(() => {
        const r = I.owner(), b = { fb, n: [], r, f: 0 }
        BW.set(r, b)
        let a
        try {
          a = I.nodes(C($p))
        } catch (e) {
          on ? record(e, r, 'render') : G.console && console.error(e)
          r.q()
          ;(b.f = 1), (a = fbn(fb))
        }
        for (const x of (b.n = a)) BW.set(x, b)
        return a
      }, I.owner()),
    { C, h: C.h, f: C.f, p: C.p, form: C.form }
  )

const MSG = { B0902: 'fx re-ran >100 times in one flush: sig→fx→sig cycle; cut off so the tab does not hang' }
export const errors = [] // deduplicated records, most recent last (Errors view, MCP brain_errors)
export const sub = new Set() // fn(record) on each new or repeated error
const crumbs = []
let opts = {}, on = false, sent = 0, F0 = null

export function crumb(k, d) {
  const l = crumbs[crumbs.length - 1]
  if (l && l.k === k && l.d === d) return void l.n++ // the same repeated action is counted, not stacked
  crumbs.push({ k, d, n: 1, at: Date.now() })
  if (crumbs.length > 30) crumbs.shift()
}

const hash = s => {
  let h = 5381
  for (let i = 0; i < s.length; i++) h = ((h * 33) ^ s.charCodeAt(i)) >>> 0
  return h.toString(36)
}
const path = u => {
  try {
    return new URL(u, G.location && G.location.href).pathname // no query: it may carry sensitive data
  } catch {
    return String(u).split('?')[0]
  }
}
// "url:line:col" of each frame; the first one outside the runtime is the app line.
function frames(e) {
  const out = []
  for (const l of String((e && e.stack) || '').split('\n')) {
    const m = l.match(/((?:https?|file):\/\/[^\s)]+):(\d+):(\d+)/)
    if (m) out.push({ url: m[1], line: +m[2], col: +m[3], rt: m[1].startsWith(RT) && !m[1].slice(RT.length).includes('/') })
  }
  return out
}
// Component via the DOM: roots with data-b="<hash>" (prod) → parent chain via ancestors with data-b.
function byDom(el) {
  const chain = []
  for (; el; el = el.parentNode) if (el.getAttribute && el.getAttribute('data-b')) chain.push({ h: el.getAttribute('data-b') })
  return chain.length ? { ...chain[0], chain: chain.map(c => c.h) } : null
}
// Prod chunk /_b/<hash>.brain → component hash.
const byStack = f => {
  const m = f && f.url.match(/\/_b\/([a-z0-9]+)/)
  return m ? { h: m[1], chain: [m[1]] } : null
}

// .brain lines (§18, dev): each module compiled in dev ends with an inline Source Map v3 that maps every generated
// line to its .brain line. With `maps`, a recorded error is enriched asynchronously: `src` ('views/home.brain:12', the
// first app frame) and `brainStack`; subscribers (brain_errors, overlay, doctor) are notified again. The file path
// comes from the module URL (two `index.brain` in different folders never mix); prod chunks carry no map: no-op.
const MAPS = new Map()
const B64 = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/'
export function decode(m) {
  const lines = []
  let src = 0
  for (const seg of m.split(';')) {
    let line = null
    for (const part of seg.split(',')) {
      if (!part) continue
      const v = []
      for (let i = 0, n = 0, sh = 0; i < part.length; i++) {
        const d = B64.indexOf(part[i])
        n += (d & 31) << sh
        if (d & 32) sh += 5
        else (v.push(n & 1 ? -(n >> 1) : n >> 1), (n = 0), (sh = 0))
      }
      if (v.length >= 3) (src += v[2]), line === null && (line = src + 1)
    }
    lines.push(line)
  }
  return lines
}
const mapOf = url => {
  if (!MAPS.has(url))
    MAPS.set(
      url,
      Promise.resolve(opts.load ? opts.load(url) : F0(url).then(r => r.text()))
        .then(t => {
          const m = /sourceMappingURL=data:application\/json;base64,([A-Za-z0-9+/=]+)/.exec(t)
          return m ? decode(JSON.parse(atob(m[1])).mappings) : null
        })
        .catch(() => null)
    )
  return MAPS.get(url)
}
async function toBrain(r, f) {
  const out = []
  for (const t of f.slice(0, 12)) {
    const lines = t.rt ? null : await mapOf(t.url)
    const l = lines && lines[t.line - 1]
    if (l) out.push(`${path(t.url).replace(/^\//, '')}:${l}`)
  }
  if (!out.length) return
  r.src = out[0]
  r.brainStack = out
  for (const s of sub) s(r)
}

function record(e, x, w) {
  if (e && e.kind === 'aborted') return // a cancelled request (brain-data ApiError): not an error
  const f = frames(e), top = f.find(t => !t.rt) || null
  const c = (opts.who && opts.who(x)) || (x && x.nodeType ? byDom(x) : null) || byStack(top)
  const type = (e && e.code) || (e && e.name) || typeof e
  const line = top ? `${path(top.url)}:${top.line}` : null
  const fp = hash([type, c && c.h, line, w].join('|'))
  let r = errors.find(r => r.fp === fp)
  if (r) (r.n++, (r.last = Date.now()))
  else {
    r = {
      fp, code: (e && e.code) || null, type, msg: MSG[e && e.code] || String((e && e.message) || e).slice(0, 500), where: w || null, comp: c, line,
      stack: f.slice(0, 12).map(t => `${path(t.url)}:${t.line}:${t.col}`), crumbs: crumbs.map(c => ({ ...c })),
      build: opts.build || null, ua: G.navigator ? navigator.userAgent : null, at: Date.now(), last: Date.now(), n: 1,
    }
    errors.push(r)
    if (errors.length > 100) errors.shift()
    if (opts.maps) toBrain(r, f)
    if (opts.report && G.navigator && navigator.sendBeacon && sent < (opts.max || 20)) sent++, navigator.sendBeacon(opts.report, JSON.stringify(r))
  }
  for (const s of sub) s(r)
  if (opts.log !== false && G.console) console.error(e)
  return r
}

// Readable element label without values: aria-label, name or button/link text (40 chars).
const label = el => {
  if (!el || !el.tagName) return '?'
  const t = el.tagName.toLowerCase(), a = el.getAttribute('aria-label') || el.getAttribute('name')
  const txt = /^(input|textarea|select)$/.test(t) ? '' : String(el.textContent || '').trim().slice(0, 40)
  return `${t}${a ? `[${a}]` : ''}${txt ? ` "${txt}"` : ''}`
}

export function capture(o = {}) {
  if (on) return (Object.assign(opts, o), errors)
  on = true
  opts = o
  F0 = G.fetch && G.fetch.bind(G) // the unwrapped fetch: reading source maps leaves no breadcrumbs
  if (G.addEventListener) {
    G.addEventListener('error', ev => record(ev.error || ev.message, null, 'onerror'))
    G.addEventListener('unhandledrejection', ev => record(ev.reason, null, 'promise'))
    G.addEventListener('popstate', () => crumb('nav', path(location.href)))
    G.addEventListener('hashchange', () => crumb('nav', path(location.href) + location.hash))
  }
  const d = G.document
  if (d && d.addEventListener) {
    for (const t of ['click', 'submit', 'change']) d.addEventListener(t, e => crumb(t, label(e.target)), true)
    d.addEventListener('input', e => crumb('input', label(e.target)), true) // only which field, never the value
  }
  if (G.history)
    for (const k of ['pushState', 'replaceState']) {
      const f = history[k]
      history[k] = function (s, t, u) {
        const r = f.apply(this, arguments)
        crumb('nav', path(u == null ? location.href : u))
        return r
      }
    }
  if (G.fetch) {
    const f = G.fetch
    G.fetch = function (i, init) {
      const m = (init && init.method) || (i && i.method) || 'GET', u = path((i && i.url) || i)
      crumb('fetch', `${m} ${u}`)
      return f.apply(this, arguments).then(
        r => (r.status >= 400 && crumb('fetch', `${m} ${u} → ${r.status}`), r),
        e => (crumb('fetch', `${m} ${u} → failed`), Promise.reject(e))
      )
    }
  }
  // Signal writes: name (dev, opts.sigName) or 'sig'; never the value.
  const P = Object.getPrototypeOf(sig(0)), v = Object.getOwnPropertyDescriptor(P, 'v')
  Object.defineProperty(P, 'v', {
    configurable: true,
    get: v.get,
    set(x) {
      const b = this._
      v.set.call(this, x)
      if (this._ !== b) crumb('sig', (opts.sigName && opts.sigName(this)) || 'sig')
    },
  })
  return errors
}
