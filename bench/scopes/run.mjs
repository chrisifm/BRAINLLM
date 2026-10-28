// Component encapsulation bench (QA, task #197): 12 rules that a component must keep in light DOM, checked in real
// headless Chrome against the app in bench/scopes/app, once in development (`brain run`) and once in production
// (`brain build` + a static host for dist/). See README.md for what each rule means and how it is measured.
//
// Usage: node bench/scopes/run.mjs [--modes dev,prod] [--port 5196] [--no-build]
// Env:   BRAIN (binary, default target/debug/brain) · BRAIN_CHROME (Chrome binary, see bench/lib/cdp.mjs)
// Output: bench/scopes/results/summary.json + summary.md (committed) · results/raw/<mode>.json (gitignored)
// Exit:  0 all rules pass in every mode · 1 any rule fails (regression gate) · 2 the bench itself could not run
import { spawn, execFileSync } from 'node:child_process'
import { createServer } from 'node:http'
import { createHash } from 'node:crypto'
import { existsSync, mkdirSync, readFileSync, statSync, writeFileSync } from 'node:fs'
import { extname, join, normalize } from 'node:path'
import { launch, openPage } from '../lib/cdp.mjs'

const HERE = new URL('.', import.meta.url).pathname
const ROOT = new URL('../..', import.meta.url).pathname
const APP = join(HERE, 'app')
const OUT = join(HERE, 'results')
const BRAIN = process.env.BRAIN ?? join(ROOT, 'target/debug/brain')
const argv = process.argv.slice(2)
const arg = (k, d) => { const i = argv.indexOf(`--${k}`); return i >= 0 ? argv[i + 1] : d }
const MODES = arg('modes', 'dev,prod').split(',')
const DEV_PORT = Number(arg('port', 5196))
const NO_BUILD = argv.includes('--no-build')
const sleep = (ms) => new Promise((r) => setTimeout(r, ms))
const t0 = Date.now()

// ---------- children are killed by their own PID only ----------
const children = new Set()
const cleanup = () => { for (const c of children) { try { c.kill('SIGTERM') } catch {} } children.clear() }
process.on('exit', cleanup)
process.on('SIGINT', () => (cleanup(), process.exit(130)))
process.on('SIGTERM', () => (cleanup(), process.exit(143)))

function fatal(msg) { console.error('scopes bench: ' + msg); cleanup(); process.exit(2) }
if (!existsSync(BRAIN)) fatal(`no binary at ${BRAIN}; run \`cargo build -p brain-cli\` first`)
const brainSha = createHash('sha256').update(readFileSync(BRAIN)).digest('hex').slice(0, 12)

// ---------- servers ----------
async function devServer(port) {
  const c = spawn(BRAIN, ['run', '--port', String(port)], { cwd: APP, stdio: ['ignore', 'pipe', 'pipe'] })
  children.add(c)
  let log = ''
  c.stdout.on('data', (d) => (log += d)); c.stderr.on('data', (d) => (log += d))
  const url = `http://127.0.0.1:${port}`
  let up = false
  for (let i = 0; i < 150 && !up; i++) { try { up = (await fetch(url + '/scopes')).ok } catch { await sleep(100) } }
  if (!up) { c.kill('SIGTERM'); children.delete(c); fatal(`brain run did not answer on ${url}:\n${log}`) }
  return { url, log: () => log, stop: () => { c.kill('SIGTERM'); children.delete(c) } }
}

