// BRAINLLM dev overlay (INTERNALS §20, #267). Development only: the dev client (crates/brain-serve/src/client.js)
// imports it; no page in production does. What it shows:
// - errors (a file that does not compile, or a runtime error from brain-err): a panel at the bottom that does not cover
//   the page, with the code, the snippet with the line marked, the fix and "Copy for the LLM";
// - warnings and info: a chip in a corner ("3 warnings") that opens the same list.
// API (format agreed with TOOLCHAIN #47): show(diags, { file }) replaces the diagnostics of that file (an empty list =
// it compiles again) · error(record) adds a runtime error (brain-err: { code, msg, src, brainStack }) · clear().
// A diagnostic has the shape of `brain check --json` plus `snippet: { line, text: [...] }` (a few lines around `line`).
const G = globalThis
const byFile = new Map() // file → diagnostics
let runtime = [] // runtime errors, newest last
let host = null, root = null, open = false

const esc = s => String(s ?? '').replace(/[&<>"]/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' })[c])
const all = () => [...byFile.values()].flat()

// The text a model needs to fix it without opening anything else: location, message, snippet, fix and the JSON
export function forLLM(d) {
  const out = [`${d.code} ${d.file || d.src || ''}${d.line ? `:${d.line}${d.col ? ':' + d.col : ''}` : ''} (${d.level || 'error'})`, d.msg]
  if (d.snippet && d.snippet.text)
    d.snippet.text.forEach((t, i) => {
      const n = d.snippet.line + i
      out.push(`${n === d.line ? '>' : ' '} ${String(n).padStart(4)} | ${t}`)
    })
  if (d.fix) out.push(`fix: ${d.fix}`)
  if (d.brainStack) out.push('stack:', ...[].concat(d.brainStack))
  if (d.location || d.frames || d.causes || d.rid) out.push('structured trace (development only):', JSON.stringify(traceOf(d)))
  if (d.mapping?.status && d.mapping.status !== 'mapped') out.push('Location mapping is incomplete; do not treat generated coordinates as original source coordinates.')
  out.push('Agent next steps: inspect the exact source revision, apply a focused change, run brain check --json, then reproduce the failing action. Record evidence with brain history and refresh brain quality.')
  if (d.file) out.push('brain check --json:', JSON.stringify((byFile.get(d.file) || [d]).map(({ snippet, ...x }) => x)))
  return out.join('\n')
}

function traceOf(d) {
  return { schemaVersion: 1, requestId: d.rid || d.requestId || null, component: d.comp || null,
    location: d.location || null, generatedLocation: d.generatedLocation || null,
    mapping: d.mapping || null, truncated: d.truncated || null,
    frames: Array.isArray(d.frames) ? d.frames.slice(0, 24) : [],
    causes: Array.isArray(d.causes) ? d.causes.slice(0, 6) : [] }
}

function item(d, i) {
  const loc = d.file ? `${d.file}:${d.line || ''}${d.col ? ':' + d.col : ''}` : d.src || ''
  const snip = d.snippet && d.snippet.text
    ? `<pre class="snip">${d.snippet.text.map((t, j) => `<span class="${d.snippet.line + j === d.line ? 'hit' : ''}">${String(d.snippet.line + j).padStart(4)} │ ${esc(t)}</span>`).join('\n')}</pre>`
    : ''
  return `<li class="${d.level === 'warning' || d.level === 'info' ? d.level : 'error'}"><p class="t"><b>${esc(d.code || 'Error')}</b> <span>${esc(loc)}</span></p>` +
    `<p>${esc(d.msg)}</p>${snip}${d.fix ? `<p class="fix">fix: ${esc(d.fix)}</p>` : ''}` +
    `${d.location || d.frames || d.causes ? `<details><summary>Development trace and mapping</summary><pre>${esc(JSON.stringify(traceOf(d), null, 2))}</pre></details>` : ''}` +
    `<button type="button" data-copy="${i}">Copy for the LLM</button></li>`
}

function render() {
  const d = G.document
  if (!d || !d.body) return
  const list = [...all(), ...runtime.map(r => ({ level: 'error', ...r }))]
  const errors = list.filter(x => x.level !== 'warning' && x.level !== 'info'), notes = list.filter(x => x.level === 'warning' || x.level === 'info')
  if (!list.length) {
    if (host) host.remove()
    return
  }
  if (!host) {
    host = d.createElement('brain-overlay')
    root = host.attachShadow ? host.attachShadow({ mode: 'open' }) : host
    root.addEventListener('click', e => {
      let b = e.target
      while (b && b.tagName !== 'BUTTON') b = b.parentNode
      if (!b) return
      if (b.hasAttribute('data-chip')) return void ((open = !open), render())
      const x = shown[+b.getAttribute('data-copy')]
      if (x) copy(forLLM(x), b)
    })
  }
  const panel = errors.length || open
  shown = panel ? (errors.length ? [...errors, ...(open ? notes : [])] : notes) : []
  root.innerHTML = `<style>${CSS}</style>` +
    (panel ? `<section class="panel" role="alert" aria-label="Problems"><ul>${shown.map(item).join('')}</ul></section>` : '') +
    (notes.length ? `<button type="button" class="chip" data-chip aria-expanded="${open}">${notes.length} ${notes.length === 1 ? 'warning' : 'warnings'}</button>` : '')
  if (!host.isConnected) d.body.appendChild(host)
}
let shown = []

function copy(text, btn) {
  const done = () => ((btn.textContent = 'Copied'), setTimeout(() => (btn.textContent = 'Copy for the LLM'), 1500))
  const nav = G.navigator
  if (nav && nav.clipboard && nav.clipboard.writeText) nav.clipboard.writeText(text).then(done, () => {})
  else done()
  G.__brainOverlayCopied = text // tests and tools read what was copied
}

export function show(diags, o = {}) {
  // show(all, 'comp/x.brain') (the dev client) or show(all, { file })
  const file = (typeof o === 'string' ? o : o.file) || (diags[0] && diags[0].file) || ''
  if (diags.length) byFile.set(file, diags)
  else byFile.delete(file)
  render()
}
export function error(r) {
  if (!r) return
  // the same error again (brain-err notifies twice: once more when the source map resolves) merges into the first
  const same = x => x.code === r.code && x.msg === r.msg && x.src === r.src
  const old = runtime.find(same)
  runtime = [...runtime.filter(x => !same(x)), { ...old, ...Object.fromEntries(Object.entries(r).filter(([, v]) => v != null)) }].slice(-20)
  render()
}
export function clear() {
  byFile.clear()
  runtime = []
  open = false
  render()
}

const CSS = `
:host,.panel,.chip{all:initial}
.panel{position:fixed;left:12px;right:12px;bottom:12px;max-height:45vh;overflow:auto;z-index:2147483647;padding:14px 18px;
 border-radius:14px;background:rgba(22,22,26,.95);color:#f3f3f5;font:13px/1.5 ui-sans-serif,system-ui,sans-serif;
 box-shadow:0 1px 2px rgba(0,0,0,.2),0 18px 40px -12px rgba(0,0,0,.45)}
ul{margin:0;padding:0;list-style:none;display:grid;gap:14px}
li{border-left:3px solid #ff6b5f;padding-left:12px}
li.warning{border-color:#f5b54a}
li.info{border-color:#6aa8ff}
p{margin:2px 0}
.t b{color:#ff8a80}
li.warning .t b{color:#f5b54a}
.t span{color:#b4b4bd;font:12px ui-monospace,monospace}
.snip{margin:6px 0;padding:8px 10px;border-radius:8px;background:rgba(255,255,255,.06);font:12px/1.45 ui-monospace,monospace;overflow:auto;white-space:pre}
.snip .hit{background:rgba(255,107,95,.22);display:inline-block;min-width:100%}
.fix{color:#9fe0b0}
button{font:12px ui-sans-serif,system-ui,sans-serif;margin-top:6px;padding:4px 10px;border-radius:8px;border:1px solid rgba(255,255,255,.2);
 background:rgba(255,255,255,.08);color:#f3f3f5;cursor:pointer}
.chip{position:fixed;right:16px;bottom:16px;z-index:2147483647;padding:6px 12px;border-radius:999px;border:0;cursor:pointer;
 font:600 12px ui-sans-serif,system-ui,sans-serif;color:#1a1a1a;background:#f5b54a;box-shadow:0 6px 18px -6px rgba(0,0,0,.35)}
.panel ~ .chip{top:16px;bottom:auto}
@media (prefers-reduced-motion:no-preference){button{transition:background .15s}}
`
