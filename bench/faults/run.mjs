// Fault injection bench (QA #176, INTERNALS §42 "Every component is isolated by default"): a component nested three
// levels deep (Outer → Middle → Inner, bench/faults/app) fails on demand in its render, fx, drv, a click handler, an
// async handler, a resource() without {:catch}, during SSR and during prerender; load() fails on demand too. Every
// fault runs in development (`brain run`) and in production (`brain build` + `brain start`) in real headless Chrome,
// and is judged on five criteria:
//   C1 isolation — only Inner shows its fallback; the parent, the grandparent, three siblings and the view keep
//      counting on real mouse clicks, and navigation works;
//   C2 retry     — the fallback's Retry (after the fault is turned off) remounts Inner and it works again;
//   C3 reported  — dev: the error reaches brain_errors (the dev server's browser-error ring, printed by `brain run` as
//      `[browser] {…}`) with `comp/inner.brain:<line>`; prod: SSR/load failures reach logs/app.log with the request id;
//   C4 handlers  — a failing handler (sync and async) keeps Inner mounted and interactive, and a toast appears;
//   C5 dev compile error — invalid code written into comp/inner.brain while `brain run` is up stays isolated and
//      fixing the file recovers the page through HMR.
// See README.md for the method and results/summary.md for the last run.
//
// Usage: node bench/faults/run.mjs [--modes dev,prod] [--port 5197] [--only render,click]
// Env:   BRAIN (binary to test, default target/debug/brain; copied to the work dir first) · BRAIN_FAULTS_WORK (work dir,
//        default ~/tmp/brainllm-faults) · BRAIN_CHROME (see bench/lib/cdp.mjs)
// Output: results/summary.json + summary.md (committed) · results/raw/<mode>.json (gitignored)
// Exit:  0 every applicable criterion passes · 1 any FAIL · 2 the bench itself could not run
import { spawn, spawnSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { copyFileSync, cpSync, existsSync, mkdirSync, readFileSync, rmSync, writeFileSync, chmodSync } from 'node:fs'
import { homedir } from 'node:os'
import { join } from 'node:path'
import { launch, openPage } from '../lib/cdp.mjs'

const HERE = new URL('.', import.meta.url).pathname
const ROOT = new URL('../..', import.meta.url).pathname
const OUT = join(HERE, 'results')
const argv = process.argv.slice(2)
const arg = (k, d) => { const i = argv.indexOf(`--${k}`); return i >= 0 ? argv[i + 1] : d }
const MODES = arg('modes', 'dev,prod').split(',')
const DEV_PORT = Number(arg('port', 5197))
const ONLY = arg('only', null)?.split(',')
const WORK = process.env.BRAIN_FAULTS_WORK ?? join(homedir(), 'tmp', 'brainllm-faults')
const SRC_BIN = process.env.BRAIN ?? join(ROOT, 'target/debug/brain')
const BRAIN = join(WORK, 'brain')
const APP = join(WORK, 'app')
const sleep = (ms) => new Promise((r) => setTimeout(r, ms))
const t0 = Date.now()
const home = (s) => String(s).replaceAll(homedir(), '~')

// ---------- children are killed by their own PID only ----------
const children = new Set()
const cleanup = () => { for (const c of children) { try { c.kill('SIGTERM') } catch {} } children.clear() }
process.on('exit', cleanup)
process.on('SIGINT', () => (cleanup(), process.exit(130)))
process.on('SIGTERM', () => (cleanup(), process.exit(143)))
function fatal(msg) { console.error('faults bench: ' + msg); restoreInner(); cleanup(); process.exit(2) }

if (!existsSync(SRC_BIN)) fatal(`no binary at ${SRC_BIN}; run \`cargo build -p brain-cli\` first`)
mkdirSync(WORK, { recursive: true })
// macOS kills (SIGKILL, code signature) a binary copied over a previous copy: remove it first.
rmSync(BRAIN, { force: true }); copyFileSync(SRC_BIN, BRAIN); chmodSync(BRAIN, 0o755) // a concurrent rebuild of target/ cannot change what is tested
const brainSha = createHash('sha256').update(readFileSync(BRAIN)).digest('hex').slice(0, 12)
const brain = (args, cwd = APP) => { const r = spawnSync(BRAIN, args, { cwd, encoding: 'utf8' }); return { code: r.status, out: r.stdout || '', err: r.stderr || '' } }

// fresh copy of the app (the dev compile-error case edits comp/inner.brain; the repo copy is never touched)
rmSync(APP, { recursive: true, force: true })
cpSync(join(HERE, 'app'), APP, { recursive: true, filter: (s) => !/\/(dist|logs|\.brain)(\/|$)/.test(s.slice(join(HERE, 'app').length)) })
const INNER = join(APP, 'comp/inner.brain')

// The resource case needs `{#await res}` with no {:catch}. B1101 makes that an error; the bench app turns it off with
// brain.json "lint": {"B1101": "off"} (INTERNALS §20). If the compiler still refuses the file (the override is honored
// by `brain check` but not by the compile step), the work copy gets a {:catch} so every other case can run, and the
// resource case is recorded as BLOCKED with the compiler's message. It runs again by itself once the override works.
const BLOCKED = {}
{
  const probe = join(WORK, 'app-probe')
  rmSync(probe, { recursive: true, force: true })
  cpSync(APP, probe, { recursive: true })
  const b = brain(['build'], probe)
  rmSync(probe, { recursive: true, force: true })
  const msg = (b.out + b.err).split('\n').find((l) => /B1101/.test(l))
  if (b.code !== 0 && msg) {
    const src = readFileSync(INNER, 'utf8')
    const caught = src.replace(/(\{#await res\}[^\n]*?)\{\/await\}/, '$1{:catch e}<span data-p="inner-res">res-caught</span>{/await}')
    if (caught === src) fatal('could not add {:catch} to the {#await res} line of comp/inner.brain')
    writeFileSync(INNER, caught)
    BLOCKED.resource = `\`brain build\` rejects comp/inner.brain although brain.json has "lint": {"B1101": "off"}: ${msg.trim().slice(0, 240)}`
    console.log('faults bench: resource case BLOCKED — ' + BLOCKED.resource)
  }
}
const INNER_SRC = readFileSync(INNER, 'utf8')
function restoreInner() { try { if (readFileSync(INNER, 'utf8') !== INNER_SRC) writeFileSync(INNER, INNER_SRC) } catch {} }
const check = brain(['check', '--json', '.'])
let diags = []
try { diags = JSON.parse(check.out || '[]') } catch { fatal('`brain check --json` printed no JSON:\n' + check.out + check.err) }
if (diags.some((d) => !/^(warn|warning|info|hint|note)$/i.test(String(d.level ?? d.severity ?? 'error')))) fatal('`brain check` reports errors in the bench app:\n' + check.out)

// .brain lines each fault must be reported at: the throw itself (line of `throw new Error`) or the call site.
const SRC_LINES = INNER_SRC.split('\n')
const lineOf = (re) => SRC_LINES.findIndex((l) => re.test(l)) + 1
const THROW_LINE = lineOf(/throw new Error\('fault:'/)
const callLine = (w) => (w === 'resource' ? lineOf(/fault:resource/) : lineOf(new RegExp(`explode\\('${w}'\\)`)))

// ---------- servers ----------
async function devServer(port) {
  const c = spawn(BRAIN, ['run', '--port', String(port)], { cwd: APP, stdio: ['ignore', 'pipe', 'pipe'] })
  children.add(c)
  let log = ''
  c.stdout.on('data', (d) => (log += d)); c.stderr.on('data', (d) => (log += d))
  let url = null
  for (let i = 0; i < 200 && !url; i++) {
    const m = log.match(/brain run → (http:\/\/[^\s]+)/)
    if (m) { try { if ((await fetch(m[1] + '/lab')).ok) url = m[1] } catch {} }
    if (!url) await sleep(100)
  }
  if (!url) { c.kill('SIGTERM'); children.delete(c); fatal(`brain run did not answer:\n${log}`) }
  const stop = async () => { c.kill('SIGTERM'); for (let i = 0; i < 50 && c.exitCode === null && c.signalCode === null; i++) await sleep(50); children.delete(c) }
  return { url, log: () => log, mark: () => log.length, since: (m) => log.slice(m), stop }
}

async function prodServer() {
  const b0 = Date.now()
  const b = brain(['build'])
  const buildMs = Date.now() - b0
  if (b.code !== 0) fatal('`brain build` failed:\n' + b.out + b.err)
  const port = await new Promise((ok) => { import('node:net').then(({ createServer }) => { const s = createServer(); s.listen(0, '127.0.0.1', () => { const p = s.address().port; s.close(() => ok(p)) }) }) })
  rmSync(join(APP, 'logs'), { recursive: true, force: true })
  const c = spawn(BRAIN, ['start', '--port', String(port)], { cwd: APP, stdio: ['ignore', 'pipe', 'pipe'] })
  children.add(c)
  let out = ''
  c.stdout.on('data', (d) => (out += d)); c.stderr.on('data', (d) => (out += d))
  const url = `http://127.0.0.1:${port}`
  let up = false
  for (let i = 0; i < 200 && !up; i++) { try { up = (await fetch(url + '/_health')).ok } catch { await sleep(100) } }
  if (!up) { c.kill('SIGTERM'); children.delete(c); fatal(`brain start did not answer on ${url}:\n${out}`) }
  const appLog = () => { const f = join(APP, 'logs/app.log'); return existsSync(f) ? readFileSync(f, 'utf8') : '' }
  const stop = async () => {
    const st = (() => { try { return JSON.parse(brain(['status', '--json']).out) } catch { return null } })()
    const kids = (st?.instances ?? []).map((i) => i.pid).filter(Boolean)
    brain(['stop'])
    for (let i = 0; i < 200 && c.exitCode === null && c.signalCode === null; i++) await sleep(50)
    if (c.exitCode === null && c.signalCode === null) c.kill('SIGKILL')
    for (const k of kids) { try { process.kill(k, 0); process.kill(k, 'SIGKILL') } catch {} }
    children.delete(c)
  }
  return { url, buildMs, build: b.out + b.err, log: appLog, mark: () => appLog().length, since: (m) => appLog().slice(m), out: () => out, stop }
}

// ---------- in-page probes ----------
const INIT = `
  window.__noise = { unhandled: [], errors: [] };
  addEventListener('unhandledrejection', (e) => __noise.unhandled.push(String((e.reason && (e.reason.message || e.reason.kind)) || e.reason)));
  addEventListener('error', (e) => __noise.errors.push(String(e.message)));
  window.__snap = () => {
    const P = (k) => document.querySelector('[data-p="' + k + '"]');
    const fbs = [...document.querySelectorAll('.brain-fallback, [role=alert], [data-brain-error]')].filter((e) => !e.closest('.brain-toast') && !(e.parentElement && e.parentElement.closest('.brain-fallback, [role=alert], [data-brain-error]')));
    const ov = document.getElementById('__brain_overlay');
    return {
      path: location.pathname,
      inner: !!P('inner'),
      innerText: P('inner') ? P('inner').textContent.replace(/\\s+/g, ' ').trim() : null,
      fallbacks: fbs.map((e) => ({ text: e.textContent.replace(/\\s+/g, ' ').trim().slice(0, 200), tag: e.tagName.toLowerCase(), cls: e.className || null, inMiddle: !!e.closest('[data-p=middle]'), retry: !!e.querySelector('button'), errorBox: e.hasAttribute('data-brain-error') })),
      toasts: [...document.querySelectorAll('.brain-toast')].filter((t) => !t.hidden && t.textContent.trim()).map((t) => t.textContent.trim()),
      overlay: !!(ov && ov.isConnected),
      overlayText: ov && ov.isConnected ? ov.textContent.slice(0, 300) : null,
      counters: Object.fromEntries(['view-inc', 'outer-inc', 'middle-inc', 'middle-sib', 'outer-sib', 'view-sib', 'inner-inc'].map((k) => [k, P(k) ? P(k).textContent.trim() : null])),
      noise: JSON.parse(JSON.stringify(window.__noise)),
    };
  };
`
const SIBLINGS = ['view-inc', 'outer-inc', 'middle-inc', 'middle-sib', 'outer-sib', 'view-sib']
const num = (s) => { const m = String(s ?? '').match(/(-?\d+)\s*$/); return m ? Number(m[1]) : null }

async function waitFor(page, expr, ms = 6000) {
  for (let t = 0; t < ms; t += 50) { if (await page.evaluate(expr).catch(() => false)) return true; await sleep(50) }
  return false
}
const snap = (page) => page.evaluate('window.__snap()')
// A real mouse click at the element's centre (CDP Input), so an overlay covering the page blocks it as it would block a
// person. Reports what element is on top at that point.
async function rc(page, sel) {
  const at = await page.evaluate(`(() => {
    const e = document.querySelector(${JSON.stringify(sel)});
    if (!e) return null;
    e.scrollIntoView({ block: 'center' });
    const r = e.getBoundingClientRect(), x = r.left + r.width / 2, y = r.top + r.height / 2, top = document.elementFromPoint(x, y);
    return { x, y, hit: !!top && (top === e || e.contains(top)), top: top ? (top.id ? '#' + top.id : top.tagName.toLowerCase() + (top.className ? '.' + top.className : '')) : null };
  })()`)
  if (!at) return { missing: sel }
  for (const type of ['mouseMoved', 'mousePressed', 'mouseReleased']) await page.send('Input.dispatchMouseEvent', { type, x: at.x, y: at.y, button: 'left', clickCount: type === 'mouseMoved' ? 0 : 1 })
  await sleep(120)
  return at
}
// C1 part: each sibling counter (view, grandparent, parent, three siblings) must count one real click.
async function siblingsWork(page) {
  const out = {}
  for (const k of SIBLINGS) {
    const before = num((await snap(page)).counters[k])
    const c = await rc(page, `[data-p="${k}"]`)
    let after = num((await snap(page)).counters[k])
    if (before !== null && after !== before + 1) { await sleep(400); after = num((await snap(page)).counters[k]) } // a lazy island may take a moment
    out[k] = { before, after, ok: before !== null && after === before + 1, top: c.missing ? 'missing' : c.top }
  }
  return { ok: Object.values(out).every((x) => x.ok), each: out }
}
async function navWorks(page, sel = '[data-p="nav-about"]') {
  const c = await rc(page, sel)
  const ok = await waitFor(page, `location.pathname.replace(/\\/$/, '') === '/about' && /About faults-test/.test(document.querySelector('h1')?.textContent || '')`, 6000)
  return { ok, top: c.missing ? 'missing' : c.top, path: await page.evaluate('location.pathname').catch(() => null) }
}
async function goto(page, url) {
  await page.send('Page.navigate', { url })
  const ok = await waitFor(page, `document.readyState === 'complete' && !!document.querySelector('h1')`, 10000)
  await sleep(600)
  return ok
}
// dev only: what the runtime itself recorded (brain-err), to tell a reporting gap from a capture gap
const rtErrors = (page) => page.evaluate(`import(location.origin + '/@brain/brain-err.js').then((m) => m.errors.map((r) => ({ msg: r.msg, src: r.src || null, comp: r.comp || null, where: r.where || null, n: r.n })), () => null)`).catch(() => null)

// brain_errors entries (dev log `[browser] {json}` lines) after a mark
function devErrors(srv, mark) {
  const out = []
  for (const l of srv.since(mark).split('\n')) {
    const i = l.indexOf('[browser] ')
    if (i < 0) continue
    try { out.push(JSON.parse(l.slice(i + 10))) } catch { out.push({ raw: l.slice(i + 10, i + 300) }) }
  }
  return out
}
function c3dev(entries, where) {
  const want = [THROW_LINE, callLine(where)]
  const mine = entries.filter((e) => new RegExp(`fault:${where}`).test(`${e.message} ${e.stack}`))
  const lines = [...new Set(mine.flatMap((e) => [...String(e.stack || '').matchAll(/(comp\/inner\.brain):(\d+)/g)].map((m) => `${m[1]}:${m[2]}`)))]
  const pass = mine.length > 0 && lines.some((l) => want.includes(Number(l.split(':')[1])))
  const brief = mine.length === 0 ? `no brain_errors entry for fault:${where} (${entries.length} entries in the window${entries.length ? ': ' + entries.map((e) => String(e.message).slice(0, 40)).join(' | ') : ''})`
    : `brain_errors: ${mine.length} entry; frames ${lines.join(', ') || 'none in inner.brain'} (want inner.brain:${want.join(' or ')})`
  return { pass, brief, entries: mine, want }
}
const logLines = (txt) => txt.split('\n').filter(Boolean).map((l) => { try { return JSON.parse(l) } catch { return { raw: l } } })

// ---------- faults ----------
const BOUNDARY = ['render', 'fx', 'drv', 'resource', 'ssr', 'prerender']
const FAULTS = [
  { id: 'render', title: 'render (markup expression)', path: '/lab', ctl: 'render' },
  { id: 'fx', title: 'fx', path: '/lab', ctl: 'fx' },
  { id: 'drv', title: 'drv', path: '/lab', ctl: 'drv' },
  { id: 'resource', title: 'resource() without {:catch}', path: '/lab', ctl: 'resource' },
  { id: 'click', title: 'click handler (sync)', path: '/lab', button: 'inner-sync' },
  { id: 'async', title: 'async handler (rejects after await)', path: '/lab', button: 'inner-async' },
  { id: 'ssr', title: 'SSR render (server.mjs, ?boom=1)', path: '/ssr?boom=1', heal: 'ssrHealed', where: 'ssr' },
  { id: 'prerender', title: 'prerender (build time)', path: '/pre', heal: 'ssrHealed', where: 'ssr' },
  { id: 'load', title: 'load() throws (?load=1)', path: '/ssr?load=1' },
  { id: 'compile', title: 'compile error in Inner (dev, HMR)', modes: ['dev'] },
].filter((f) => !ONLY || ONLY.includes(f.id))

const NA = (why) => ({ pass: null, brief: 'n/a — ' + why })
const V = (pass, brief, extra = {}) => ({ pass: !!pass, brief, ...extra })
const short = (s) => JSON.stringify(s.fallbacks.map((f) => `${f.inMiddle ? 'in Middle' : 'OUTSIDE Middle'}: "${f.text.slice(0, 50)}"`))

async function boundaryFault(page, srv, mode, f) {
  const where = f.where || f.id
  const raw = {}
  const prodSsr = mode === 'prod' && (f.id === 'ssr' || f.id === 'prerender')
  if (prodSsr) {
    const r = await fetch(srv.url + f.path)
    const html = await r.text()
    const mid = html.slice(html.indexOf('data-p="middle"'), html.indexOf('data-p="middle-sib"'))
    raw.http = { status: r.status, fallbackInMiddle: /brain-fallback|role="alert"/.test(mid), innerInHtml: html.includes('data-p="inner"'), siblingsInHtml: SIBLINGS.every((k) => html.includes(`data-p="${k}"`)) }
  }
  const lm = srv.mark()
  if (!(await goto(page, srv.url + f.path))) return { error: 'page did not load' }
  if (f.ctl) { await rc(page, `[data-ctl="${f.ctl}"]`); await waitFor(page, `!document.querySelector('[data-p=inner]')`, 3000) }
  else await waitFor(page, `!document.querySelector('[data-p=inner]')`, 3000)
  raw.after = await snap(page)
  raw.siblings = await siblingsWork(page)
  raw.afterSiblings = await snap(page)
  // C2: Retry with the fault still on (it must fall back again, keeping Retry: 2 failures < 3) — render only, which
  // turns the update-time throw into a mount-time throw
  if (f.id === 'render' && raw.after.fallbacks[0]?.retry) {
    await rc(page, '[data-p=middle] .brain-fallback button, [data-p=middle] [role=alert] button')
    await sleep(300)
    raw.retryStillFailing = await snap(page)
  }
  // heal, then Retry
  await rc(page, `[data-ctl="${f.ctl || f.heal}"]`)
  await sleep(200)
  raw.healed = await snap(page)
  raw.retryClick = await rc(page, '[data-p=middle] .brain-fallback button, [data-p=middle] [role=alert] button')
  await waitFor(page, `!!document.querySelector('[data-p=inner]') && /res-ok/.test(document.querySelector('[data-p=inner]').textContent)`, 3000)
  raw.retried = await snap(page)
  const i0 = num(raw.retried.counters['inner-inc'])
  await rc(page, '[data-p="inner-inc"]')
  raw.retriedInc = num((await snap(page)).counters['inner-inc'])
  raw.rt = mode === 'dev' ? await rtErrors(page) : null
  raw.nav = await navWorks(page)
  await sleep(mode === 'prod' ? 300 : 500)
  const a = raw.after
  // C1
  const fbOk = !a.inner && a.fallbacks.length === 1 && a.fallbacks[0].inMiddle && !a.overlay
  let c1 = V(fbOk && raw.siblings.ok && raw.nav.ok,
    `${a.inner ? 'Inner still rendered (no fallback)' : `fallbacks ${short(a)}`}; siblings ${raw.siblings.ok ? '6/6 count' : 'FAIL ' + JSON.stringify(Object.fromEntries(Object.entries(raw.siblings.each).filter(([, v]) => !v.ok)))}; nav ${raw.nav.ok ? 'ok' : 'FAIL ' + raw.nav.path}${a.overlay ? '; full-page overlay' : ''}`)
  if (prodSsr) {
    const h = raw.http
    const htmlOk = h.status === 200 && h.fallbackInMiddle && !h.innerInHtml && h.siblingsInHtml
    c1 = V(c1.pass && htmlOk, `HTML ${h.status}, fallback in Middle ${h.fallbackInMiddle}, Inner in HTML ${h.innerInHtml}; ` + c1.brief)
  }
  // C2
  const r = raw.retried
  const retryOk = a.fallbacks[0]?.retry && r.inner && r.fallbacks.length === 0 && /render-ok/.test(r.innerText) && /drv-ok/.test(r.innerText) && /res-ok/.test(r.innerText) && raw.retriedInc === i0 + 1
  let c2brief = !a.fallbacks[0]?.retry ? 'fallback has no Retry button' : r.inner ? `remounted: "${r.innerText.slice(0, 60)}", inner counter ${i0}→${raw.retriedInc}` : `Retry did nothing: ${short(r)}${raw.retryClick.missing ? ' (no Retry button found)' : ''}`
  let againOk = true
  if (raw.retryStillFailing) {
    const s = raw.retryStillFailing
    againOk = !s.inner && s.fallbacks.length === 1 && s.fallbacks[0].retry
    c2brief += `; Retry with the fault still on → ${againOk ? 'falls back again with Retry' : `${short(s)}${s.inner ? ` AND a partial Inner ("${s.innerText.slice(0, 40)}")` : ''}`}`
  }
  const c2 = V(retryOk && againOk, c2brief)
  // C3
  let c3
  if (mode === 'dev') {
    c3 = c3dev(devErrors(srv, lm), where)
    const rtr = (raw.rt || []).filter((x) => new RegExp(`fault:${where}`).test(x.msg))
    if (rtr.length) c3.brief += `; runtime brain-err has it as ${rtr.map((x) => `${x.src || 'no src'} (${x.comp || 'no comp'})`).join(', ')}`
  } else if (f.id === 'ssr') {
    const L = logLines(srv.since(lm)).filter((e) => JSON.stringify(e).includes('component failed during SSR'))
    const e = L.find((x) => /fault:ssr/.test(JSON.stringify(x)))
    c3 = V(e && e.rid, e ? `app.log: "component failed during SSR" rid ${e.rid} comp ${e.comp ?? 'null'} where ${e.where}` : `no "component failed during SSR" entry in logs/app.log (${L.length} SSR entries)`, { entries: L })
  } else if (f.id === 'prerender') {
    const b = srv.build
    c3 = V(/fault:ssr|failed|fallback/i.test(b) && /pre/.test(b), /fault:ssr/.test(b) ? 'brain build reported it' : '`brain build` printed nothing about the component that fell back while /pre was prerendered (exit 0)')
  } else c3 = NA('brain_errors is a dev-server tool; production has no error sink for client errors (§42)')
  raw.c3entries = c3.entries; delete c3.entries
  return { raw, c1, c2, c3, c4: NA('not a handler'), c5: NA('dev compile errors only') }
}

async function handlerFault(page, srv, mode, f) {
  const raw = {}
  const lm = srv.mark()
  if (!(await goto(page, srv.url + f.path))) return { error: 'page did not load' }
  raw.before = await snap(page)
  raw.click = await rc(page, `[data-p="${f.button}"]`)
  await sleep(400)
  raw.after = await snap(page)
  const h0 = num(raw.after.counters['inner-inc'])
  await rc(page, '[data-p="inner-inc"]')
  raw.innerInc = num((await snap(page)).counters['inner-inc'])
  raw.siblings = await siblingsWork(page)
  raw.rt = mode === 'dev' ? await rtErrors(page) : null
  raw.nav = await navWorks(page)
  await sleep(300)
  const a = raw.after
  const c4 = V(a.inner && a.fallbacks.length === 0 && h0 === 1 && raw.innerInc === 2 && a.toasts.length > 0,
    `${a.inner ? 'Inner kept' : 'Inner REPLACED: ' + short(a)}; handler ran (inner counter ${h0}), then counts ${h0}→${raw.innerInc}; toast ${a.toasts.length ? JSON.stringify(a.toasts[0]) : 'NONE'}${a.noise.unhandled.length ? `; unhandled rejection ${JSON.stringify(a.noise.unhandled)}` : ''}`)
  const c1 = V(a.fallbacks.length === 0 && raw.siblings.ok && raw.nav.ok && !a.overlay,
    `fallbacks ${a.fallbacks.length}; siblings ${raw.siblings.ok ? '6/6 count' : 'FAIL ' + JSON.stringify(raw.siblings.each)}; nav ${raw.nav.ok ? 'ok' : 'FAIL'}`)
  let c3
  if (mode === 'dev') {
    c3 = c3dev(devErrors(srv, lm), f.id)
    const rtr = (raw.rt || []).filter((x) => new RegExp(`fault:${f.id}`).test(x.msg))
    if (rtr.length) c3.brief += `; runtime brain-err has it as ${rtr.map((x) => `${x.src || 'no src'} (${x.comp || 'no comp'})`).join(', ')}`
    raw.c3entries = c3.entries; delete c3.entries
  } else c3 = NA('brain_errors is a dev-server tool; production has no error sink for client errors (§42)')
  return { raw, c1, c2: NA('the component never falls back'), c3, c4, c5: NA('dev compile errors only') }
}

async function loadFault(page, srv, mode, f) {
  const raw = {}
  const lm = srv.mark()
  const r = await fetch(srv.url + f.path)
  const html = await r.text()
  raw.http = { status: r.status, errorPage: /Something went wrong/.test(html), rid: (html.match(/Reference:\s*(?:<!---->)?\s*([\w-]+)/) || [])[1] || null }
  await goto(page, srv.url + f.path)
  raw.page = await page.evaluate(`({ h1: document.querySelector('h1')?.textContent, text: document.body.innerText.slice(0, 300), nav: !!document.querySelector('nav a[href="/about"]') })`)
  raw.nav = await navWorks(page, 'nav a[href="/about"]')
  await sleep(300)
  const shown = /Something went wrong/.test(raw.page.h1 || '')
  // A streamed route (#232) has sent 200 and its headers before load() fails, so it cannot answer 500
  // (runtime/server/server.mjs `streamed`). There the status criterion is the logged "load failed (streamed)"
  // entry whose request id is the one the page shows; a non-streamed route must still answer 500.
  const pageRid = (raw.page.text.match(/Reference:\s*([\w-]+)/) || [])[1] || null
  const streamedOk = mode === 'prod' && r.status === 200 && logLines(srv.since(lm)).some((e) => e.msg === 'load failed (streamed)' && e.rid === pageRid)
  const statusOk = mode === 'dev' || r.status === 500 || streamedOk
  const c1 = V(shown && raw.page.nav && raw.nav.ok && statusOk,
    `HTTP ${r.status}${streamedOk ? ' (streamed route: headers sent before load(); logged as "load failed (streamed)" with the page reference ' + pageRid + ')' : ''}; view shows ${JSON.stringify(raw.page.h1)} (want the _error.brain page); layout nav ${raw.page.nav ? 'kept' : 'MISSING'}, navigation ${raw.nav.ok ? 'ok' : 'FAIL'}`)
  let c3
  if (mode === 'dev') {
    const tail = srv.since(lm)
    const hit = tail.split('\n').filter((l) => /fault:load|load failed|load\b.*fail/i.test(l))
    c3 = V(hit.length > 0 && /views\/ssr\.brain/.test(hit.join('\n')), hit.length ? `dev log: ${hit[0].trim().slice(0, 160)}` : 'nothing about the failing load() in the dev log or brain_errors')
    raw.devLog = hit
  } else {
    const L = logLines(srv.since(lm)).filter((e) => /load failed/.test(JSON.stringify(e)))
    const e = L[0]
    c3 = V(e && e.rid && /fault:load/.test(JSON.stringify(e)), e ? `app.log: "load failed" rid ${e.rid} status ${e.status}${raw.http.rid ? `, page reference ${raw.http.rid}` : ''}` : 'no "load failed" entry in logs/app.log')
    raw.log = L
  }
  return { raw, c1, c2: NA('a failing load() renders _error.brain (§22), no component Retry'), c3, c4: NA('not a handler'), c5: NA('dev compile errors only') }
}

// C5: invalid code in comp/inner.brain while `brain run` is up, then fixed.
const BROKEN = INNER_SRC.replace('let hits = sig(0)', 'let hits = sig(0)\n  hits.push(1)')
async function compileFault(page, srv) {
  const raw = {}
  writeFileSync(INNER, BROKEN)
  raw.check = (() => { const r = brain(['check', '--json', 'comp/inner.brain']); try { return JSON.parse(r.out).map((d) => `${d.code} ${d.file}:${d.line} ${d.msg}`) } catch { return r.out.slice(0, 300) } })()
  writeFileSync(INNER, INNER_SRC)
  await sleep(500)
  const lm = srv.mark()
  // (a) the file breaks while the page is open (HMR)
  await goto(page, srv.url + '/lab')
  await page.evaluate('window.__mark = 1')
  writeFileSync(INNER, BROKEN)
  await waitFor(page, `!!document.getElementById('__brain_overlay') || !!document.querySelector('[data-brain-error]') || !document.querySelector('[data-p=inner]')`, 5000)
  await sleep(500)
  raw.hmr = await snap(page)
  raw.hmrSiblings = await siblingsWork(page)
  raw.hmrMark = await page.evaluate('window.__mark === 1')
  // (b) a fresh load while the file is broken
  await goto(page, srv.url + '/lab')
  raw.load = await snap(page)
  raw.loadSiblings = await siblingsWork(page)
  await page.evaluate('window.__mark = 2')
  // (c) the file is fixed
  writeFileSync(INNER, INNER_SRC)
  const recovered = await waitFor(page, `!!document.querySelector('[data-p=inner]') && !document.getElementById('__brain_overlay')?.isConnected`, 8000)
  await sleep(400)
  raw.fixed = await snap(page)
  raw.fixedMark = await page.evaluate('window.__mark').catch(() => null)
  raw.log = srv.since(lm).split('\n').filter((l) => /\[hmr\]|error/i.test(l)).map((l) => l.slice(0, 300))
  const h = raw.hmr, l = raw.load, x = raw.fixed
  const boxH = h.fallbacks.find((f) => f.errorBox) || null, boxL = l.fallbacks.find((f) => f.errorBox) || null
  const hmrOk = !h.overlay && !!boxH && boxH.inMiddle && raw.hmrSiblings.ok
  const loadOk = !l.overlay && !!boxL && boxL.inMiddle && /\bB\d{4}\b/.test(boxL.text) && /fix/.test(boxL.text) && raw.loadSiblings.ok
  const fixOk = recovered && x.inner && x.fallbacks.length === 0 && !x.overlay
  const sib = (s) => (s.ok ? '6/6 siblings count' : `siblings blocked: ${Object.entries(s.each).filter(([, v]) => !v.ok).map(([k, v]) => `${k} (top: ${v.top})`).join(', ')}`)
  const c5 = V(hmrOk && loadOk && fixOk, [
    `brain check: ${Array.isArray(raw.check) ? raw.check[0] : raw.check}`,
    `on save: ${h.overlay ? 'FULL-PAGE overlay #__brain_overlay ("' + (h.overlayText || '').split('\n')[0].slice(0, 60) + '")' : boxH ? 'error box in Inner\'s place' : 'no notice'}, Inner ${h.inner ? 'still shows old code' : 'gone'}, ${sib(raw.hmrSiblings)}`,
    `fresh load while broken: ${boxL ? `box in ${boxL.inMiddle ? 'Inner\'s place' : 'wrong place'} ("${boxL.text.slice(0, 70)}")` : l.overlay ? 'full-page overlay' : 'no box: ' + short(l)}, ${sib(raw.loadSiblings)}`,
    `after the fix: ${fixOk ? `Inner back via ${raw.fixedMark === 2 ? 'HMR (no reload)' : 'a full page reload'}` : 'NOT recovered: ' + short(x) + (x.overlay ? ' overlay still up' : '')}`,
  ].join('; '))
  restoreInner()
  return { raw, c1: NA('see C5'), c2: NA('see C5'), c3: NA('see C5'), c4: NA('see C5'), c5 }
}

async function probeMode(chrome, mode, srv) {
  const page = await openPage(chrome.port, 'about:blank')
  const logs = []
  page.on((m) => {
    if (m.method === 'Runtime.consoleAPICalled' && /error|warn/.test(m.params.type)) logs.push(`[${m.params.type}] ` + m.params.args.map((a) => a.value ?? a.description ?? '').join(' ').slice(0, 300))
    if (m.method === 'Runtime.exceptionThrown') logs.push('[exception] ' + String(m.params.exceptionDetails.exception?.description || m.params.exceptionDetails.text).slice(0, 300))
  })
  await page.send('Page.addScriptToEvaluateOnNewDocument', { source: INIT })
  await page.send('Emulation.setDeviceMetricsOverride', { width: 1200, height: 900, deviceScaleFactor: 1, mobile: false })
  const res = {}
  try {
    for (const f of FAULTS) {
      if (f.modes && !f.modes.includes(mode)) continue
      if (BLOCKED[f.id]) {
        res[f.id] = Object.fromEntries(['c1', 'c2', 'c3', 'c4', 'c5'].map((k) => [k, { pass: 'blocked', brief: 'BLOCKED — ' + BLOCKED[f.id] }]))
        process.stdout.write(`  ${mode} ${f.id.padEnd(9)} BLOCKED\n`)
        continue
      }
      const n0 = logs.length, s0 = Date.now()
      let r
      try {
        r = f.id === 'compile' ? await compileFault(page, srv)
          : f.id === 'load' ? await loadFault(page, srv, mode, f)
            : f.button ? await handlerFault(page, srv, mode, f)
              : await boundaryFault(page, srv, mode, f)
      } catch (e) { r = { error: String(e.stack || e) } }
      if (r.error) for (const k of ['c1', 'c2', 'c3', 'c4', 'c5']) r[k] = V(false, 'bench error: ' + r.error.split('\n')[0])
      r.console = logs.slice(n0)
      // where the error did surface: console.error frames in inner.brain (dev serves inner.brain as JS: these are
      // generated JS lines, not .brain lines) and what brain-err recorded
      if (mode === 'dev' && r.c3 && r.c3.pass === false) {
        const w = f.where || f.id
        const fr = [...new Set(r.console.filter((l) => l.includes('fault:' + w)).flatMap((l) => [...l.matchAll(/comp\/inner\.brain:(\d+)/g)].map((m) => m[1])))]
        r.c3.brief += `; console.error only${fr.length ? ` (frames inner.brain:${fr.join(',')} = generated JS lines; the .brain lines are ${THROW_LINE}/${callLine(w)})` : ''}; brain-err recorded ${Array.isArray(r.raw?.rt) ? r.raw.rt.length : 'n/a'}`
      }
      r.ms = Date.now() - s0
      res[f.id] = r
      process.stdout.write(`  ${mode} ${f.id.padEnd(9)} ${['c1', 'c2', 'c3', 'c4', 'c5'].map((k) => (r[k].pass === null ? '-' : r[k].pass ? 'P' : 'F')).join('')}\n`)
    }
  } finally { page.close() }
  return res
}

// ---------- main ----------
mkdirSync(join(OUT, 'raw'), { recursive: true })
const chrome = await launch()
children.add({ kill: () => chrome.close() })
const modes = {}
let buildMs = null, buildOut = null
for (const mode of MODES) {
  const started = Date.now()
  let srv
  if (mode === 'dev') srv = await devServer(DEV_PORT)
  else if (mode === 'prod') { srv = await prodServer(); buildMs = srv.buildMs; buildOut = srv.build }
  else fatal(`unknown mode ${mode} (dev, prod)`)
  let res
  try { res = await probeMode(chrome, mode, srv) } finally { restoreInner(); await srv.stop() }
  modes[mode] = { url: srv.url, ms: Date.now() - started, res }
  writeFileSync(join(OUT, 'raw', `${mode}.json`), home(JSON.stringify({ mode, url: srv.url, serverLog: mode === 'dev' ? srv.log() : srv.log(), res }, null, 1)))
}
chrome.close()
rmSync(APP, { recursive: true, force: true }) // nothing is left behind but the binary copy

// ---------- report ----------
const CRIT = { c1: 'isolation', c2: 'retry', c3: 'reported', c4: 'handler keeps component', c5: 'dev compile error' }
const OWNER = {
  c1: (f) => (f === 'ssr' ? 'BUILD runtime/server/server.mjs SSR + RUNTIME runtime/brain-guard.js' : f === 'prerender' ? 'BUILD crates/brain-pack/src/prerender.rs + RUNTIME runtime/brain-guard.js' : f === 'load' ? 'BUILD runtime/server/server.mjs (_error.brain)' : 'RUNTIME runtime/brain-guard.js · COMPILER guard wrapping in codegen'),
  c2: () => 'RUNTIME runtime/brain-guard.js (Retry/remount)',
  c3: (f, m) => (m === 'dev' ? 'TOOLCHAIN brain_errors (crates/brain-serve client.js/server.rs) + RUNTIME runtime/brain-err.js' : f === 'prerender' ? 'BUILD crates/brain-pack/src/prerender.rs (report fallbacks)' : 'BUILD runtime/server/server.mjs log'),
  c4: () => 'RUNTIME runtime/brain-guard.js (handler path)',
  c5: () => 'TOOLCHAIN dev overlay (crates/brain-serve/src/client.js, server.rs)',
}
const rows = []
for (const f of FAULTS) for (const mode of MODES) {
  const r = modes[mode].res[f.id]
  if (!r) continue
  rows.push({ fault: f.id, title: f.title, mode, ...Object.fromEntries(Object.keys(CRIT).map((k) => [k, { pass: r[k].pass, brief: r[k].brief }])) })
}
const failures = []
for (const r of rows) for (const k of Object.keys(CRIT)) if (r[k].pass === false) failures.push({ fault: r.fault, mode: r.mode, criterion: `${k.toUpperCase()} ${CRIT[k]}`, evidence: r[k].brief, owner: OWNER[k](r.fault, r.mode) })
const pass = failures.length === 0
const totalMs = Date.now() - t0
const summary = {
  bench: 'faults', task: 176, date: new Date().toISOString(), brainSha256: brainSha, brainSource: home(SRC_BIN), work: home(WORK), node: process.version,
  modes: MODES, pass, failures: failures.length, blocked: BLOCKED,
  runtimeMs: { total: totalMs, build: buildMs, ...Object.fromEntries(MODES.map((m) => [m, modes[m].ms])) },
  innerLines: { throw: THROW_LINE, ...Object.fromEntries(['render', 'fx', 'drv', 'resource', 'click', 'async', 'ssr'].map((w) => [w, callLine(w)])) },
  rows, failureList: failures,
  buildOutputTail: buildOut ? home(buildOut.split('\n').slice(-6).join('\n')) : null,
}
writeFileSync(join(OUT, 'summary.json'), home(JSON.stringify(summary, null, 1)) + '\n')
const cell = (c) => (c.pass === null ? 'n/a' : c.pass === 'blocked' ? 'BLOCKED' : c.pass ? 'PASS' : '**FAIL**')
const esc = (s) => String(s).replace(/\|/g, '\\|').replace(/\n/g, ' ')
const md = [
  '# Fault injection bench — results',
  '',
  `Binary sha256 \`${brainSha}\` · ${summary.date} · total ${(totalMs / 1000).toFixed(1)} s (build ${buildMs == null ? 'skipped' : (buildMs / 1000).toFixed(1) + ' s'}) · ${failures.length} failing cell(s) · verdict **${pass ? 'PASS' : 'FAIL'}**`,
  '',
  'C1 isolation · C2 retry · C3 reported (dev: brain_errors with `comp/inner.brain:<line>`; prod: logs/app.log with rid) · C4 a failing handler keeps the component + toast · C5 dev compile error isolated and recovered by HMR.',
  '',
  '| Fault | Mode | C1 | C2 | C3 | C4 | C5 |',
  '|---|---|---|---|---|---|---|',
  ...rows.map((r) => `| ${r.title} | ${r.mode} | ${cell(r.c1)} | ${cell(r.c2)} | ${cell(r.c3)} | ${cell(r.c4)} | ${cell(r.c5)} |`),
  '',
  '## Blocked',
  '',
  Object.keys(BLOCKED).length ? Object.entries(BLOCKED).map(([k, v]) => `- ${k}: ${esc(v)}`).join('\n') : 'None.',
  '',
  '## Failures',
  '',
  failures.length ? '| Fault | Mode | Criterion | Evidence | Owner |\n|---|---|---|---|---|\n' + failures.map((x) => `| ${x.fault} | ${x.mode} | ${x.criterion} | ${esc(x.evidence)} | ${x.owner} |`).join('\n') : 'None.',
  '',
  '## Evidence per cell',
  '',
  '| Fault | Mode | Criterion | Evidence |',
  '|---|---|---|---|',
  ...rows.flatMap((r) => Object.keys(CRIT).filter((k) => r[k].pass !== null && r[k].pass !== 'blocked').map((k) => `| ${r.fault} | ${r.mode} | ${k.toUpperCase()} | ${esc(r[k].brief)} |`)),
  '',
  `Inner lines (comp/inner.brain): throw ${THROW_LINE}; call sites ${Object.entries(summary.innerLines).filter(([k]) => k !== 'throw').map(([k, v]) => `${k} ${v}`).join(', ')}.`,
  'Raw per-mode data (every snapshot, click target, console line and the full server log): `results/raw/<mode>.json` (not committed).',
  '',
].join('\n')
writeFileSync(join(OUT, 'summary.md'), home(md))

console.log(`faults bench · brain ${brainSha} · ${(totalMs / 1000).toFixed(1)} s`)
for (const r of rows) console.log(`${r.fault.padEnd(10)}${r.mode.padEnd(6)}${Object.keys(CRIT).map((k) => (r[k].pass === null ? 'n/a ' : r[k].pass === 'blocked' ? 'BLKD' : r[k].pass ? 'PASS' : 'FAIL').padEnd(6)).join('')}`)
for (const x of failures) console.log(`FAIL ${x.fault}/${x.mode} ${x.criterion}: ${x.evidence} [${x.owner}]`)
console.log(`verdict ${pass ? 'PASS' : 'FAIL'} → bench/faults/results/summary.md`)
process.exit(pass ? 0 : 1)