const MIME = {
  '.html': 'text/html; charset=utf-8', '.js': 'text/javascript; charset=utf-8', '.mjs': 'text/javascript; charset=utf-8',
  '.css': 'text/css; charset=utf-8', '.json': 'application/json', '.txt': 'text/plain; charset=utf-8',
  '.svg': 'image/svg+xml', '.png': 'image/png', '.webmanifest': 'application/manifest+json', '.md': 'text/markdown; charset=utf-8',
}
// The app has no server functions, so `brain build` emits a static dist/: serve it like a static host (clean URLs).
function staticServer(dir) {
  const isFile = (p) => { try { return statSync(p).isFile() ? p : null } catch { return null } }
  const srv = createServer((req, res) => {
    const u = new URL(req.url, 'http://x')
    const p = normalize(join(dir, decodeURIComponent(u.pathname)))
    if (!p.startsWith(dir) || u.pathname.startsWith('/_brain/')) return res.writeHead(404).end()
    const hit = isFile(p) || isFile(join(p, 'index.html')) || isFile(p + '.html')
    if (!hit) return res.writeHead(404, { 'content-type': 'text/html; charset=utf-8' }).end(readFileSync(join(dir, '404.html')))
    res.writeHead(200, { 'content-type': MIME[extname(hit)] || 'application/octet-stream', 'cache-control': 'no-cache' })
    res.end(readFileSync(hit))
  })
  return new Promise((r) => srv.listen(0, '127.0.0.1', () => r({ url: `http://127.0.0.1:${srv.address().port}`, stop: () => srv.close() })))
}

// ---------- in-page probes ----------
// Helpers injected before any app script runs.
const INIT = `
  window.__keys0 = Object.getOwnPropertyNames(window);
  window.__wait = (ms = 60) => new Promise((r) => setTimeout(r, ms));
  window.__click = async (el) => { if (!el) { (window.__missing ||= []).push(new Error().stack.split('\\n')[2]); return; } el.click(); await window.__wait(); };
  window.__label = (n) => {
    const el = n.nodeType === 1 ? n : n.parentElement;
    const id = el.closest('[data-probe],[data-id],[data-where],[data-leaf],[id]');
    const tag = id ? (id.dataset.probe || (id.dataset.id && 'row-' + id.dataset.id) || id.dataset.where || id.dataset.leaf || '#' + id.id) : '?';
    return tag + '>' + el.nodeName.toLowerCase() + '.' + (el.className || '') + (n.nodeType === 3 ? '#text' : '');
  };
  window.__observe = () => {
    const recs = [];
    const mo = new MutationObserver((rs) => recs.push(...rs));
    mo.observe(document.body, { subtree: true, childList: true, characterData: true, attributes: true });
    return async () => {
      await window.__wait();
      recs.push(...mo.takeRecords()); mo.disconnect();
      const nodes = new Set(); const types = {};
      for (const r of recs) { const k = r.type + (r.attributeName ? ':' + r.attributeName : ''); types[k] = (types[k] || 0) + 1; }
      for (const r of recs) { nodes.add(r.target); for (const n of r.addedNodes) nodes.add(n); for (const n of r.removedNodes) nodes.add(n); }
      return { records: recs.length, nodes: nodes.size, types, where: [...nodes].map(window.__label) };
    };
  };
`

