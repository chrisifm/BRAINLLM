// Precision bench (QA #172, INTERNALS §41.3): on pages with 1 000 components, one write must mutate exactly the DOM
// nodes bound to the written field. A MutationObserver (childList, characterData, attributes, whole document) records
// every mutation of one action; each mutation becomes a label (`<data-k owner>><tag.class>#text`, `…@attr`,
// `+row-7` / `-row-7` for added/removed keyed nodes) and the multiset is compared with the exact expected one.
//
// Cases: local state · global store (one field, an equal write, the other field) · group state (provide + store in a
// subtree, depends on RUNTIME/COMPILER #169/#170) · a 1 000-row keyed list (update one field, swap two rows, remove one).
// A case whose source files do not compile is BLOCKED (the files are left out of the working copy).
//
// Usage: node bench/precision/run.mjs [--modes dev,prod] [--port 5199] [--no-build]
// Env:   BRAIN (binary, default target/debug/brain) · BRAIN_CHROME (see bench/lib/cdp.mjs)
// Output: results/summary.json + summary.md (committed) · results/raw/<mode>.json (gitignored)
// Exit:  0 every runnable case exact · 1 any extra (or missing) mutation — regression gate · 2 the bench could not run
import { spawn, execFileSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { cpSync, existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { join, relative } from 'node:path'
import { launch, openPage } from '../lib/cdp.mjs'
import { createFaultServer } from '../http/server.mjs'

const HERE = new URL('.', import.meta.url).pathname
const ROOT = new URL('../..', import.meta.url).pathname
const SRC = join(HERE, 'app'), WORK = join(HERE, '.work'), OUT = join(HERE, 'results')
const BRAIN = process.env.BRAIN ?? join(ROOT, 'target/debug/brain')
const argv = process.argv.slice(2)
const arg = (k, d) => { const i = argv.indexOf(`--${k}`); return i >= 0 ? argv[i + 1] : d }
const MODES = arg('modes', 'dev,prod').split(',')
const DEV_PORT = Number(arg('port', 5199))
const NO_BUILD = argv.includes('--no-build')
const sleep = (ms) => new Promise((r) => setTimeout(r, ms))
const t0 = Date.now()

const children = new Set()
const cleanup = () => { for (const c of children) { try { c.kill('SIGTERM') } catch {} } children.clear() }
process.on('exit', cleanup)
process.on('SIGINT', () => (cleanup(), process.exit(130)))
process.on('SIGTERM', () => (cleanup(), process.exit(143)))
function fatal(msg) { console.error('precision bench: ' + msg); cleanup(); process.exit(2) }
if (!existsSync(BRAIN)) fatal(`no binary at ${BRAIN}; run \`cargo build -p brain-cli\` first`)
const brainSha = createHash('sha256').update(readFileSync(BRAIN)).digest('hex').slice(0, 12)
const isError = (d) => !/^(warn|warning|info|hint|note)$/i.test(String(d.severity ?? d.level ?? 'error'))
const brainCheck = (dir) => {
  let s
  try { s = execFileSync(BRAIN, ['check', '--json', '.'], { cwd: dir, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] }) } catch (e) { s = e.stdout }
  return JSON.parse(s || '[]')
}

const range = (n, f) => Array.from({ length: n }, (_, i) => f(i))
const RT = 'RUNTIME runtime/brain.js (signals, keyed lists)'
// view · needs (files) · ready (count check) · click selector · expected labels
const CASES = [
  { id: 'P1', title: 'local state: one counter among 1 000 components', view: '/scale', needs: ['views/scale.brain', 'comp/cell.brain', 'comp/pad.brain'], owner: RT,
    ready: `document.querySelectorAll('.cell').length === 1000`, click: `[data-k="cell-500"] .inc`,
    expect: ['cell-500>span.v#text', 'cell-500>span.v@data-n'] },
  { id: 'P2', title: 'global store: write one field (10 readers of it, 10 of another field)', view: '/scale', needs: ['views/scale.brain', 'comp/global-a.brain'], owner: RT + ' · channels store',
    ready: `document.querySelectorAll('.cell').length === 1000 && document.querySelectorAll('.ga').length === 10`, click: `[data-a="incA"]`,
    expect: range(10, (i) => `ga-${i}>span.ga#text`) },
  { id: 'P3', title: 'global store: write an equal value (Object.is) → nothing', view: '/scale', needs: ['views/scale.brain'], owner: RT + ' · channels store',
    ready: `document.querySelectorAll('.ga').length === 10`, click: `[data-a="sameA"]`, expect: [] },
  { id: 'P4', title: 'global store: write the other field', view: '/scale', needs: ['views/scale.brain', 'comp/global-b.brain'], owner: RT + ' · channels store',
    ready: `document.querySelectorAll('.gb').length === 10`, click: `[data-a="incB"]`,
    expect: range(10, (i) => `gb-${i}>span.gb#text`) },
  { id: 'P5', title: 'group state: provide(Cart, store()) per subtree; write x in subtree one', view: '/group', needs: ['views/group.brain', 'comp/group-root.brain', 'comp/group-reader.brain'], owner: 'RUNTIME #169 (store in provide) · COMPILER #170',
    ready: `document.querySelectorAll('.cell').length === 1000 && document.querySelectorAll('.gr').length === 20`, click: `[data-a="incX-one"]`,
    expect: range(5, (i) => `one-x-${i}>span.gr#text`) },
  { id: 'P6', title: '1 000-row keyed list: update one row field', view: '/list', needs: ['views/list.brain'], owner: RT,
    ready: `document.querySelectorAll('[data-k=list] > li').length === 1000`, click: `[data-a="bump"]`, expect: ['row-500>span.hits#text'] },
  { id: 'P7', title: '1 000-row keyed list: swap rows 2 and 999 (moves only those two)', view: '/list', needs: ['views/list.brain'], owner: RT,
    ready: `document.querySelectorAll('[data-k=list] > li').length === 1000`, click: `[data-a="swap"]`,
    expect: ['+row-2', '-row-2', '+row-999', '-row-999'], order: `[...document.querySelectorAll('[data-k=list] > li')].map((l) => l.dataset.k).filter((k, i) => i < 3 || i > 996).join(',')`,
    wantOrder: 'row-1,row-999,row-3,row-998,row-2,row-1000' },
  { id: 'P8', title: '1 000-row keyed list: remove one row', view: '/list', needs: ['views/list.brain'], owner: RT,
    ready: `document.querySelectorAll('[data-k=list] > li').length === 1000`, click: `[data-a="drop"]`, expect: ['-row-500'] },
]

// observe one action and label every mutated node
const MEASURE = (sel) => `(async () => {
  const k = (n) => { const e = n && (n.nodeType === 1 ? n : n.parentElement); return (e && e.closest('[data-k]')?.dataset.k) || '?' }
  const d = (e) => e ? e.tagName.toLowerCase() + (e.classList[0] ? '.' + e.classList[0] : '') : '?'
  const kn = (n) => n.nodeType === 1 && n.dataset && n.dataset.k ? n.dataset.k : null
  const recs = []
  const mo = new MutationObserver((rs) => recs.push(...rs))
  mo.observe(document.documentElement, { subtree: true, childList: true, characterData: true, attributes: true })
  const t = performance.now()
  document.querySelector(${JSON.stringify(sel)}).click()
  await new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)))
  await new Promise((r) => setTimeout(r, 80))
  recs.push(...mo.takeRecords()); mo.disconnect()
  const ms = performance.now() - t
  const out = []
  for (const r of recs) {
    if (r.type === 'characterData') out.push(k(r.target) + '>' + d(r.target.parentElement) + '#text')
    else if (r.type === 'attributes') out.push(k(r.target) + '>' + d(r.target) + '@' + r.attributeName)
    else {
      for (const n of r.addedNodes) out.push(kn(n) ? '+' + kn(n) : k(r.target) + '>' + d(r.target) + '+' + (n.nodeType === 3 ? '#text' : n.nodeType === 8 ? '#comment' : d(n)))
      for (const n of r.removedNodes) out.push(kn(n) ? '-' + kn(n) : k(r.target) + '>' + d(r.target) + '-' + (n.nodeType === 3 ? '#text' : n.nodeType === 8 ? '#comment' : d(n)))
    }
  }
  return { records: recs.length, labels: out, ms: Math.round(ms) }
})()`

function multisetDiff(got, want) {
  const g = [...got], extra = [], missing = []
  for (const w of want) { const i = g.indexOf(w); if (i >= 0) g.splice(i, 1); else missing.push(w) }
  extra.push(...g)
  return { extra, missing }
}

function prepare() {
  rmSync(join(WORK, 'app'), { recursive: true, force: true }); mkdirSync(WORK, { recursive: true })
  const bad = new Map()
  for (const d of brainCheck(SRC).filter(isError)) if (!bad.has(d.file)) bad.set(d.file, `${d.code} ${d.file}:${d.line} ${d.msg}`)
  // a view that imports a left-out component cannot compile either: leave out views that need a bad file
  for (const c of CASES) if (c.needs.some((f) => bad.has(f))) for (const f of c.needs.filter((f) => f.startsWith('views/'))) if (!bad.has(f)) bad.set(f, 'needs ' + c.needs.find((x) => bad.has(x)) + ': ' + bad.get(c.needs.find((x) => bad.has(x))))
  cpSync(SRC, join(WORK, 'app'), { recursive: true, filter: (p) => { const r = relative(SRC, p); return !bad.has(r) && !/^(dist|\.brain)(\/|$)/.test(r) } })
  const left = brainCheck(join(WORK, 'app')).filter(isError)
  if (left.length) fatal('the working copy still has errors:\n' + JSON.stringify(left, null, 1))
  return bad
}

async function runMode(mode, chromePort, bad) {
  const app = join(WORK, 'app')
  let srv, devProc = null, devLog = '', buildMs = null
  if (mode === 'dev') {
    devProc = spawn(BRAIN, ['run', '--port', String(DEV_PORT)], { cwd: app, stdio: ['ignore', 'pipe', 'pipe'] })
    children.add(devProc)
    devProc.stdout.on('data', (d) => (devLog += d)); devProc.stderr.on('data', (d) => (devLog += d))
    let up = false
    for (let i = 0; i < 150 && !up; i++) { try { up = (await fetch(`http://127.0.0.1:${DEV_PORT}/list`)).ok } catch { await sleep(100) } }
    if (!up) fatal(`brain run did not answer on :${DEV_PORT}:\n${devLog}`)
    srv = createFaultServer({ upstream: `http://127.0.0.1:${DEV_PORT}` })
  } else {
    if (!NO_BUILD) {
      const b0 = Date.now()
      try { execFileSync(BRAIN, ['build'], { cwd: app, stdio: 'pipe' }) } catch (err) { fatal('`brain build` failed:\n' + (err.stdout || '') + (err.stderr || '')) }
      buildMs = Date.now() - b0
    }
    srv = createFaultServer({ staticDir: join(app, 'dist') })
  }
  const base = await srv.listen(0)
  const url = (p) => base + p + (mode === 'prod' ? '/' : '')
  const page = await openPage(chromePort, 'about:blank')
  const logs = []
  page.on((m) => {
    if (m.method === 'Runtime.consoleAPICalled' && m.params.type === 'error') logs.push('[error] ' + m.params.args.map((x) => x.value ?? x.description ?? '').join(' '))
    if (m.method === 'Runtime.exceptionThrown') logs.push('[exception] ' + (m.params.exceptionDetails.exception?.description || m.params.exceptionDetails.text))
  })
  const results = []
  for (const c of CASES) {
    const why = c.needs.map((f) => bad.get(f)).find(Boolean)
    if (why) { results.push({ id: c.id, status: 'BLOCKED', evidence: why }); continue }
    let r
    try {
      await page.send('Page.navigate', { url: url(c.view) })
      let ok = false
      for (let t = 0; t < 15000 && !ok; t += 100) { ok = await page.evaluate(`document.readyState === 'complete' && (${c.ready})`).catch(() => false); if (!ok) await sleep(100) }
      if (!ok) throw new Error(`${c.view} did not render the expected nodes`)
      await sleep(600)
      // production hydrates an island on its first interaction by rendering it again and swapping it in (§38): trigger
      // that with a click on the inert <h1> and record it apart, so the measured write sees a hydrated page
      const warm = await page.evaluate(MEASURE('main h1'))
      for (let t = 0; t < 5000; t += 100) { if (await page.evaluate(`!document.querySelector('script[type="b/state"]')`)) break; await sleep(100) }
      await sleep(200)
      // a write that never takes effect (page not hydrated yet) shows as "missing": retry once after a pause
      let m = await page.evaluate(MEASURE(c.click))
      if (!m.labels.length && c.expect.length) { await sleep(1000); m = await page.evaluate(MEASURE(c.click)) }
      const { extra, missing } = multisetDiff(m.labels, c.expect)
      const order = c.order ? await page.evaluate(c.order) : null
      const orderOk = !c.wantOrder || order === c.wantOrder
      const pass = !extra.length && !missing.length && orderOk
      const fmt = (a) => { const cnt = {}; for (const x of a) cnt[x] = (cnt[x] || 0) + 1; return Object.entries(cnt).slice(0, 8).map(([x, n]) => (n > 1 ? `${x} ×${n}` : x)).join(', ') + (Object.keys(cnt).length > 8 ? ` … (${Object.keys(cnt).length} distinct)` : '') }
      r = { id: c.id, status: pass ? 'PASS' : 'FAIL', mutated: m.labels.length, firstInteraction: { mutations: warm.labels.length, ms: warm.ms, sample: warm.labels.slice(0, 6) }, expected: c.expect.length, extra, missing, order, ms: m.ms,
        evidence: pass ? `${m.labels.length} mutation(s) = expected ${c.expect.length}${order ? '; order ok' : ''} (${m.ms} ms)`
          : [extra.length && `${extra.length} extra: ${fmt(extra)}`, missing.length && `${missing.length} missing: ${fmt(missing)}`, !orderOk && `order ${order} (want ${c.wantOrder})`].filter(Boolean).join('; ') }
    } catch (err) { r = { id: c.id, status: 'FAIL', evidence: 'error: ' + err.message } }
    results.push(r)
    process.stdout.write(`  ${mode} ${c.id} ${r.status}\n`)
  }
  page.close()
  await srv.close()
  if (devProc) { devProc.kill('SIGTERM'); children.delete(devProc) }
  return { results, buildMs, console: logs }
}

mkdirSync(join(OUT, 'raw'), { recursive: true })
const bad = prepare()
const chrome = await launch()
children.add({ kill: () => chrome.close() })
const modes = {}
for (const mode of MODES) {
  if (!['dev', 'prod'].includes(mode)) fatal(`unknown mode ${mode}`)
  const m0 = Date.now()
  modes[mode] = await runMode(mode, chrome.port, bad)
  modes[mode].ms = Date.now() - m0
  writeFileSync(join(OUT, 'raw', `${mode}.json`), JSON.stringify({ mode, ...modes[mode] }, null, 1))
}
chrome.close()

const rows = CASES.map((c) => ({ id: c.id, title: c.title, owner: c.owner, expected: c.expect.length,
  modes: Object.fromEntries(MODES.map((m) => { const r = modes[m].results.find((x) => x.id === c.id); return [m, { status: r.status, mutated: r.mutated ?? null, evidence: r.evidence }] })) }))
const all = rows.flatMap((r) => Object.values(r.modes).map((x) => x.status))
const count = (s) => all.filter((x) => x === s).length
const anyFail = count('FAIL') > 0
const totalMs = Date.now() - t0
const summary = { bench: 'precision', date: new Date().toISOString(), brainSha256: brainSha, modes: MODES, verdict: anyFail ? 'FAIL' : 'PASS',
  counts: { PASS: count('PASS'), FAIL: count('FAIL'), BLOCKED: count('BLOCKED') }, leftOut: Object.fromEntries(bad),
  runtimeMs: { total: totalMs, ...Object.fromEntries(MODES.map((m) => [m, modes[m].ms])), build: modes.prod?.buildMs ?? null },
  consoleErrors: Object.fromEntries(MODES.map((m) => [m, modes[m].console])),
  firstInteraction: Object.fromEntries(MODES.map((m) => [m, Object.fromEntries(modes[m].results.filter((r) => r.firstInteraction).map((r) => [r.id, r.firstInteraction]))])), rows }
writeFileSync(join(OUT, 'summary.json'), JSON.stringify(summary, null, 1) + '\n')
const esc = (s) => String(s).replace(/\|/g, '\\|')
const ev = (r) => { const e = Object.entries(r.modes); const b = e.filter(([, x]) => x.status !== 'PASS'); return (b.length ? b : e.slice(0, 1)).map(([m, x]) => `${m}: ${x.evidence}`).join(' · ') }
writeFileSync(join(OUT, 'summary.md'), [
  '# Precision bench — results', '',
  `Binary sha256 \`${brainSha}\` · ${summary.date} · total ${(totalMs / 1000).toFixed(1)} s · PASS ${summary.counts.PASS} · FAIL ${summary.counts.FAIL} · BLOCKED ${summary.counts.BLOCKED} · verdict **${summary.verdict}**`, '',
  `| # | Case | Expected | ${MODES.join(' | ')} | Owner | Evidence |`, `|---|---|---|${MODES.map(() => '---').join('|')}|---|---|`,
  ...rows.map((r) => `| ${r.id} | ${esc(r.title)} | ${r.expected} | ${MODES.map((m) => (r.modes[m].status === 'PASS' ? `pass (${r.modes[m].mutated})` : `**${r.modes[m].status}**${r.modes[m].mutated != null ? ` (${r.modes[m].mutated})` : ''}`)).join(' | ')} | ${esc(r.owner)} | ${esc(ev(r))} |`),
  '', '**First interaction (informational, not gated):** ' + MODES.map((m) => `${m}: ` + Object.entries(summary.firstInteraction[m]).filter(([id]) => ['P1', 'P5', 'P6'].includes(id)).map(([id, f]) => `${id} view ${f.mutations} mutation(s) in ${f.ms} ms`).join(', ')).join(' · ') + ' (a click on the inert `<h1>` before the measured write; in production it renders the island again and swaps it in).',
  '', 'Numbers in parentheses: labelled mutations observed (expected count in the third column). Raw labels: `results/raw/<mode>.json` (not committed).', '',
].join('\n'))
const pad = (s, n) => (String(s) + ' '.repeat(n)).slice(0, n)
console.log(`precision bench · brain ${brainSha} · ${(totalMs / 1000).toFixed(1)} s`)
for (const r of rows) console.log(`${pad(r.id, 4)}${pad(r.title, 64)}${MODES.map((m) => pad(r.modes[m].status, 8)).join('')}${ev(r).slice(0, 170)}`)
console.log(`verdict ${summary.verdict} → bench/precision/results/summary.md`)
process.exit(anyFail ? 1 : 0)
