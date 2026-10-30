// brain dev HMR client (SPEC §8). CSS: swaps the stylesheet without remounting. JS: `hmr(hash, mod)` from
// /@brain/brain-dev.js remounts keeping the sigs; if it is missing or fails, the page reloads.
// brain.json `base` (#148): every dev URL lives under it (`<meta name="brain-base" content="/app/">`)
const base = (document.querySelector('meta[name="brain-base"]')?.content ?? '/').replace(/\/$/, '');
const url = `ws://${location.host}${base}/@brain/hmr`;
let box;
// Timing to measure HMR latency (QA, bench/sites/hmr.mjs): epoch ms, same base as Date.now().
const now = () => performance.timeOrigin + performance.now();
const applied = (kind, file, receivedAt, outcome = {}) =>
  dispatchEvent(new CustomEvent('brain:hmr', { detail: { kind, file, receivedAt, appliedAt: now(), ...outcome } }));
function overlay(diags) {
  box ??= Object.assign(document.createElement('pre'), { id: '__brain_overlay' });
  // a panel at the bottom, not a full-page cover: the rest of the app stays clickable while one file is broken
  box.style.cssText = 'position:fixed;left:12px;right:12px;bottom:12px;max-height:40vh;margin:0;padding:16px 20px;z-index:2147483647;border-radius:10px;background:rgba(20,20,24,.94);color:#ffd7d7;font:13px/1.5 ui-monospace,monospace;white-space:pre-wrap;overflow:auto;box-shadow:0 8px 30px rgba(0,0,0,.3)';
  box.textContent = diags.map(d => `${d.code} ${d.file}:${d.line}:${d.col}\n  ${d.msg}\n  fix: ${d.fix}`).join('\n\n');
  document.body.append(box);
}
window.__brainOverlay = overlay;
// the dev overlay (RUNTIME #267, runtime/brain-overlay.js): a bottom panel for errors with snippet, fix and «Copy for
// the LLM», a corner chip for warnings; falls back to the plain panel above if it cannot load
let O = null;
const ov = import(base + '/@brain/brain-overlay.js').then(m => (O = m), () => null);
async function show(all, file) {
  await ov;
  if (O) O.show(all, { file });
  else if (all.some(d => d.level === 'error')) overlay(all.filter(d => d.level === 'error'));
}
// dev only: the runtime reports extra diagnostics (missing i18n keys, `brain:net` per api() request)
globalThis.__brainDev = true;
// Browser errors → server (brain_errors via MCP, brain run terminal). Queued until the WS opens.
let ws;
const pending = [];
const report = e => {
  const msg = JSON.stringify({ t: 'brain:error', url: location.pathname, at: Date.now(), ...e });
  ws?.readyState === 1 ? ws.send(msg) : pending.length < 50 && pending.push(msg);
};
// Errors are recorded by the runtime's brain-err (window errors, rejections and what component boundaries catch),
// mapped to `.brain` lines through the dev modules' inline source maps (`src: 'comp/inner.brain:6'`, `brainStack`).
// A record is sent 300 ms after its last update: the source-map mapping fires `sub` again once it resolves.
let errOn = false;
const quiet = new Map();
import(base + '/@brain/brain-err.js').then(({ capture, sub }) => {
  capture({ maps: true, log: true });
  errOn = true;
  sub.add(r => {
    clearTimeout(quiet.get(r.fp));
    quiet.set(r.fp, setTimeout(() => {
      quiet.delete(r.fp);
      const st = r.brainStack ?? r.stack ?? [];
      report({ message: r.msg, type: r.type, code: r.code, src: r.src ?? r.line, stack: Array.isArray(st) ? st.join('\n') : String(st), comp: r.comp ? (r.comp.name ?? r.comp.file ?? null) : null, where: r.where, location: r.location, generatedLocation: r.generatedLocation, frames: r.frames, causes: r.causes, truncated: r.truncated, mapping: r.mapping, brainStack: r.brainStack ?? null, componentChain: r.comp?.chain ?? [], crumbs: r.crumbs, count: r.n, fp: r.fp });
      ov.then(() => O && O.error(r));
    }, 300));
  });
}, () => {});
// before brain-err loads (or if it cannot), the plain browser events still reach brain_errors
addEventListener('error', ev => errOn || report({ message: String(ev.message), stack: String(ev.error?.stack ?? ''), source: ev.filename ?? '', line: ev.lineno ?? 0, col: ev.colno ?? 0 }));
addEventListener('unhandledrejection', ev => errOn || report({ message: String(ev.reason?.message ?? ev.reason), stack: String(ev.reason?.stack ?? '') }));
// network panel (brain_logs, `brain run`): method, URL, status, ms and size of every api() request — never bodies
addEventListener('brain:net', ev => {
  const msg = JSON.stringify({ t: 'brain:net', ...ev.detail });
  ws?.readyState === 1 ? ws.send(msg) : pending.length < 50 && pending.push(msg);
});
let hotModules = 0;
async function hot(path, receivedAt) {
  try {
    // Browsers do not evict evaluated ESM versions; bound cache-busting versions in a long dev session.
    if (++hotModules > 256) return reload(path, receivedAt, 'native-module-cache-budget');
    const mod = await import(`${base}${path}?t=${Date.now()}`);
    const hash = mod.default?.h;
    const dev = await import(base + '/@brain/brain-dev.js');
    if (!hash || typeof dev.hmr !== 'function') return reload(path, receivedAt, 'module-graph-or-runtime-version-change');
    const count = dev.hmr(hash, mod);
    const outcome = dev.lastUpdate || { mode: 'component-remount', reason: 'legacy-runtime', remounted: count, patched: 0 };
    if (!count && outcome.mode !== 'future-mount-updated') return reload(path, receivedAt, outcome.reason || 'runtime-version-incompatible');
    applied('js', path, receivedAt, outcome);
  } catch {
    reload(path, receivedAt, 'module-import-or-runtime-version-failed');
  }
}
// A file that stopped compiling: its live instances become the error box in place (HMR, same hash as the last good
// version), so siblings keep working and keep their state; the panel lists the diagnostics without covering the page.
async function broken(diags, receivedAt, file, all) {
  let placed = 0;
  for (const file of new Set(diags.map(d => d.file).filter(f => f && f.endsWith('.brain')))) {
    try {
      const mod = await import(`${base}/${file}?t=${Date.now()}`);
      const dev = await import(base + '/@brain/brain-dev.js');
      const n = mod.default?.h && typeof dev.hmr === 'function' ? dev.hmr(mod.default.h, mod) : 0;
      if (n) (placed += n), applied('error-box', '/' + file, receivedAt);
    } catch {}
  }
  // the box shows the diagnostics where the component was; the panel only when no live instance could take it
  // (the warnings of the file still go to the chip)
  const list = all ?? diags;
  if (file) await show(placed ? list.filter(d => d.level !== 'error') : list, file);
  else if (!placed) overlay(diags);
}
function reload(file, receivedAt, reason = 'document-shell-or-server-contract-change') {
  applied('reload', file, receivedAt, { mode: 'full-reload', reason });
  console.info('brain:hmr', { mode: 'full-reload', reason, file });
  location.reload();
}
// Swaps the stylesheet without remounting; `appliedAt` is taken once the new sheet has loaded.
function swapCss(path, receivedAt) {
  const links = [...document.querySelectorAll(`link[data-brain="${base}${path}"]`)];
  if (!links.length) return applied('css', path, receivedAt);
  let left = links.length;
  const done = () => --left === 0 && applied('css', path, receivedAt);
  for (const l of links) {
    const next = l.cloneNode();
    next.href = `${base}${path}?css&t=${Date.now()}`;
    next.onload = next.onerror = () => { l.remove(); done(); };
    l.after(next);
  }
}
function connect() {
  ws = new WebSocket(url);
  ws.onopen = () => { while (pending.length) ws.send(pending.shift()); };
  ws.onmessage = e => {
    const receivedAt = now();
    const m = JSON.parse(e.data);
    if (m.type === 'error') return broken(m.diags, receivedAt, m.file, m.all);
    if (m.type === 'diags') return show(m.all, m.file);
    box?.remove();
    if (m.type === 'reload') return reload(m.path ?? null, receivedAt);
    // one save may change both blocks: swap the stylesheet AND hot-swap the code (state survives)
    if (m.css) swapCss(m.path, receivedAt);
    if (m.js) hot(m.path, receivedAt);
  };
  ws.onclose = () => {
    const retry = () => fetch(base + '/@brain/ping').then(() => location.reload(), () => setTimeout(retry, 500));
    setTimeout(retry, 500);
  };
}
connect();