// Everything on /scopes, in this order (some steps depend on earlier clicks, e.g. rule 9 reads #c3 after rule 1).
const PAGE = `(async () => {
  const $ = (s, r = document) => r.querySelector(s);
  const $$ = (s, r = document) => [...r.querySelectorAll(s)];
  const txt = (s) => $(s)?.textContent.trim();
  const out = {};

  // 11 (part 1): globals before any interaction
  const appNames = ['count', 'items', 'toggle', 'handleClick', 'label', 'shared', 'rows', 'bump', 'Tone', 'tone', 't', 'shift', 'broken', 'clicks', 'val', 'explode', 'make', 'aHits', 'bHits', 'onA', 'onB', 'viewHits', 'onMid', 'parsed', 'tags', 'show'];
  out.globals = { newKeys: Object.getOwnPropertyNames(window).filter((k) => !window.__keys0.includes(k) && !k.startsWith('__')), leakedAppNames: appNames.filter((n) => n in window) };

  // 1 local state
  for (let i = 0; i < 3; i++) await window.__click($('#c1 button'));
  out.r1 = ['#c1', '#c2', '#c3'].map((s) => txt(s + ' button'));

  // 4a precise update: one counter click
  let stop = window.__observe();
  await window.__click($('#c2 button'));
  out.r4counter = await stop();

  // 2 context
  const tones = () => Object.fromEntries($$('#r2 .tone').map((e, i) => [e.closest('[id^=ctx]').id + ':' + e.dataset.where + ':' + i, e.textContent.trim()]));
  out.r2before = tones();
  stop = window.__observe();
  await window.__click($('#ctx-red .shift'));
  out.r2after = tones();
  out.r2mut = await stop();

  // 3 store
  await window.__click($('#far-a .inc'));
  await window.__click($('#far-b .inc'));
  out.r3 = { a: txt('#far-a .val'), b: txt('#far-b .val'), other: txt('#far-other .val'), jsStore: $$('#r11 .store').map((e) => e.textContent.trim()) };
  stop = window.__observe();
  await window.__click($('#far-other .inc'));
  out.r3otherMut = await stop();
  out.r3otherVal = txt('#far-other .val');

  // 4b keyed list of 200 rows
  out.r4rows = $$('#r4 li').length;
  stop = window.__observe();
  await window.__click($('#r4 .bump'));
  out.r4list = await stop();
  out.r4row57 = txt('#r4 li[data-id="57"] .hits');

  // 5/8/10 CSS
  out.css = Object.fromEntries($$('#r5 [data-probe]').map((e) => {
    const c = getComputedStyle(e);
    return [e.dataset.probe, { color: c.color, fontSize: c.fontSize, borderTop: c.borderTopWidth, deco: c.textDecorationLine }];
  }));
  out.cssB = $$('#r5 [data-probe="b-root"]').map((r) => ({ in: r.parentElement.closest('[data-b]')?.dataset.probe || 'view', h2: getComputedStyle($('[data-probe="b-h2"]', r)).color, p: getComputedStyle($('[data-probe="b-p"]', r)).color }));
  const viewRoot = $('main[data-b]');
  out.viewSpans = $$('main span').filter((e) => e.closest('[data-b]') !== viewRoot).map((e) => getComputedStyle(e).borderTopWidth);
  out.viewOwnSpans = $$('main span').filter((e) => e.closest('[data-b]') === viewRoot).map((e) => getComputedStyle(e).borderTopWidth);
  out.escape = { body: getComputedStyle(document.body).fontSize, main: getComputedStyle($('main')).fontSize, selection: getComputedStyle($('[data-probe="v-p"]'), '::selection').color };
  out.slotText = { aIntoB: txt('[data-probe="a-slot-into-b"]'), aIntoHost: txt('[data-probe="a-slot-into-host"]'), hostOwn: txt('[data-probe="host-own"]') };

  // 6 ids
  const ids = $$('[id]').map((e) => e.id);
  out.r6 = {
    dupIds: ids.filter((v, i) => ids.indexOf(v) !== i),
    fields: ['#lf1', '#lf2'].map((s) => {
      const l = $(s + ' label'), inp = $(s + ' input'), a = $(s + ' a'), hint = $(s + ' small');
      return { inputId: inp.id, labelFor: l.htmlFor, controlIsOwn: l.control === inp, describedby: inp.getAttribute('aria-describedby'), hintId: hint.id, href: a.getAttribute('href') };
    }),
    viewIds: ['r1', 'c1', 'anchor-target', 'lf1'].map((i) => !!document.getElementById(i)),
  };

  // 7 events
  let domPings = 0;
  document.addEventListener('ping', () => domPings++, true);
  const ev = () => ({ a: txt('#r7 .a-hits'), b: txt('#r7 .b-hits'), view: txt('#r7 .view-hits'), sib: txt('#r7 .sib-hits'), last: txt('#r7 .last-ping') });
  await window.__click($('#r7 [data-leaf="a"]'));
  const afterA = ev();
  await window.__click($('#r7 [data-leaf="b"]'));
  const afterB = ev();
  await window.__click($('#r7 [data-leaf="sib"]'));
  out.r7 = { afterA, afterB, afterSib: ev(), domPings };

  // 9 errors: a handler error keeps the component; a drv error swaps only that component for its fallback
  await window.__click($('#boom1 .explode'));
  const afterHandler = { boom1: $('#boom1').textContent.trim().slice(0, 60), boom1Buttons: $$('#boom1 button').length, boom1Clicks: txt('#boom1 .clicks') ?? null, boom2: txt('#boom2 .val') };
  await window.__click($('#after-boom button'));
  await window.__click($('#boom2 .break'));
  const afterDrv = { boom2: $('#boom2').textContent.trim().slice(0, 60), mainKids: $('main')?.children.length ?? 'main gone' };
  await window.__click($('#after-boom button'));
  await window.__click($('#c3 button'));
  out.r9 = { afterHandler, afterDrv, afterCounter: txt('#after-boom button'), c3: txt('#c3 button') };

  // 11 same names
  await window.__click($('#js-a1 .go'));
  await window.__click($('#js-b1 .go'));
  out.r11 = ['#js-a1', '#js-a2', '#js-b1'].map((s) => ({ count: txt(s + ' .count'), items: txt(s + ' .items'), store: txt(s + ' .store') }));
  out.globalsAfter = { leakedAppNames: appNames.filter((n) => n in window), newKeys: Object.getOwnPropertyNames(window).filter((k) => !window.__keys0.includes(k) && !k.startsWith('__')) };

  // 12 custom property per instance
  out.r12 = { plain: getComputedStyle($('#tint-plain .swatch')).color, custom: getComputedStyle($('#tint-custom .swatch')).color, viewTitle: getComputedStyle($('[data-probe="v-h2"]')).color, rootAccent: getComputedStyle(document.documentElement).getPropertyValue('--accent').trim() };
  return out;
})()`

