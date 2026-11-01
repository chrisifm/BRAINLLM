// Dev observability panel. `brain run` loads this file; builds never contain it. Alt+Shift+O or `?brain-observe` opens it.
// It shows what the running page does: Web Vitals, requests, long frames, errors and live components, and "Pick" names
// the component and source file of an element. `__brainObserve.snapshot()` returns the same data as JSON for an agent.
import { live } from './brain-dev.js'
import { sub } from './brain-err.js'
import { vitals } from './brain-vitals.js'

const LIMIT = 40
const data = { vitals: {}, requests: [], frames: [], errors: 0, updates: 0 }
const keep = (list, item) => { list.unshift(item); if (list.length > LIMIT) list.pop() }
let host, box, on = false, timer, picking = null

const CSS = `:host{all:initial;position:fixed;left:16px;bottom:16px;z-index:2147483646;font:12px/1.45 system-ui,-apple-system,sans-serif;color:#1a1a1a}
.p{width:min(420px,calc(100vw - 32px));max-height:min(70vh,560px);overflow:auto;box-sizing:border-box;border-radius:16px;padding:12px 14px;background:rgba(255,255,255,.76);
-webkit-backdrop-filter:blur(20px) saturate(1.6);backdrop-filter:blur(20px) saturate(1.6);border:1px solid rgba(0,0,0,.08);
box-shadow:0 1px 3px rgba(16,24,40,.08),0 12px 32px rgba(16,24,40,.14)}@supports not (backdrop-filter:blur(1px)){.p{background:#fff}}
@media (prefers-color-scheme:dark){:host{color:#f2f2f2}.p{background:rgba(28,28,30,.84);border-color:rgba(255,255,255,.1)}}
.h{display:flex;justify-content:space-between;align-items:center;gap:8px;font-weight:600}h3{margin:10px 0 4px;font-size:11px;font-weight:600;opacity:.65}
.v{display:flex;flex-wrap:wrap;gap:6px;font-variant-numeric:tabular-nums}.v span{padding:2px 7px;border-radius:8px;background:rgba(127,127,127,.14)}
.good{color:#0a7d32}.needs-improvement{color:#a85b00}.poor,.bad{color:#d70015}
.r{display:flex;gap:8px;font-variant-numeric:tabular-nums}.r b{font-weight:500;min-width:30px}.r i{flex:1;font-style:normal;overflow:hidden;white-space:nowrap;text-overflow:ellipsis}.e{opacity:.6}
button{font:inherit;padding:3px 10px;border-radius:8px;border:1px solid rgba(127,127,127,.35);background:transparent;color:inherit;cursor:pointer}`

const el = (tag, cls, text) => Object.assign(document.createElement(tag), cls ? { className: cls } : {}, text === undefined ? {} : { textContent: text })
const fileOf = i => i.C?.f ?? null
const nameOf = i => i.C?.C?.name || i.C?.name || i.h

export function components() {
  const rows = []
  for (const [hash, set] of live) if (set.size) { const i = set.values().next().value; rows.push({ name: nameOf(i), file: fileOf(i), hash, instances: set.size }) }
  return rows.sort((a, b) => b.instances - a.instances)
}
// The innermost live component: brain-dev marks each root element of an instance with `$i`.
export function componentOf(node) {
  for (let n = node; n; n = n.parentNode ?? n.host) {
    const i = n.$i
    if (i) return { name: nameOf(i), file: fileOf(i), hash: i.h }
  }
  return null
}
export const snapshot = () => ({ path: location.pathname, vitals: { ...data.vitals }, requests: data.requests.slice(), longFrames: data.frames.slice(), errors: data.errors, hotUpdates: data.updates, components: components() })