// ---------- dev routing (brain run) ----------
// The page carries its route as JSON (`#brain-route`: view, layouts, props from load). route() mounts it; same-origin
// link clicks fetch the next page, read its route JSON and mount it in place, so module state survives navigation.
let dispose = null;
let why = null;
let insp = null;
let mockOn = null;
export async function route(data) {
  const el = document.getElementById('brain-view');
  el.dataset.brainReady = 'false';
  data ??= JSON.parse(document.getElementById('brain-route').textContent);
  const { mount } = await import(base + '/@brain/brain.js');
  // `brain run --mock[=seed]` (#266): api() answers fake, deterministic data from its schema (RUNTIME's brain-mock.js)
  await (mockOn ??= (async () => {
    const seed = document.querySelector('meta[name="brain-mock"]')?.content;
    if (seed == null) return;
    const [{ mocker }, { http }] = await Promise.all([import(base + '/@brain/brain-mock.js'), import(base + '/@brain/brain-data.js')]);
    http({ mock: mocker(seed) });
  })().catch(() => null));
  // dev only: update provenance for brain_why and updates per interaction for brain_impact (RUNTIME #198)
  await (why ??= import(base + '/@brain/brain-why.js').catch(() => null));
  // the memory/latency inspector (§9): its batches go to the dev server over this socket (GET /@brain/metrics,
  // MCP brain_inspect); dropped while the socket is reconnecting
  await (insp ??= import(base + '/@brain/brain-inspect.js').then(m => m.inspect({ send: s => ws?.readyState === 1 && ws.send(s) }), () => null));
  const at = u => (u.startsWith('/') && !u.startsWith(base + '/') ? base + u : u);
  const V = (await import(at(data.v))).default;
  const L = [];
  for (const l of data.l) L.push((await import(at(l))).default);
  if (typeof dispose === 'function') dispose();
  el.replaceChildren();
  let tree = () => V(data.p);
  for (let i = L.length - 1; i > 0; i--) { const inner = tree, C = L[i]; tree = () => C({ children: inner }); }
  dispose = L.length ? mount(L[0], el, { children: tree }) : mount(V, el, data.p);
  current();
  // Document load can precede these dynamic imports. MCP must wait for the actual mounted view.
  el.dataset.brainReady = 'true';
}
// `_loading.brain` while the next page's load runs (§22): shown only if the response takes over 150 ms, so fast
// navigations never flash it
let seq = 0;
async function loading(url, mine) {
  await new Promise((r) => setTimeout(r, 150));
  if (mine !== seq) return;
  try {
    const u = new URL(url, location.href);
    const d = await (await fetch(`${base}/@brain/loading?path=${encodeURIComponent(u.pathname)}`)).json();
    if (d.v && mine === seq) await route({ v: d.v, l: d.l, p: {} });
  } catch {}
}
async function go(url, push) {
  // same event as brain-router.js: brain-data aborts background requests (priority low, prefetch) on it
  dispatchEvent(new CustomEvent('brain:navigate', { detail: { url } }));
  const mine = ++seq;
  loading(url, mine);
  try {
    const r = await fetch(url, { headers: { accept: 'text/html' } });
    seq++; // the response arrived: a pending loading state must not mount any more
    // a guard redirected or denied (or the page failed): let the server answer the navigation itself
    if (r.redirected || r.status !== 200) return location.assign(r.redirected ? r.url : url);
    const doc = new DOMParser().parseFromString(await r.text(), 'text/html');
    const json = doc.getElementById('brain-route');
    // another language: its dictionary and <html lang> come with a full load (as brain-router.js does)
    if (!json || doc.documentElement.lang !== document.documentElement.lang) return location.assign(url);
    // the scroll of the page we leave, restored when back/forward returns to it (brain-router.js does the same)
    if (push) history.replaceState({ ...history.state, y: scrollY }, ''), history.pushState({ y: 0 }, '', url);
    document.title = doc.title;
    // the head tags a view sets (seo/head): description, canonical, robots, Open Graph, Twitter, JSON-LD
    const seoTags = 'meta[name=description],link[rel=canonical],meta[name=robots],meta[property^="og:"],meta[name^="twitter:"],script[type="application/ld+json"]';
    for (const n of document.head.querySelectorAll(seoTags)) n.remove();
    for (const n of doc.head.querySelectorAll(seoTags)) document.head.append(document.adoptNode(n));
    // route().params (brain-router.js) reads <meta name="brain-params">: carry the next page's (or drop it)
    document.querySelector('meta[name="brain-params"]')?.remove();
    const pm = doc.querySelector('meta[name="brain-params"]');
    if (pm) document.head.append(pm);
    await route(JSON.parse(json.textContent));
    scrollTo(0, push ? 0 : (history.state && history.state.y) || 0);
  } catch {
    location.assign(url);
  }
}
addEventListener('click', e => {
  const a = e.target.closest?.('a[href]');
  if (!a || e.defaultPrevented || e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey || e.altKey || a.target || a.hasAttribute('download')) return;
  const u = new URL(a.href, location.href);
  if (u.origin !== location.origin || u.pathname.startsWith(base + '/@brain/') || /\.[a-z0-9]+$/i.test(u.pathname)) return;
  if (u.pathname === location.pathname && u.hash) return;
  e.preventDefault();
  go(u.href, true);
});
addEventListener('popstate', () => go(location.href, false));
// active links (§41.5, as brain-router.js): every internal <a href> to the current page gets aria-current="page"
function current() {
  for (const a of document.querySelectorAll('a[href]')) {
    let u;
    try { u = new URL(a.getAttribute('href'), location.href); } catch { continue; }
    if (u.origin === location.origin && u.pathname.replace(/\/$/, '') === location.pathname.replace(/\/$/, '')) a.setAttribute('aria-current', 'page');
    else if (a.getAttribute('aria-current') === 'page') a.removeAttribute('aria-current');
  }
}
history.scrollRestoration = 'manual';