// /errors: a component that throws while it is created, between two healthy counters.
const ERRORS = `(async () => {
  const $ = (s) => document.querySelector(s);
  if (!$('#bad')) return { bad: 'view not rendered: ' + document.body.innerText.slice(0, 200) };
  await window.__click($('#before button'));
  await window.__click($('#after button'));
  return { before: $('#before button')?.textContent, bad: $('#bad').textContent.trim(), after: $('#after button')?.textContent };
})()`

async function waitFor(page, expr, ms = 8000) {
  for (let t = 0; t < ms; t += 50) { if (await page.evaluate(expr).catch(() => false)) return true; await sleep(50) }
  return false
}

async function probe(chromePort, base, prod) {
  const url = (p) => base + p + (prod ? '/' : '')
  const page = await openPage(chromePort, 'about:blank')
  const logs = []
  page.on((m) => {
    if (m.method === 'Runtime.consoleAPICalled' && m.params.type !== 'log') logs.push(`[${m.params.type}] ` + m.params.args.map((a) => a.value ?? a.description ?? '').join(' '))
    if (m.method === 'Runtime.exceptionThrown') logs.push('[exception] ' + (m.params.exceptionDetails.exception?.description || m.params.exceptionDetails.text))
  })
  await page.send('Page.addScriptToEvaluateOnNewDocument', { source: INIT })
  try {
    await page.send('Page.navigate', { url: url('/scopes') })
    if (!(await waitFor(page, `!!document.querySelector('#tint-custom .swatch') && document.readyState === 'complete'`))) throw new Error('/scopes did not render')
    await sleep(1000)
    const m = await page.evaluate(PAGE)
    m.missing = await page.evaluate('window.__missing || []')
    await page.send('Page.navigate', { url: url('/errors') })
    if (!(await waitFor(page, `!!document.querySelector('#after button') && document.readyState === 'complete'`))) throw new Error('/errors did not render')
    await sleep(800)
    const e = await page.evaluate(ERRORS)
    return { m, e, logs }
  } finally { page.close() }
}