function section(title, rows, empty) {
  const out = [el('h3', '', title)]
  if (!rows.length) out.push(el('div', 'e', empty))
  return out.concat(rows)
}
function draw() {
  if (!box) return
  const pick = el('button', '', picking ? 'Picking… Esc cancels' : 'Pick')
  pick.onclick = () => (picking ? stopPick() : startPick())
  const head = el('div', 'h', 'brain observe')
  head.append(pick)
  const metrics = el('div', 'v')
  for (const [name, m] of Object.entries(data.vitals)) metrics.append(el('span', m.rating, `${name} ${m.value}${name === 'CLS' ? '' : ' ms'}`))
  const row = (a, b, c, cls) => { const r = el('div', 'r' + (cls ? ' ' + cls : '')); r.append(el('b', '', a), el('i', '', b), el('span', '', c)); return r }
  box.replaceChildren(
    head,
    ...section('Web Vitals', metrics.childElementCount ? [metrics] : [], 'Waiting for the browser to report.'),
    ...section(`Requests (${data.requests.length})`, data.requests.slice(0, 8).map(r => row(String(r.status ?? '…'), `${r.method ?? ''} ${r.url}`, `${r.ms ?? '?'} ms`, r.status >= 400 ? 'bad' : '')), 'No api() request yet.'),
    ...section(`Long frames (${data.frames.length}) · errors ${data.errors} · hot updates ${data.updates}`, data.frames.slice(0, 4).map(f => row(`${f.ms}`, f.source || 'unattributed', 'ms')), 'No frame over 50 ms.'),
    ...section(`Components (${live.size})`, components().slice(0, 8).map(c => row(`×${c.instances}`, c.file ?? c.name, '')), 'None mounted.'),
  )
}
function toggle(force) {
  on = force ?? !on
  try { on ? sessionStorage.setItem('brain:observe', '1') : sessionStorage.removeItem('brain:observe') } catch {}
  if (!on) { stopPick(); clearInterval(timer); host?.remove(); return on }
  if (!host) {
    host = document.createElement('brain-observe')
    const root = host.attachShadow({ mode: 'open' })
    root.append(el('style', '', CSS))
    box = root.appendChild(el('div', 'p'))
  }
  document.body.append(host)
  draw()
  timer = setInterval(draw, 1000)
  return on
}
function startPick() {
  const mark = el('div')
  mark.style.cssText = 'position:fixed;z-index:2147483645;pointer-events:none;outline:2px solid #0071e3;background:rgba(0,113,227,.08);border-radius:3px'
  const tip = el('div')
  tip.style.cssText = 'position:fixed;z-index:2147483645;pointer-events:none;padding:2px 8px;border-radius:6px;background:#111;color:#fff;font:12px ui-monospace,monospace'
  document.body.append(mark, tip)
  const move = ev => {
    if (ev.target === host) return
    const r = ev.target.getBoundingClientRect(), c = componentOf(ev.target)
    Object.assign(mark.style, { left: r.left + 'px', top: r.top + 'px', width: r.width + 'px', height: r.height + 'px' })
    Object.assign(tip.style, { left: r.left + 'px', top: Math.max(0, r.top - 24) + 'px' })
    tip.textContent = c ? c.file ?? c.name : 'outside any component'
  }
  const click = ev => {
    if (ev.target === host) return
    ev.preventDefault(); ev.stopPropagation()
    const c = componentOf(ev.target)
    if (c) console.info('brain:observe', c)
    dispatchEvent(new CustomEvent('brain:observe-pick', { detail: c }))
    stopPick()
  }
  const key = ev => ev.key === 'Escape' && stopPick()
  addEventListener('mousemove', move, true); addEventListener('click', click, true); addEventListener('keydown', key, true)
  picking = () => { removeEventListener('mousemove', move, true); removeEventListener('click', click, true); removeEventListener('keydown', key, true); mark.remove(); tip.remove() }
  draw()
}
function stopPick() { picking?.(); picking = null; draw() }

export function start() {
  vitals(m => { data.vitals[m.name] = m })
  addEventListener('brain:net', ev => keep(data.requests, ev.detail))
  addEventListener('brain:hmr', () => data.updates++)
  sub.add(() => data.errors++)
  if (PerformanceObserver.supportedEntryTypes?.includes('long-animation-frame'))
    new PerformanceObserver(list => { for (const e of list.getEntries()) keep(data.frames, { ms: Math.round(e.duration), source: e.scripts?.[0]?.sourceURL ? new URL(e.scripts[0].sourceURL, location.href).pathname : '' }) }).observe({ type: 'long-animation-frame', buffered: true })
  addEventListener('keydown', ev => { if (ev.altKey && ev.shiftKey && ev.code === 'KeyO') { ev.preventDefault(); toggle() } })
  globalThis.__brainObserve = { snapshot, toggle, componentOf }
  let saved = false
  try { saved = sessionStorage.getItem('brain:observe') === '1' } catch {}
  if (saved || new URLSearchParams(location.search).has('brain-observe')) toggle(true)
}
