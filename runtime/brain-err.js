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
  on ? record(e, x, w) : print(e)
  // §42: a failing event handler never destroys its component (it is reported; brain-guard shows the toast)
  if (String(w).startsWith('on:')) return
  for (; x; x = x.p || x.parentNode) {
    const b = BW.get(x)
    if (b) return fall(b)
  }
})
const print = (e, ref) => G.console && console.error(I.development?.() ? e : `BRAIN_ERROR: Operation failed. Reference: ${ref || 'unavailable'}`)
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
          on ? record(e, r, 'render') : print(e)
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
  k = clean(k, 40); d = clean(d, 200)
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
    return String(u).split(/[?#]/)[0]
  }
}
const LIMIT = { frames: 24, causes: 6, stackBytes: 16384, stackLines: 256, maps: 64, sourceBytes: 1048576, mapLines: 65536, loads: 2, pending: 64, deadline: 2000 }
const get = (o, k) => { try { return o && o[k] } catch { return null } }
const clean = (s, max = 500) => String(s == null ? '' : s).slice(0, max).replace(/(?:https?|file):\/\/[^\s)]+/g, u => u.split(/[?#]/)[0]).replace(/([/\w.:%-]+)[?#][^\s)'"]*/g, '$1')
// Only structural stack locations are retained, never argument lists or the raw stack header.
function frames(e) {
  const raw = String(get(e, 'stack') || ''), out = []
  let omitted = raw.length > LIMIT.stackBytes
  const lines = raw.slice(0, LIMIT.stackBytes).split('\n')
  if (lines.length > LIMIT.stackLines) omitted = true
  for (const l of lines.slice(0, LIMIT.stackLines)) {
    const m = l.match(/((?:https?|file):\/\/[^\s)]+):(\d+):(\d+)/)
    if (!m) continue
    if (![+m[2], +m[3]].every(n => Number.isSafeInteger(n) && n > 0)) { omitted = true; continue }
    if (out.length >= LIMIT.frames) { omitted = true; break }
    out.push({ url: m[1], line: +m[2], col: +m[3], rt: m[1].startsWith(RT) && !m[1].slice(RT.length).includes('/') })
  }
  return { frames: out, truncated: omitted }
}
const generated = t => t ? { file: path(t.url).slice(-1024), line: t.line, column: t.col } : null
const location = (t, c) => ({ component: c ? clean(c.name || c.h || '', 120) || null : null, file: t ? path(t.url).slice(-1024) : c && c.file ? c.file : null, line: t ? t.line : null, column: t ? t.col : null, precision: t ? 'generated' : 'component-only' })
function trace(e, c) {
  const f = frames(e), top = f.frames.find(t => !t.rt) || null
  return { rawFrames: f.frames, location: location(top, c), generatedLocation: generated(top), frames: f.frames.map(t => ({ generated: generated(t), source: null, runtime: t.rt })), truncated: { stack: f.truncated, frames: f.truncated, locations: f.frames.some(t => path(t.url).length > 1024) } }
}
function causes(e, c) {
  const seen = new Set([e]), out = []
  let next = get(e, 'cause'), truncated = false
  while (next != null) {
    if (seen.has(next) || out.length >= LIMIT.causes) { truncated = true; break }
    seen.add(next)
    const t = trace(next, c)
    out.push({ depth: out.length + 1, type: clean(get(next, 'name') || typeof next, 80), code: clean(get(next, 'code') || '', 80) || null, msg: clean(get(next, 'message') || (typeof next === 'string' ? next : ''), 300), ...t })
    next = get(next, 'cause')
  }
  return { entries: out, truncated }
}
// Component via the DOM: roots with data-b="<hash>" (prod) → parent chain via ancestors with data-b.
function byDom(el) {
  const chain = []
  for (; el && chain.length < 33; el = el.parentNode) if (el.getAttribute && el.getAttribute('data-b')) chain.push({ h: el.getAttribute('data-b') })
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
const MAPS = new Map(), pending = []
let loading = 0, enriching = 0
const B64 = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/'
export function decode(m) {
  if (typeof m !== 'string' || m.length > LIMIT.sourceBytes) return []
  const lines = []
  let src = 0
  for (const seg of m.split(';').slice(0, LIMIT.mapLines)) {
    let line = null
    for (const part of seg.split(',')) {
      if (!part) continue
      const v = []
      for (let i = 0, n = 0, sh = 0; i < part.length; i++) {
        const d = B64.indexOf(part[i])
        if (d < 0 || sh > 30) return []
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
async function source(url, signal) {
  if (opts.load) {
    const text = await opts.load(url, { signal })
    return typeof text === 'string' && text.length <= LIMIT.sourceBytes && new TextEncoder().encode(text).byteLength <= LIMIT.sourceBytes ? text : null
  }
  if (!F0) return null
  const r = await F0(url, { signal, credentials: 'same-origin' })
  if (r.ok === false || +(r.headers && r.headers.get('content-length')) > LIMIT.sourceBytes) return null
  if (!r.body || !r.body.getReader) { const text = await r.text(); return text.length <= LIMIT.sourceBytes && new TextEncoder().encode(text).byteLength <= LIMIT.sourceBytes ? text : null }
  const reader = r.body.getReader(), decoder = new TextDecoder()
  let text = '', bytes = 0
  try {
    for (;;) {
      const part = await reader.read()
      if (part.done) break
      bytes += part.value.byteLength
      if (bytes > LIMIT.sourceBytes) { await reader.cancel(); return null }
      text += decoder.decode(part.value, { stream: true })
    }
    return text + decoder.decode()
  } finally { reader.releaseLock() }
}
function pump() {
  while (loading < LIMIT.loads && pending.length) {
    const task = pending.shift()
    if (task.done) continue
    loading++
    Promise.resolve().then(() => source(task.url, task.controller.signal)).then(text => {
      const m = text && /sourceMappingURL=data:application\/json;base64,([A-Za-z0-9+/=]+)/.exec(text)
      const map = m && JSON.parse(atob(m[1]))
      const lines = map && map.version === 3 && decode(map.mappings)
      task.finish({ lines: lines && lines.length ? lines : null, status: lines && lines.length ? 'mapped' : 'unavailable' })
    }).catch(() => task.finish({ lines: null, status: 'unavailable' })).finally(() => { loading--; pump() })
  }
}
function mapOf(input) {
  let url
  try {
    const u = new URL(input), origin = G.location && G.location.origin
    if (!['http:', 'https:', 'file:'].includes(u.protocol) || (u.protocol !== 'file:' && origin && origin !== 'null' && u.origin !== origin)) return Promise.resolve({ lines: null, status: 'unavailable' })
    u.hash = ''; url = u.href
  } catch { return Promise.resolve({ lines: null, status: 'unavailable' }) }
  const hit = MAPS.get(url)
  if (hit) { MAPS.delete(url); MAPS.set(url, hit); return hit.promise }
  if (pending.length >= LIMIT.pending) return Promise.resolve({ lines: null, status: 'limited' })
  if (MAPS.size >= LIMIT.maps) {
    const old = [...MAPS].find(([, e]) => e.done)
    if (!old) return Promise.resolve({ lines: null, status: 'limited' })
    MAPS.delete(old[0])
  }
  const entry = { done: false }
  entry.promise = new Promise(resolve => {
    const task = { url, done: false, controller: new AbortController(), finish(result) {
      if (task.done) return
      task.done = entry.done = true
      clearTimeout(timer)
      const at = pending.indexOf(task)
      if (at >= 0) pending.splice(at, 1)
      if (result.status !== 'mapped' && MAPS.get(url) === entry) MAPS.delete(url)
      resolve(result)
    } }
    const timer = setTimeout(() => { task.controller.abort(); task.finish({ lines: null, status: 'timeout' }) }, LIMIT.deadline)
    pending.push(task)
  })
  MAPS.set(url, entry)
  pump()
  return entry.promise
}
async function toBrain(r, traces) {
  const jobs = []
  for (const item of traces) for (const [index, t] of item.rawFrames.entries()) {
    if (t.rt) continue
    jobs.push(mapOf(t.url).then(result => {
      const line = result.lines && result.lines[t.line - 1]
      if (Number.isSafeInteger(line) && line > 0 && item.frames[index]) item.frames[index].source = { file: path(t.url).replace(/^\//, '').slice(-1024), line, column: null }
      return result.status
    }))
  }
  const status = await Promise.all(jobs)
  for (const item of traces) {
    const first = item.frames.find(t => t.source)
    if (first) item.location = { component: item.location.component, ...first.source, precision: 'source-line' }
    delete item.rawFrames
  }
  for (let i = 1; i < traces.length; i++) r.causes[i - 1].location = traces[i].location
  r.location = traces[0].location
  r.frames = traces[0].frames
  const out = r.frames.filter(t => t.source).map(t => `${t.source.file}:${t.source.line}`)
  if (out.length) { r.src = out[0]; r.brainStack = out }
  r.mapping = { status: status.includes('timeout') ? 'timeout' : status.includes('limited') ? 'limited' : out.length ? 'mapped' : 'unavailable' }
  r.truncated.mapping = status.some(s => s === 'timeout' || s === 'limited')
  boundRecord(r)
  for (const s of sub) s(r)
}

// Bound the serialized notification too, including mapped locations and nested causes.
function boundRecord(r) {
  const size = () => new TextEncoder().encode(JSON.stringify(r)).byteLength
  r.truncated.output ||= false
  while (size() > 65536) {
    r.truncated.output = true
    const nested = [...r.causes].reverse().find(c => c.frames.length)
    if (nested) { nested.frames.pop(); nested.truncated.frames = true; continue }
    if (r.frames.length > 1) { r.frames.pop(); r.stack.pop(); if (r.brainStack) r.brainStack.pop(); r.truncated.frames = true; continue }
    if (r.causes.length) { r.causes.pop(); r.truncated.causes = true; continue }
    break
  }
}
function record(e, x, w) {
  if (e && e.kind === 'aborted') return // a cancelled request (brain-data ApiError): not an error
  const extracted = frames(e), f = extracted.frames, top = f.find(t => !t.rt) || null
  const owner = (opts.who && opts.who(x)) || (x && x.nodeType ? byDom(x) : null) || byStack(top)
  const c = owner ? { id: typeof owner.id === 'number' ? owner.id : null, h: clean(owner.h || '', 120) || null, name: clean(owner.name || '', 120) || null, file: owner.file ? clean(owner.file, 500).split(/[?#]/)[0] : null, chain: Array.isArray(owner.chain) ? owner.chain.slice(0, 32).map(n => clean(n, 120)) : [] } : null
  const type = clean(get(e, 'code') || get(e, 'name') || typeof e, 80)
  const line = top ? `${path(top.url).slice(-1024)}:${top.line}` : null
  const fp = hash([type, c && c.h, line, w].join('|'))
  let r = errors.find(r => r.fp === fp)
  if (r) (r.n++, (r.last = Date.now()))
  else {
    const t = trace(e, c), chain = causes(e, c)
    r = {
      fp, code: clean(get(e, 'code') || '', 80) || null, type, msg: MSG[get(e, 'code')] || clean(get(e, 'message') || e), where: clean(w || '', 120) || null, comp: c, line,
      stack: f.slice(0, LIMIT.frames).map(t => `${path(t.url).slice(-1024)}:${t.line}:${t.col}`), location: t.location, generatedLocation: t.generatedLocation, frames: t.frames, causes: chain.entries.map(({ rawFrames, ...item }) => item), truncated: { ...t.truncated, causes: chain.truncated, componentChain: !!(owner && owner.chain && owner.chain.length > 32), breadcrumbs: crumbs.length >= 30, locations: f.some(t => path(t.url).length > 1024), mapping: false }, mapping: { status: opts.maps ? 'pending' : 'disabled' }, crumbs: crumbs.map(c => ({ ...c })),
      build: clean(opts.build || '', 200) || null, ua: G.navigator ? clean(navigator.userAgent, 300) : null, at: Date.now(), last: Date.now(), n: 1,
    }
    boundRecord(r)
    errors.push(r)
    if (errors.length > 100) errors.shift()
    if (opts.maps && enriching < 100) {
      enriching++
      const traces = [t, ...chain.entries.slice(0, r.causes.length)]
      for (let i = 0; i < r.causes.length; i++) chain.entries[i].frames = r.causes[i].frames
      toBrain(r, traces).finally(() => { enriching-- })
    } else {
      if (opts.maps) { r.mapping.status = 'limited'; r.truncated.mapping = true }
      for (const item of chain.entries) delete item.rawFrames
    }
    if (opts.report && G.navigator && navigator.sendBeacon && sent < (opts.max || 20)) sent++, navigator.sendBeacon(opts.report, JSON.stringify(r))
  }
  for (const s of sub) s(r)
  if (opts.log !== false) print(e, r.fp)
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
    G.addEventListener('hashchange', () => crumb('nav', path(location.href)))
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