// ---------- verdicts ----------
const ACC = 'rgb(0, 98, 196)', OK = 'rgb(22, 121, 74)', BAD = 'rgb(196, 43, 28)', MUTED = 'rgb(85, 89, 94)'
const onlyIn = (mut, prefix) => mut.where.every((w) => w.startsWith(prefix))
// [color, font-size, border-top] each probe must compute (null = not asserted). v = view, a = CardA, b = CardB (ui/),
// m = MultiRoot (several roots, {#if} and {#each} roots), host = SlotHost.
const SAME_NAME = {
  'v-h2': [ACC, '31px', '7px'], 'v-p': [ACC, '31px', '7px'], 'v-btn': [ACC, '31px'], 'v-em': [null, '31px', '0px'],
  'a-h2': [OK, '17px', '3px'], 'a-p': [OK, '17px', '3px'], 'a-btn': [OK, '17px'], 'a-plain-span': [null, null, '0px'],
  'b-h2': [BAD, '23px', '5px'], 'b-p': [BAD, '23px', '5px'], 'b-btn': [BAD, '23px'],
  'm-root1': [MUTED, '11px'], 'm-root2': [MUTED, '11px'], 'm-cond': [MUTED, '11px'], 'm-each-x': [MUTED, '11px'], 'm-each-y': [MUTED, '11px'],
  'host-own': [null, null, '0px'],
}
const ESCAPE = { 'v-after-escape': [ACC, '31px', '7px'], 'esc-own': [null, null, '0px'] }
function cssCheck(css, table) {
  const fails = []
  for (const [k, [c, f, b]] of Object.entries(table)) {
    const g = css[k]
    if (!g) { fails.push(k + ': missing'); continue }
    if (c && g.color !== c) fails.push(`${k}: color ${g.color} (want ${c})`)
    if (f && g.fontSize !== f) fails.push(`${k}: font-size ${g.fontSize} (want ${f})`)
    if (b && g.borderTop !== b) fails.push(`${k}: border-top ${g.borderTop} (want ${b})`)
  }
  return fails
}

function verdicts({ m, e }) {
  const css = m.css
  const sameFails = cssCheck(css, SAME_NAME)
  if (m.viewSpans.includes('7px')) sameFails.push(`view rule \`span { border-top: 7px }\` reached ${m.viewSpans.filter((x) => x === '7px').length} component spans`)
  if (m.cssB.some((b) => b.h2 !== BAD || b.p !== BAD)) sameFails.push('two CardB instances differ: ' + JSON.stringify(m.cssB))
  const escFails = cssCheck(css, ESCAPE)
  if (m.escape.body === '41px') escFails.push('`@media { body { font-size: 41px } }` reached <body>')
  if (m.escape.main === '41px') escFails.push('`main { font-size: 41px }` reached the view <main>')
  if (m.escape.selection === BAD) escFails.push('`::selection` reached a view paragraph')
  const slotFails = []
  const s = (k) => css[k] || {}
  if (s('a-slot-into-b').color !== OK) slotFails.push(`CardA content slotted into CardB: color ${s('a-slot-into-b').color} font ${s('a-slot-into-b').fontSize} border ${s('a-slot-into-b').borderTop} (want CardA: ${OK} 17px 3px)`)
  if (s('a-slot-into-host').color !== OK || s('a-slot-into-host').deco !== 'underline') slotFails.push(`CardA content slotted into SlotHost: color ${s('a-slot-into-host').color} font ${s('a-slot-into-host').fontSize} deco ${s('a-slot-into-host').deco} (want CardA: ${OK} 17px underline)`)
  if (s('v-slot-into-b').color !== ACC) slotFails.push(`view content slotted into CardB: color ${s('v-slot-into-b').color} font ${s('v-slot-into-b').fontSize} (want view: ${ACC} 31px)`)
  if (m.slotText.aIntoB !== 'parent-state' || m.slotText.aIntoHost !== 'parent-state' || m.slotText.hostOwn !== 'child-state') slotFails.push('slot state: ' + JSON.stringify(m.slotText))
  const f = m.r6.fields
  const b2 = m.r2before, a2 = m.r2after
  const ok2 = b2['ctx-red:own-markup:0'] === 'red' && b2['ctx-red:slot-red:1'] === 'red' && b2['ctx-red:own-markup:2'] === 'blue-inner' && b2['ctx-red:slot-inner:3'] === 'blue-inner' && b2['ctx-green:slot-green:5'] === 'green' && b2['ctx-none:outside:6'] === 'none' && a2['ctx-red:slot-red:1'] === 'red+' && a2['ctx-red:slot-inner:3'] === 'blue-inner' && a2['ctx-green:slot-green:5'] === 'green'
  const r9 = m.r9
  return [
    { id: 1, rule: 'Local state per instance', pass: m.r1.join('|') === 'One: 3 × 2 = 6|Two: 0 × 2 = 0|Three: 0 × 2 = 0',
      brief: m.r1.join(' · '), evidence: m.r1 },
    { id: 2, rule: 'Context per subtree', pass: ok2,
      brief: `after shift: red→${a2['ctx-red:slot-red:1']}, inner ${a2['ctx-red:slot-inner:3']}, green ${a2['ctx-green:slot-green:5']}, outside ${b2['ctx-none:outside:6']}`, evidence: { before: b2, after: a2, mutated: m.r2mut } },
    { id: 3, rule: 'Global store shared, field-precise', pass: m.r3.a === '101' && m.r3.b === '101' && m.r3otherVal === '1' && onlyIn(m.r3otherMut, '#far-other'),
      brief: `A=${m.r3.a} B=${m.r3.b} other=${m.r3otherVal}; other write touched ${m.r3otherMut.nodes} node(s) in ${[...new Set(m.r3otherMut.where.map((w) => w.split('>')[0]))].join(',')}`, evidence: { ...m.r3, otherWriteMutations: m.r3otherMut } },
    { id: 4, rule: 'Precise updates', pass: m.r4counter.nodes <= 3 && onlyIn(m.r4counter, '#c2') && m.r4rows === 200 && m.r4row57 === '1' && onlyIn(m.r4list, 'row-57'),
      brief: `counter click: ${m.r4counter.nodes} node(s); 200-row bump: ${m.r4list.nodes} node(s) in ${[...new Set(m.r4list.where.map((w) => w.split('>')[0]))].join(',')}`, evidence: { counter: m.r4counter, rows: m.r4rows, list: m.r4list } },
    { id: 5, rule: 'CSS same-name isolation', pass: sameFails.length === 0,
      brief: sameFails.length ? sameFails.slice(0, 2).join('; ') : `${Object.keys(SAME_NAME).length} probes + ${m.viewSpans.length} component spans as expected`, evidence: { fails: sameFails, componentSpanBorders: [...new Set(m.viewSpans)], viewSpanBorders: [...new Set(m.viewOwnSpans)], twoCardB: m.cssB } },
    { id: 6, rule: 'IDs per instance', pass: m.r6.dupIds.length === 0 && f[0].inputId !== f[1].inputId && f.every((x) => x.labelFor === x.inputId && x.controlIsOwn && x.describedby === x.hintId && x.href === '#' + x.inputId) && m.r6.viewIds.every(Boolean),
      brief: `inputs ${f.map((x) => x.inputId).join(' / ')}; dup ids ${m.r6.dupIds.length}`, evidence: m.r6 },
    { id: 7, rule: 'Events reach only the direct parent', pass: m.r7.afterA.a === '1' && m.r7.afterA.b === '0' && m.r7.afterA.view === '0' && m.r7.afterB.b === '1' && m.r7.afterB.view === '1' && m.r7.afterB.last === 'b-relayed' && m.r7.afterSib.sib === '1' && m.r7.afterSib.view === '1' && m.r7.afterSib.a === '1' && m.r7.domPings === 0,
      brief: `leaf a→mid only (view ${m.r7.afterA.view}); b relayed as "${m.r7.afterB.last}"; DOM ping events ${m.r7.domPings}`, evidence: m.r7 },
    { id: 8, rule: 'Slots keep the author\'s scope', pass: slotFails.length === 0,
      brief: slotFails.length ? slotFails.join('; ') : 'slotted content styled and bound by its author', evidence: { fails: slotFails, text: m.slotText } },
    { id: 9, rule: 'Error isolation', pass: !/Boom failed/.test(r9.afterHandler.boom1) && r9.afterHandler.boom1Clicks === '1' && r9.afterHandler.boom2 === 'fine' && /Boom failed/.test(r9.afterDrv.boom2) && r9.afterDrv.mainKids === 11 && r9.afterCounter === 'After: 2 × 2 = 4' && /^Three: 1/.test(r9.c3) && /Setup failed/.test(e.bad) && /1/.test(e.before) && /1/.test(e.after),
      brief: `handler throw: #boom1 ${/Boom failed/.test(r9.afterHandler.boom1) ? 'REPLACED by its fallback "' + r9.afterHandler.boom1 + '" (SPEC: a failing click handler keeps the component and shows a toast)' : 'kept, clicks ' + r9.afterHandler.boom1Clicks}; drv throw → "${r9.afterDrv.boom2.slice(0, 24)}"; setup throw → "${String(e.bad).slice(0, 24)}"; neighbours "${r9.afterCounter}", "${e.before}", "${e.after}"`, evidence: { ...r9, errorsView: e } },
    { id: 10, rule: 'CSS cannot escape its component', pass: escFails.length === 0,
      brief: escFails.length ? escFails.join('; ') : `body ${m.escape.body}, main ${m.escape.main}, sibling p ${css['v-after-escape']?.fontSize}`, evidence: { fails: escFails, escape: m.escape, afterEscape: css['v-after-escape'] } },
    { id: 11, rule: 'JS same names stay module-local', pass: m.r11[0].count === '1' && m.r11[1].count === '0' && m.r11[2].count === '1010' && m.r11[0].items === 'a1,a2' && m.r11[2].items === 'b1,b2,b3' && m.globals.leakedAppNames.length === 0 && m.globalsAfter.leakedAppNames.length === 0,
      brief: `counts ${m.r11.map((x) => x.count).join('/')}; leaked globals ${m.globalsAfter.leakedAppNames.length}`, evidence: { instances: m.r11, globalsAtLoad: m.globals, globalsAfter: m.globalsAfter } },
    { id: 12, rule: 'Custom property per instance', pass: m.r12.plain === ACC && m.r12.custom === 'rgb(10, 20, 30)' && m.r12.viewTitle === ACC,
      brief: `plain ${m.r12.plain}, custom ${m.r12.custom}, view ${m.r12.viewTitle}`, evidence: m.r12 },
  ]
}

// ---------- main ----------
mkdirSync(join(OUT, 'raw'), { recursive: true })
const check = execFileSync(BRAIN, ['check', '--json', '.'], { cwd: APP, encoding: 'utf8' })
const diags = JSON.parse(check || '[]')
if (diags.some((d) => (d.severity ?? d.level ?? 'error') === 'error')) fatal('`brain check` reports errors in the bench app:\n' + check)

const chrome = await launch()
children.add({ kill: () => chrome.close() })
const modes = {}
let buildMs = null
for (const mode of MODES) {
  const started = Date.now()
  let srv
  if (mode === 'dev') srv = await devServer(DEV_PORT)
  else if (mode === 'prod') {
    if (!NO_BUILD) {
      const b0 = Date.now()
      try { execFileSync(BRAIN, ['build'], { cwd: APP, stdio: 'pipe' }) } catch (err) { fatal('`brain build` failed:\n' + (err.stdout || '') + (err.stderr || '')) }
      buildMs = Date.now() - b0
    }
    srv = await staticServer(join(APP, 'dist'))
  } else fatal(`unknown mode ${mode} (dev, prod)`)
  let raw
  try { raw = await probe(chrome.port, srv.url, mode === 'prod') } catch (err) { srv.stop(); fatal(`${mode}: ${err.message}`) }
  srv.stop()
  const rules = verdicts(raw)
  modes[mode] = { url: srv.url, ms: Date.now() - started, rules, missingClickTargets: raw.m.missing, consoleErrors: raw.logs }
  writeFileSync(join(OUT, 'raw', `${mode}.json`), JSON.stringify({ mode, url: srv.url, raw, rules }, null, 1))
}
chrome.close()

// ---------- report ----------
const ids = modes[MODES[0]].rules.map((r) => r.id)
const row = (id) => {
  const per = Object.fromEntries(MODES.map((md) => [md, modes[md].rules.find((r) => r.id === id)]))
  const first = per[MODES[0]]
  const failing = MODES.filter((md) => !per[md].pass)
  const evidence = failing.length ? failing.map((md) => `${md}: ${per[md].brief}`).join(' · ') : first.brief
  return { id, rule: first.rule, ...Object.fromEntries(MODES.map((md) => [md, per[md].pass])), evidence }
}
const table = ids.map(row)
const allPass = table.every((r) => MODES.every((md) => r[md]))
const totalMs = Date.now() - t0
const summary = {
  bench: 'scopes', date: new Date().toISOString(), brainSha256: brainSha, modes: MODES, pass: allPass,
  passed: Object.fromEntries(MODES.map((md) => [md, modes[md].rules.filter((r) => r.pass).length + '/' + modes[md].rules.length])),
  runtimeMs: { total: totalMs, build: buildMs, ...Object.fromEntries(MODES.map((md) => [md, modes[md].ms])) },
  table,
  missingClickTargets: Object.fromEntries(MODES.map((md) => [md, modes[md].missingClickTargets])),
  consoleErrors: Object.fromEntries(MODES.map((md) => [md, modes[md].consoleErrors.length])),
}
writeFileSync(join(OUT, 'summary.json'), JSON.stringify(summary, null, 1) + '\n')
const esc = (s) => String(s).replace(/\|/g, '\\|')
const md = [
  '# Scopes bench — results',
  '',
  `Binary sha256 \`${brainSha}\` · ${summary.date} · total ${(totalMs / 1000).toFixed(1)} s (build ${buildMs == null ? 'skipped' : (buildMs / 1000).toFixed(1) + ' s'}) · ` +
    MODES.map((m) => `${m} ${summary.passed[m]}`).join(' · ') + ` · verdict **${allPass ? 'PASS' : 'FAIL'}**`,
  '',
  `| # | Rule | ${MODES.join(' | ')} | Evidence |`,
  `|---|---|${MODES.map(() => '---').join('|')}|---|`,
  ...table.map((r) => `| ${r.id} | ${r.rule} | ${MODES.map((m) => (r[m] ? 'pass' : '**FAIL**')).join(' | ')} | ${esc(r.evidence)} |`),
  '',
  'Raw per-mode data (every computed style, mutation record and console line): `results/raw/<mode>.json` (not committed).',
  '',
].join('\n')
writeFileSync(join(OUT, 'summary.md'), md)

const pad = (s, n) => (s + ' '.repeat(n)).slice(0, n)
console.log(`scopes bench · brain ${brainSha} · ${(totalMs / 1000).toFixed(1)} s`)
console.log(`${pad('#', 3)}${pad('rule', 38)}${MODES.map((m) => pad(m, 6)).join('')}evidence`)
for (const r of table) console.log(`${pad(String(r.id), 3)}${pad(r.rule, 38)}${MODES.map((m) => pad(r[m] ? 'pass' : 'FAIL', 6)).join('')}${r.evidence}`)
console.log(`verdict ${allPass ? 'PASS' : 'FAIL'} → bench/scopes/results/summary.md`)
process.exit(allPass ? 0 : 1)
