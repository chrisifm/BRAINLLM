// QA #124 — repeat-visit cache bench (INTERNALS §35, the part that does not depend on the build signature #130).
// For every app: build with the current `brain`, serve dist/ with exactly the headers the build emits (_headers, see
// lib/static.mjs), and with Slow 4G + 4x CPU measure a cold visit and a repeat visit (bytes and requests that reached
// the server, LCP), then offline navigation to every visited route. Also: `brain run` registers no service worker and
// removes an existing one, and each brain.json opt-out (cache.sw/http/offline/data) really turns its layer off.
//
//   node bench/cache/run.mjs [--runs 3] [--apps tasks,landing] [--skip-optouts] [--skip-dev]
//
// Inputs: the example apps in $BRAIN_APPS (default ~/tmp/brainllm/apps), copied to $BRAIN_CACHE_SCRATCH
// (default ~/tmp/brainllm-cache) before building; never built in place. Output: results/summary.json,
// results/summary.md and results/raw/*.json (ignored by git).
import { homedir } from 'node:os'
import { spawn, spawnSync } from 'node:child_process'
import { cpSync, rmSync, mkdirSync, writeFileSync, readFileSync, existsSync, readdirSync, statSync } from 'node:fs'
import { join, dirname, relative } from 'node:path'
import { fileURLToPath } from 'node:url'
import { createHash } from 'node:crypto'
import { startStatic } from './lib/static.mjs'
import { launchBrowser, sleep, SLOW_4G, OFFLINE, CPU_RATE } from './lib/browser.mjs'
import { runSignature } from './lib/signature.mjs'

const HERE = dirname(fileURLToPath(import.meta.url))
const REPO = join(HERE, '..', '..')
const BRAIN_SRC = process.env.BRAIN_BIN ?? join(REPO, 'target', 'debug', 'brain')
const BRAIN = join(process.env.BRAIN_CACHE_SCRATCH ?? join(homedir(), 'tmp/brainllm-cache'), 'brain') // snapshot: other sessions rebuild target/ while this runs
const SRC_APPS = process.env.BRAIN_APPS ?? join(homedir(), 'tmp/brainllm/apps')
const SCRATCH = process.env.BRAIN_CACHE_SCRATCH ?? join(homedir(), 'tmp/brainllm-cache')
const OUT = join(HERE, 'results')
const RAW = join(OUT, 'raw')
const arg = (k, d) => { const i = process.argv.indexOf(k); return i > 0 ? process.argv[i + 1] : d }
const RUNS = Number(arg('--runs', 3))
const ONLY = arg('--apps', null)?.split(',')
const API_DELAY = 1500

// ---------- apps ----------
// The probe page is found by data-p attributes, never by class: `brain build` renames component classes (#260).
const DATA_APP = {
  'types.jsbrain': "export const Item = { id: 'num!', name: 'str!' }\n",
  'api/items.jsbrain': "export default api('/api/items', { get: { out: '[Item]', cache: '10m' } })\n",
  'views/home.brain': `<meta seo title="Data" description="resource() cache probe." canonical="/">
<script>
  import itemsApi from '../api/items.jsbrain'
  // the API is called only after a click: \`brain build\` prerenders the view in Node, where a plain
  // resource(itemsApi.get) fails ("Failed to parse URL from /api/items") and the page ships that error (see README)
  let ready = sig(false)
  let items = resource(() => (ready ? itemsApi.get() : Promise.resolve([])))
</script>
<main class="home">
  <h1>Items</h1>
  {#await items}
    <p class="loading">Loading…</p>
  {:then list}
    <ul>{#each list as it (it.id)}<li class="item" data-p="item">{it.name}</li>{:empty}<li>No items.</li>{/each}</ul>
  {:catch e}
    <p class="error" role="alert">Could not load items.</p>
  {/await}
  <button type="button" class="load" data-p="load" on:click={() => (ready = true)}>Load</button>
  <a href="/about">About</a>
  <img src="/logo.svg" alt="Logo" width="32" height="32">
</main>
`,
  'views/about.brain': `<meta seo title="About" description="About page of the data probe." canonical="/about">
<main class="about">
  <h1>About</h1>
  <a href="/">Back</a>
</main>
`,
  'public/logo.svg': '<svg xmlns="http://www.w3.org/2000/svg" width="32" height="32"><rect width="32" height="32" fill="#888"/></svg>',
  'public/notes.txt': 'hello\n',
}
// the same view with a plain resource(): used only to record what the prerender bakes into the page
const PLAIN_HOME = DATA_APP['views/home.brain'].replace(/  \/\/ the API[\s\S]*?Promise\.resolve\(\[\]\)\)\)\n/, '  let items = resource(itemsApi.get)\n').replace(/  <button[^\n]*\n/, '')
const API = {
  '/api/items': { delay: API_DELAY, body: [{ id: 1, name: 'alpha' }, { id: 2, name: 'beta' }] },
  // not under /api/: only its Cache-Control says it must never be stored by a shared cache
  '/me.json': { delay: 0, cc: 'private, max-age=60', body: { name: 'someone' } },
}

const sh = (cmd, args, cwd) => {
  const r = spawnSync(cmd, args, { cwd, encoding: 'utf8' })
  return { ok: r.status === 0, out: String(r.stdout || '') + String(r.stderr || '') }
}

function prepare() {
  rmSync(join(SCRATCH, 'apps'), { recursive: true, force: true })
  mkdirSync(join(SCRATCH, 'apps'), { recursive: true })
  const apps = []
  for (const name of ['tasks', 'landing', 'my-project']) {
    const dst = join(SCRATCH, 'apps', name)
    cpSync(join(SRC_APPS, name), dst, { recursive: true })
    rmSync(join(dst, 'dist'), { recursive: true, force: true })
    apps.push({ name, dir: dst, origin: `example ${name}` })
  }
  for (const name of ['fresh', 'data']) {
    const r = sh(BRAIN, ['new', name], join(SCRATCH, 'apps'))
    if (!r.ok) throw new Error(`brain new ${name} failed: ${r.out}`)
    apps.push({ name, dir: join(SCRATCH, 'apps', name), origin: name === 'fresh' ? 'fresh `brain new`' : '`brain new` + api() + resource() + public/' })
  }
  for (const [f, s] of Object.entries(DATA_APP)) {
    const p = join(SCRATCH, 'apps', 'data', f)
    mkdirSync(dirname(p), { recursive: true })
    writeFileSync(p, s)
  }
  return apps
}

function build(dir, cache) {
  if (cache) writeFileSync(join(dir, 'brain.json'), JSON.stringify({ cache }, null, 2) + '\n')
  rmSync(join(dir, 'dist'), { recursive: true, force: true })
  const r = sh(BRAIN, ['build'], dir)
  if (!r.ok) throw new Error(`brain build failed in ${dir}:\n${r.out}`)
  return { log: r.out.trim().split('\n').slice(-3), full: r.out, routes: routesOf(join(dir, 'dist')) }
}

function routesOf(dist) {
  const out = []
  const walk = (d) => {
    for (const f of readdirSync(d)) {
      const p = join(d, f)
      if (statSync(p).isDirectory()) { if (!f.startsWith('_')) walk(p) }
      else if (f === 'index.html' || f === 'index.brain') out.push('/' + relative(dist, d).split('\\').join('/'))
    }
  }
  walk(dist)
  return out.map((r) => (r === '/' ? '/' : r.replace(/\/$/, ''))).sort((a, b) => a.length - b.length)
}

// ---------- measurement helpers ----------
async function quiet(server, ms = 1000, max = 20000) {
  const t0 = Date.now()
  let n = server.log().length, last = Date.now()
  while (Date.now() - t0 < max) {
    await sleep(100)
    const m = server.log().length
    if (m !== n) { n = m; last = Date.now() }
    else if (Date.now() - last >= ms) return
  }
}

const kind = (p) => (p === '/sw.js' ? 'sw' : /\.(js|jsbrain)$/.test(p) ? 'js' : /\.(css|cssbrain)$/.test(p) ? 'css' : p.startsWith('/api/') ? 'api' : /\.[a-z0-9]+$/.test(p) && !p.endsWith('.html') ? 'other' : 'html')

function serverSummary(entries) {
  const s = { requests: entries.length, bytes: 0, byKind: {}, jsCss200: 0, jsCss200Bytes: 0, statuses: {} }
  for (const e of entries) {
    s.bytes += e.bytes
    const k = kind(e.path)
    s.byKind[k] ??= { n: 0, bytes: 0, s200: 0, s304: 0 }
    s.byKind[k].n++; s.byKind[k].bytes += e.bytes
    if (e.status === 200) s.byKind[k].s200++
    if (e.status === 304) s.byKind[k].s304++
    s.statuses[e.status] = (s.statuses[e.status] ?? 0) + 1
    if ((k === 'js' || k === 'css') && e.status === 200) { s.jsCss200++; s.jsCss200Bytes += e.bytes }
  }
  return s
}

function pageSummary(ledger) {
  const own = ledger.filter((r) => /^https?:/.test(r.url))
  return {
    requests: own.length,
    network: own.filter((r) => !r.fromSW && !r.fromDisk && !r.fromMemory && !r.fromPrefetch && !r.failed).length,
    fromSW: own.filter((r) => r.fromSW).length,
    fromHttpCache: own.filter((r) => r.fromDisk || r.fromMemory).length,
    encodedBytes: own.reduce((a, r) => a + (r.fromSW || r.fromDisk || r.fromMemory ? 0 : r.encoded), 0),
  }
}

const SW_READY = `(async()=>{if(!('serviceWorker' in navigator))return 'unsupported';const t=Date.now();
  while(Date.now()-t<15000){const r=await navigator.serviceWorker.getRegistration();if(r&&r.active&&r.active.state==='activated')return 'activated';await new Promise(r=>setTimeout(r,100))}
  const r=await navigator.serviceWorker.getRegistration();return r?(r.active?r.active.state:(r.installing?'installing':'waiting')):'none'})()`
const STATE = `(async()=>({url:location.href,h1:document.querySelector('h1')?.textContent?.trim()??null,title:document.title,
  lcp:self.__lcp,fcp:performance.getEntriesByName('first-contentful-paint')[0]?.startTime??null,
  regs:'serviceWorker' in navigator?(await navigator.serviceWorker.getRegistrations()).length:-1,
  controlled:!!navigator.serviceWorker?.controller,caches:self.caches?await caches.keys():[]}))()`

async function visit(browser, page, server, url, { waitSW = false } = {}) {
  await browser.setConditions(SLOW_4G, page.sessionId)
  page.resetLedger()
  const m = server.mark()
  const nav = await page.goto(url)
  const sw = waitSW ? await page.evaluate(SW_READY, 20000).catch((e) => 'error: ' + e.message) : null
  await quiet(server, 1200)
  const st = await page.evaluate(STATE).catch((e) => ({ error: e.message }))
  return { url, loadEvent: nav.ok, navError: nav.errorText, sw, state: st, server: serverSummary(server.since(m)), serverEntries: server.since(m), page: pageSummary(page.ledger()) }
}

async function measureRun(app, dist, run) {
  const server = await startStatic({ api: API })
  server.setRoot(dist)
  const base = `http://127.0.0.1:${server.port}`
  const browser = await launchBrowser(join(SCRATCH, 'profiles', `${app.name}-${run}`))
  const res = { app: app.name, run }
  try {
    // 1. cold visit
    const p1 = await browser.newPage()
    res.first = await visit(browser, p1, server, base + '/', { waitSW: true })
    // visit the other routes (full navigations) so they are known offline
    res.routes = {}
    res.routes['/'] = res.first.state.h1
    for (const r of app.routes.filter((r) => r !== '/')) {
      const v = await visit(browser, p1, server, base + r)
      res.routes[r] = v.state.h1
    }
    await p1.close()
    // 2. repeat visit: same profile, new page, new navigation
    const p2 = await browser.newPage()
    res.second = await visit(browser, p2, server, base + '/')
    // 3. offline: the origin is gone (server closed) and the network is emulated offline for the page and the worker
    await server.close()
    await browser.setConditions(OFFLINE, p2.sessionId)
    res.offline = []
    for (const [r, h1] of Object.entries(res.routes)) {
      const nav = await p2.goto(base + r, 15000)
      await sleep(500)
      const st = await p2.evaluate(STATE).catch((e) => ({ error: e.message }))
      const errorPage = !st.url || st.url.startsWith('chrome-error://') || !!nav.errorText
      res.offline.push({ route: r, expectedH1: h1, h1: st.h1, ok: !errorPage && !!st.h1 && st.h1 === h1, errorPage, navError: nav.errorText, url: st.url })
    }
    {
      const nav = await p2.goto(base + '/never-visited', 15000)
      await sleep(500)
      const st = await p2.evaluate(STATE).catch((e) => ({ error: e.message }))
      res.offlineUnvisited = { h1: st.h1 ?? null, errorPage: !st.url || st.url.startsWith('chrome-error://') || !!nav.errorText, navError: nav.errorText }
    }
    await p2.close()
  } finally {
    await server.close().catch(() => {})
    await browser.close()
  }
  return res
}

const median = (xs) => { const v = xs.filter((x) => typeof x === 'number').sort((a, b) => a - b); return v.length ? v[Math.floor((v.length - 1) / 2)] : null }

function aggregate(app, runs) {
  const pick = (f) => runs.map(f)
  const first = { bytes: median(pick((r) => r.first.server.bytes)), requests: median(pick((r) => r.first.server.requests)), lcp: median(pick((r) => r.first.state.lcp)), jsCss200Bytes: median(pick((r) => r.first.server.jsCss200Bytes)) }
  const second = { bytes: median(pick((r) => r.second.server.bytes)), requests: median(pick((r) => r.second.server.requests)), lcp: median(pick((r) => r.second.state.lcp)), jsCss200Bytes: Math.max(...pick((r) => r.second.server.jsCss200Bytes)) }
  const offRoutes = runs[0].offline.map((o) => o.route)
  const offlineOk = offRoutes.map((route) => ({ route, ok: runs.every((r) => r.offline.find((o) => o.route === route)?.ok) }))
  const notes = []
  const sec = runs[0].second.serverEntries.map((e) => `${e.path} ${e.status}`)
  if (second.jsCss200Bytes > 0) notes.push(`repeat visit downloaded JS/CSS: ${runs.flatMap((r) => r.second.serverEntries.filter((e) => /\.(js|css)$/.test(e.path) && e.status === 200).map((e) => e.path)).join(', ')}`)
  if (!(second.lcp < first.lcp)) notes.push(`LCP not lower on the repeat visit`)
  const dup = runs[0].first.serverEntries.filter((e) => e.path.startsWith('/_b/') && e.status === 200)
  const dupPaths = dup.map((e) => e.path).filter((p, i, a) => a.indexOf(p) !== i)
  if (dupPaths.length) notes.push(`cold visit downloads ${dupPaths.length} hashed files twice (page + worker precache), ${dup.filter((e, i) => dup.findIndex((x) => x.path === e.path) !== i).reduce((a, e) => a + e.bytes, 0)} B extra`)
  const failsOffline = offlineOk.filter((o) => !o.ok)
  if (failsOffline.length) notes.push(`offline failed: ${failsOffline.map((o) => o.route).join(', ')}`)
  const dupBytes = dup.filter((e, i) => dup.findIndex((x) => x.path === e.path) !== i).reduce((a, e) => a + e.bytes, 0)
  return {
    app: app.name, origin: app.origin, dupBytes, routes: app.routes, runs: runs.length, first, second,
    secondRequestsSeen: sec, swState: pick((r) => r.first.sw), controlledOnSecond: pick((r) => r.second.state.controlled),
    offline: offlineOk, offlineUnvisited: runs[0].offlineUnvisited,
    pass: { zeroJsCss: second.jsCss200Bytes === 0, lcpLower: second.lcp < first.lcp, offline: failsOffline.length === 0 },
    notes,
  }
}

// ---------- dev server (`brain run`) ----------
function startDev(dir, port) {
  return new Promise((ok, fail) => {
    const p = spawn(BRAIN, ['run', '--port', String(port)], { cwd: dir, stdio: ['ignore', 'pipe', 'pipe'] })
    let buf = ''
    const t = setTimeout(() => fail(new Error('brain run did not start: ' + buf)), 15000)
    const on = (d) => { buf += d; const m = /http:\/\/127\.0\.0\.1:(\d+)/.exec(buf); if (m) { clearTimeout(t); ok({ proc: p, pid: p.pid, port: Number(m[1]) }) } }
    p.stdout.on('data', on); p.stderr.on('data', on)
  })
}
async function stopDev(d) {
  if (!d) return
  try { process.kill(d.pid, 'SIGTERM') } catch {}
  await sleep(400)
  try { process.kill(d.pid, 0); process.kill(d.pid, 'SIGKILL') } catch {}
}

async function devTest(app) {
  const out = {}
  // a) a fresh profile on `brain run` never gets a registration
  {
    const browser = await launchBrowser(join(SCRATCH, 'profiles', 'dev-a'))
    let dev
    try {
      const probe = await startStatic(); const port = probe.port; await probe.close()
      dev = await startDev(app.dir, port)
      const p = await browser.newPage({ mobile: false })
      await p.goto(`http://127.0.0.1:${dev.port}/`)
      await sleep(3000)
      out.fresh = await p.evaluate(STATE)
      out.fresh.pass = out.fresh.regs === 0
    } finally { await stopDev(dev); await browser.close() }
  }
  // b) a production worker registered on the same origin is removed when that origin serves `brain run`
  {
    const browser = await launchBrowser(join(SCRATCH, 'profiles', 'dev-b'))
    let dev, server
    try {
      server = await startStatic(); server.setRoot(join(app.dir, 'dist'))
      const port = server.port
      const p = await browser.newPage({ mobile: false })
      await p.goto(`http://127.0.0.1:${port}/`)
      out.before = { sw: await p.evaluate(SW_READY, 20000), ...(await p.evaluate(STATE)) }
      await server.close(); server = null
      dev = await startDev(app.dir, port)
      out.samePort = dev.port === port
      await p.goto(`http://127.0.0.1:${port}/`)
      await sleep(4000)
      out.after1 = await p.evaluate(STATE)
      await p.goto(`http://127.0.0.1:${port}/`)
      await sleep(2000)
      out.after2 = await p.evaluate(STATE)
      out.pass = out.before.regs === 1 && out.samePort && out.after2.regs === 0 && !out.after2.caches.some((c) => c.startsWith('brain-'))
    } finally { await stopDev(dev); if (server) await server.close(); await browser.close() }
  }
  return out
}

// ---------- opt-outs ----------
function headersOf(dir) {
  const d = join(dir, 'dist')
  return { _headers: existsSync(join(d, '_headers')), nginx: existsSync(join(d, '_brain', 'nginx.conf')) }
}

async function headerProbe(dist) {
  const server = await startStatic({ api: API }); server.setRoot(dist)
  const base = `http://127.0.0.1:${server.port}`
  const html = readFileSync(join(dist, 'index.html'), 'utf8')
  const chunk = /\/_b\/[a-z0-9]+\.js/.exec(html)?.[0]
  const css = /\/_b\/[a-z0-9]+\.css/.exec(html)?.[0]
  const img = /src="(\/[^"]+\.svg)"/.exec(html)?.[1]
  const out = {}
  for (const [k, p] of Object.entries({ html: '/', about: '/about', js: chunk, css, img, unhashedPublic: '/notes.txt', sw: '/sw.js' })) {
    if (!p) continue
    const r = await fetch(base + p, { headers: { 'accept-encoding': 'gzip' } })
    out[k] = { path: p, status: r.status, cacheControl: r.headers.get('cache-control'), etag: r.headers.get('etag') }
  }
  out.registerSnippet = /serviceWorker\.register\(/.test(html)
  out.swIsKill = readFileSync(join(dist, 'sw.js'), 'utf8').includes('registration.unregister()') && !readFileSync(join(dist, 'sw.js'), 'utf8').includes('BRAIN_BUILD')
  await server.close()
  return out
}

// Memory cache: click Load (API answers after API_DELAY ms), go to /about and back through the router, click Load
// again and time until the list is on screen. Persistent cache: on a new page of the same profile, click Load and time.
const WAIT = `const w=async(f,ms=8000)=>{const t=performance.now();while(performance.now()-t<ms){if(f())return Math.round(performance.now()-t);await new Promise(r=>setTimeout(r,20))}return null};`
const SPA_BACK = `(async()=>{${WAIT}const o={};
  if(await w(()=>document.querySelector('[data-p=load]'))===null)return {error:'no Load button'};
  document.querySelector('[data-p=load]').click();o.firstLoadMs=await w(()=>document.querySelector('[data-p=item]'));
  if(o.firstLoadMs===null)return {...o,error:'items never loaded: '+document.querySelector('main')?.textContent};
  document.querySelector('a[href="/about"]').click();o.aboutMs=await w(()=>document.querySelector('h1')?.textContent==='About');
  document.querySelector('a[href="/"]').click();await w(()=>document.querySelector('[data-p=load]'));
  document.querySelector('[data-p=load]').click();o.msToItemsOnBack=await w(()=>document.querySelector('[data-p=item]'));
  o.clientImgSrc=document.querySelector('main img')?.getAttribute('src');return o})()`
const TIME_TO_ITEMS = `(async()=>{${WAIT}await w(()=>document.querySelector('[data-p=load]'));document.querySelector('[data-p=load]').click();return w(()=>document.querySelector('[data-p=item]'),10000)})()`

async function dataProbe(dist) {
  const server = await startStatic({ api: API }); server.setRoot(dist)
  const base = `http://127.0.0.1:${server.port}`
  const browser = await launchBrowser(join(SCRATCH, 'profiles', 'data-' + createHash('sha1').update(dist).digest('hex').slice(0, 6)))
  const out = {}
  try {
    const p = await browser.newPage({ mobile: false })
    await p.goto(base + '/')
    out.swState = await p.evaluate(SW_READY, 20000)
    out.memory = await p.evaluate(SPA_BACK, 30000)
    await quiet(server, 800)
    const cacheDump = `(async()=>{const o={};for(const k of await caches.keys()){const c=await caches.open(k);o[k]=(await c.keys()).map(r=>new URL(r.url).pathname)}return o})()`
    await p.evaluate(`fetch('/me.json').then(r=>r.text())`)
    await sleep(500)
    out.caches = await p.evaluate(cacheDump)
    out.swCachedPrivate = Object.entries(out.caches).filter(([k, v]) => k.startsWith('brain-') && v.includes('/me.json')).map(([k]) => k)
    out.dataCacheHasItems = (out.caches['brain:data'] ?? []).includes('/api/items')
    out.swCachedApi = Object.entries(out.caches).filter(([k, v]) => k.startsWith('brain-') && v.includes('/api/items')).map(([k]) => k)
    await p.close()
    // next visit: does the stored answer show before the 1.5 s API answers?
    const p2 = await browser.newPage({ mobile: false })
    const t0 = Date.now()
    await p2.goto(base + '/')
    const seen = await p2.evaluate(TIME_TO_ITEMS, 15000)
    out.nextVisitMsToItems = seen
    out.nextVisitApiHits = server.log().filter((e) => e.path === '/api/items' && e.t >= t0).length
    await p2.close()
  } finally { await server.close(); await browser.close() }
  return out
}

async function optOutOffline(dist) {
  const app = { name: 'optout-offline', routes: ['/', '/about'] }
  const r = await measureRun(app, dist, 'offline-false')
  return { offline: r.offline, unvisited: r.offlineUnvisited, swState: r.first.sw }
}

async function swOffReachesPastVisitors(defaultDist, offDist) {
  const server = await startStatic({ api: API }); server.setRoot(defaultDist)
  const base = `http://127.0.0.1:${server.port}`
  const browser = await launchBrowser(join(SCRATCH, 'profiles', 'sw-off-past'))
  const out = {}
  try {
    const p = await browser.newPage({ mobile: false })
    await p.goto(base + '/')
    out.before = { sw: await p.evaluate(SW_READY, 20000), ...(await p.evaluate(STATE)) }
    server.setRoot(offDist) // deploy the sw:false build on the same origin
    await p.goto(base + '/')
    await sleep(4000)
    out.after1 = await p.evaluate(STATE)
    await p.goto(base + '/about')
    await sleep(2000)
    out.after2 = await p.evaluate(STATE)
    out.pass = out.before.regs === 1 && out.after2.regs === 0 && !out.after2.caches.some((c) => c.startsWith('brain-'))
  } finally { await server.close(); await browser.close() }
  return out
}

async function swOffFresh(offDist) {
  const server = await startStatic(); server.setRoot(offDist)
  const browser = await launchBrowser(join(SCRATCH, 'profiles', 'sw-off-fresh'))
  try {
    const p = await browser.newPage({ mobile: false })
    await p.goto(`http://127.0.0.1:${server.port}/`)
    await sleep(3000)
    return await p.evaluate(STATE)
  } finally { await server.close(); await browser.close() }
}

async function optOuts(dataApp) {
  const base = join(SCRATCH, 'apps')
  const variants = {}
  for (const [k, cache] of Object.entries({ default: null, sw: { sw: false }, http: { http: false }, offline: { offline: false }, data: { data: false } })) {
    const dir = join(base, `data-opt-${k}`)
    rmSync(dir, { recursive: true, force: true })
    cpSync(dataApp.dir, dir, { recursive: true })
    rmSync(join(dir, 'brain.json'), { force: true })
    variants[k] = { dir, cache, build: build(dir, cache), files: headersOf(dir) }
  }
  const dist = (k) => join(variants[k].dir, 'dist')
  // evidence: the prerender of a plain resource(itemsApi.get) and the public/ URL inside client chunks
  {
    const dir = join(base, 'data-plain')
    rmSync(dir, { recursive: true, force: true })
    cpSync(dataApp.dir, dir, { recursive: true })
    writeFileSync(join(dir, 'views', 'home.brain'), PLAIN_HOME)
    build(dir, null)
    variants.plain = { dir }
  }
  const r = { variants: Object.fromEntries(Object.entries(variants).filter(([k]) => k !== 'plain').map(([k, v]) => [k, { cache: v.cache, files: v.files, build: v.build.log }])) }
  r.prerenderedPlainResource = /<main[\s\S]*?<\/main>/.exec(readFileSync(join(dist('plain'), 'index.html'), 'utf8'))?.[0]
  r.chunksWithUnhashedLogo = readdirSync(join(dist('default'), '_b')).filter((f) => f.endsWith('.js') && readFileSync(join(dist('default'), '_b', f), 'utf8').includes('/logo.svg'))
  r.sameBuildDefaultVsDataFalse = readFileSync(join(dist('default'), 'sw.js'), 'utf8').split('\n')[0] === readFileSync(join(dist('data'), 'sw.js'), 'utf8').split('\n')[0]
  r.headers = {}
  for (const k of Object.keys(variants)) r.headers[k] = await headerProbe(dist(k))
  r.sw = { fresh: await swOffFresh(dist('sw')), pastVisitor: await swOffReachesPastVisitors(dist('default'), dist('sw')), snippetInHtml: r.headers.sw.registerSnippet, killWorker: r.headers.sw.swIsKill }
  r.sw.pass = r.sw.fresh.regs === 0 && !r.sw.snippetInHtml && r.sw.killWorker && r.sw.pastVisitor.pass
  const h = r.headers.http
  r.http = {
    files: variants.http.files,
    immutable: Object.values(h).some((x) => x && typeof x === 'object' && /immutable/.test(x.cacheControl ?? '')),
    anyCacheControl: Object.values(h).filter((x) => x && typeof x === 'object' && x.cacheControl).map((x) => `${x.path}: ${x.cacheControl}`),
    publicHashed: (h.img?.path ?? '').startsWith('/_b/'),
    note: 'ETag comes from the emulated static host (Netlify/Cloudflare Pages add one to every file), not from the build',
  }
  // http:false turns off the CACHING headers only; _headers/nginx may still carry security headers (CSP, HSTS…).
  r.http.pass = !r.http.immutable && r.http.anyCacheControl.length === 0
  r.offline = await optOutOffline(dist('offline'))
  r.offline.offlineBuilt = ['_offline.html', '_offline'].filter((f) => existsSync(join(dist('offline'), f)))
  r.offline.pass = r.offline.offline.every((o) => o.errorPage) && r.offline.unvisited.errorPage && r.offline.offlineBuilt.length === 0
  r.data = { default: await dataProbe(dist('default')), off: await dataProbe(dist('data')) }
  const d = r.data
  r.data.metaEmitted = /<meta name="brain-data-cache" content="off">/.test(readFileSync(join(dist('data'), 'index.html'), 'utf8'))
  r.data.b1102Build = /B1102/.test(variants.data.build.full)
  r.data.b1102Check = /"code":"B1102"/.test(sh(BRAIN, ['check', '--json'], variants.data.dir).out)
  r.data.b1102 = r.data.b1102Check
  r.data.layerOff = d.off.memory?.msToItemsOnBack >= API_DELAY * 0.8 && !d.off.dataCacheHasItems && d.off.nextVisitMsToItems >= API_DELAY * 0.8
  r.data.pass = d.default.memory?.msToItemsOnBack != null && d.off.memory?.msToItemsOnBack != null && d.off.memory.msToItemsOnBack >= API_DELAY * 0.8 && !d.off.dataCacheHasItems && d.off.nextVisitMsToItems >= API_DELAY * 0.8
  r.data.toolchainParsesKey = /"data"/.test(readFileSync(join(REPO, 'crates', 'brain-cli', 'src', 'main.rs'), 'utf8').match(/fn cache_cfg[\s\S]*?\n}\n/)?.[0] ?? '')
  return r
}

// ---------- report ----------
function verdict(sum) {
  const V = [], P = (x) => (x ? 'PASS' : 'FAIL'), o = sum.optOuts, g = sum.signature
  if (sum.apps.length) {
    V.push(['1. cold visit downloads precached chunks once', P(sum.apps.every((a) => a.dupBytes === 0)), sum.apps.map((a) => `${a.app} ${a.dupBytes} B duplicated`).join(' · ')])
  }
  if (o) {
    const api = [...o.data.default.swCachedApi, ...o.data.off.swCachedApi], priv = [...(o.data.default.swCachedPrivate ?? []), ...(o.data.off.swCachedPrivate ?? [])]
    V.push(['2. worker never stores /api/* (no-store) nor private responses', P(!api.length && !priv.length), `worker caches holding /api/items: ${JSON.stringify(api)} · holding /me.json (private): ${JSON.stringify(priv)}`])
    V.push(['3. offline:false → browser error page offline, no _offline built', P(o.offline.pass), `${o.offline.offline.map((x) => `${x.route}: ${x.errorPage ? 'error page' : 'rendered "' + x.h1 + '"'}`).join(' · ')} · unvisited: ${o.offline.unvisited.errorPage ? 'error page' : '"' + o.offline.unvisited.h1 + '"'} · _offline in dist: ${JSON.stringify(o.offline.offlineBuilt)}`])
    V.push(['4a. data:false: build emits <meta name="brain-data-cache" content="off">', P(o.data.metaEmitted), `meta in dist/index.html: ${o.data.metaEmitted}`])
    V.push(['4b. data:false: B1102 warning for api() cache:', P(o.data.b1102), `\`brain check --json\` reports B1102: ${o.data.b1102Check} · \`brain build\` prints it: ${o.data.b1102Build}`])
    V.push(['4c. data:false: layer really off', P(o.data.layerOff), `items after router back-nav ${Math.round(o.data.off.memory?.msToItemsOnBack ?? -1)} ms (default ${Math.round(o.data.default.memory?.msToItemsOnBack ?? -1)} ms), next visit ${o.data.off.nextVisitMsToItems} ms (default ${o.data.default.nextVisitMsToItems} ms), API ${API_DELAY} ms · brain:data holds /api/items: ${o.data.off.dataCacheHasItems} (default ${o.data.default.dataCacheHasItems})`])
  }
  if (g) {
    const c = g.chunk, h = g.health
    if (c.blocked) V.push(['5. deleted chunk → exactly 1 document load, no loop, island alive', 'BLOCKED', c.blocked])
    else V.push(['5. deleted chunk → exactly 1 document load, no loop, island alive', P(c.pass), `document loads ${c.fullLoads} (${c.navs.join(' → ')}) · island adds a task: ${c.islandAlive} · final build ${c.final.build} = B ${c.sigB}${c.adapted ? ` · adapted: ${c.adapted}` : ''}`])
    if (h.blocked) V.push(['6. server.mjs: /sw.js no-cache, static HTML ETag = signature', 'BLOCKED', h.blocked])
    else V.push(['6. server.mjs: /sw.js no-cache, static HTML ETag = signature', P(h.swNoCache && h.staticEtagIsSignature), `/sw.js ${h.pages['/sw.js'].cacheControl} · ETag of / ${h.pages['/'].etag} vs signature ${h.printed}`])
  }
  if (sum.apps.length) {
    const ok = sum.apps.every((a) => a.pass.zeroJsCss && a.pass.lcpLower && a.pass.offline)
    V.push(['repeat visit: 0 B JS/CSS, LCP lower, visited routes offline', P(ok), sum.apps.map((a) => `${a.app} ${a.second.jsCss200Bytes} B, LCP ${Math.round(a.first.lcp)}→${Math.round(a.second.lcp)} ms, offline ${a.offline.filter((x) => x.ok).length}/${a.offline.length}`).join(' · ')])
  }
  if (o) {
    V.push(['prerender of a plain resource() does not freeze the error', P(!/role="alert"/.test(o.prerenderedPlainResource ?? '')), (o.prerenderedPlainResource ?? '').replace(/\|/g, '\\|').slice(0, 140)])
    V.push(['public/ paths inside JS are hashed', P(!o.chunksWithUnhashedLogo.length && (o.data.default.memory?.clientImgSrc ?? '').startsWith('/_b/')), `client-rendered img src ${o.data.default.memory?.clientImgSrc} · chunks with /logo.svg: ${JSON.stringify(o.chunksWithUnhashedLogo)}`])
  }
  if (sum.manifest) V.push(['manifest icons point at files that exist', P(sum.manifest.missing.length === 0), `fresh app manifest icons: ${JSON.stringify(sum.manifest.icons)} · missing: ${JSON.stringify(sum.manifest.missing)}`])
  return V
}

function md(sum) {
  const L = []
  const kb = (b) => (b == null ? '—' : (b / 1024).toFixed(1) + ' KB')
  const ms = (x) => (x == null ? '—' : Math.round(x) + ' ms')
  L.push(`# Cache bench results (§35)`, '', `binary \`${sum.binary.sha12}\` (${sum.binary.version}) · ${sum.date} · Chrome ${sum.chrome} · runs per app: ${sum.runs} (medians)`, '')
  L.push(`Throttling: ${sum.throttling}`, '')
  const V = verdict(sum)
  if (V.length) { L.push('## Verdict', '', '| item | result | evidence |', '|---|---|---|'); for (const [a, b, c] of V) L.push(`| ${a} | ${b} | ${c} |`); L.push('') }
  L.push('| app | first bytes / requests / LCP | second bytes / requests / LCP | 2nd JS/CSS bytes | offline routes OK | notes |', '|---|---|---|---|---|---|')
  for (const a of sum.apps) {
    L.push(`| ${a.app} | ${kb(a.first.bytes)} / ${a.first.requests} / ${ms(a.first.lcp)} | ${kb(a.second.bytes)} / ${a.second.requests} / ${ms(a.second.lcp)} | ${a.second.jsCss200Bytes} | ${a.offline.filter((o) => o.ok).length}/${a.offline.length} (${a.offline.map((o) => o.route + (o.ok ? ' ✓' : ' ✗')).join(' ')}) | ${a.notes.join('; ') || 'second visit: ' + a.secondRequestsSeen.join(', ')} |`)
  }
  if (sum.dev) {
    L.push('', '## `brain run` registers no service worker', '', '| check | result |', '|---|---|')
    L.push(`| fresh profile on \`brain run\`: registrations | ${sum.dev.fresh.regs} (${sum.dev.fresh.pass ? 'PASS' : 'FAIL'}) |`)
    L.push(`| production worker on the same origin, then \`brain run\`: registrations before → after 1st load → after 2nd load | ${sum.dev.before.regs} → ${sum.dev.after1.regs} → ${sum.dev.after2.regs}; brain-* caches after: ${JSON.stringify(sum.dev.after2.caches)} (${sum.dev.pass ? 'PASS' : 'FAIL'}) |`)
  }
  if (sum.optOuts) {
    const o = sum.optOuts
    L.push('', '## Opt-outs (`brain.json` `cache.*`, app `data`)', '', '| switch | expected | observed | result |', '|---|---|---|---|')
    L.push(`| \`sw: false\` | no registration snippet, kill worker, fresh visitor never registers, past visitor's worker removed | snippet ${o.sw.snippetInHtml ? 'present' : 'absent'} · sw.js kill worker: ${o.sw.killWorker} · fresh regs ${o.sw.fresh.regs} · past visitor regs ${o.sw.pastVisitor.before.regs} → ${o.sw.pastVisitor.after1.regs} → ${o.sw.pastVisitor.after2.regs}, caches after ${JSON.stringify(o.sw.pastVisitor.after2.caches)} | ${o.sw.pass ? 'PASS' : 'FAIL'} |`)
    L.push(`| \`http: false\` | no caching headers (no immutable, no Cache-Control); security headers may stay | _headers ${o.http.files._headers} · nginx ${o.http.files.nginx} · immutable ${o.http.immutable} · Cache-Control seen: ${o.http.anyCacheControl.join('; ') || 'none'} · public/ hashed: ${o.http.publicHashed} | ${(!o.http.immutable && o.http.anyCacheControl.length === 0) ? 'PASS' : 'FAIL'} |`)
    L.push(`| \`offline: false\` | browser error page offline (no cached pages, no offline page), no _offline built |  ${o.offline.offline.map((x) => `${x.route}: ${x.ok ? 'rendered "' + x.h1 + '"' : x.errorPage ? 'error page' : 'h1 ' + JSON.stringify(x.h1)}`).join(' · ')} · unvisited: ${o.offline.unvisited.errorPage ? 'error page' : 'h1 ' + JSON.stringify(o.offline.unvisited.h1)} · _offline in dist: ${JSON.stringify(o.offline.offlineBuilt ?? [])} | ${o.offline.pass ? 'PASS' : 'FAIL'} |`)
    L.push(`| \`data: false\` | resource() memory cache and Cache API data cache off | items after router back-nav: default ${ms(o.data.default.memory?.msToItemsOnBack)} vs off ${ms(o.data.off.memory?.msToItemsOnBack)} · items on next visit (cache: '10m'): default ${ms(o.data.default.nextVisitMsToItems)} vs off ${ms(o.data.off.nextVisitMsToItems)} (API takes ${API_DELAY} ms) · brain:data has /api/items: default ${o.data.default.dataCacheHasItems} vs off ${o.data.off.dataCacheHasItems} · toolchain parses cache.data: ${o.data.toolchainParsesKey} | ${o.data.pass ? 'PASS' : 'FAIL'} |`)
    L.push(`| (evidence) data:false changes the build | different output | sw.js manifest identical to the default build: ${o.sameBuildDefaultVsDataFalse} | ${o.sameBuildDefaultVsDataFalse ? 'FAIL' : 'PASS'} |`)
    L.push(`| (side finding) prerender of a plain \`resource(api.get)\` | loading state or data in the page | \`${(o.prerenderedPlainResource ?? '').replace(/\|/g, '\\|').slice(0, 160)}…\` | ${/role="alert"/.test(o.prerenderedPlainResource ?? '') ? 'FAIL' : 'PASS'} |`)
    L.push(`| (side finding) hashed public/ URL in client-rendered markup | /_b/<hash>.svg | client render uses \`${o.data.default.memory?.clientImgSrc}\`; chunks with /logo.svg: ${o.chunksWithUnhashedLogo.join(', ') || 'none'} | ${o.chunksWithUnhashedLogo.length ? 'FAIL' : 'PASS'} |`)
    L.push(`| (side finding) worker caches API JSON | /api/* (no-store) never stored by the worker | build caches holding /api/items: default ${JSON.stringify(o.data.default.swCachedApi)} · data:false ${JSON.stringify(o.data.off.swCachedApi)} | ${o.data.default.swCachedApi.length ? 'FAIL' : 'PASS'} |`)
  }
  if (sum.signature) {
    const g = sum.signature, P = (x) => (x ? 'PASS' : 'FAIL')
    const i = g.identical, h = g.health, d = g.deploy, c = g.chunk
    L.push('', '## Build signature (§35.7)', '', '| case | result | evidence |', '|---|---|---|')
    const blocked = [['1. deploy A→B with an open tab', d], ['2. deleted chunk → one full reload, no loop, island alive', c], ['3. identical rebuild / signature', i], ['4. /_health and server.mjs headers', h]].filter(([, x]) => x.blocked)
    for (const [name, x] of blocked) L.push(`| ${name} | BLOCKED | ${String(x.blocked).replace(/\|/g, '\\|')} |`)
    if (!d.blocked) L.push(`| 1. deploy A→B with an open tab | ${P(d.pass)} | A \`${d.sigA}\` → B \`${d.sigB}\` · tab events after the deploy: ${d.navs.join(' → ')} · /about shows "About B" after ${d.aboutMs} ms, / shows "My tasks B" · final page build \`${d.afterHome.build}\` · A-only files returned: ${d.oldFiles.length} · responses carrying build A: ${d.oldHtml.length} · caches: ${JSON.stringify(d.afterHome.caches)} |`)
    if (!c.blocked) L.push(`| 2. deleted chunk → one full reload, no loop, island alive | ${P(c.pass)} | tab on A \`${c.sigA}\`, B \`${c.sigB}\` deployed, A's island chunk \`${c.chunk}\` gone (server, ${c.removedFromCaches} worker cache, HTTP cache) · requests for it: ${JSON.stringify(c.chunkRequests)} · tab events: ${c.navs.join(' → ') || 'none'} (document loads: ${c.fullLoads}) · page after: build \`${c.final.build}\`, h1 "${c.final.h1}" · island adds a task: ${c.islandAlive} · guard keys ${JSON.stringify(c.reloadKeys)}${c.adapted ? ` · adapted: ${c.adapted}` : ''} |`)
    if (!i.blocked) L.push(`| 3a. identical rebuild keeps the signature | ${P(i.identicalPass)} | printed ${i.printed.map((x) => '\`' + x + '\`').join(', ')} (same dir twice, then a fresh copy) · files that differ: ${i.sameDirDiff.length} / ${i.otherDirDiff.length} |`)
    if (!i.blocked) L.push(`| 3b. the signature is the same everywhere | ${P(i.placesAgree)} | meta \`${i.places.meta}\` · sw.js \`${i.places.sw}\` · _headers X-Brain-Build ${i.places.headers.map((x) => '\`' + x + '\`').join(',')} · nginx \`${i.places.nginx}\` |`)
    if (!i.blocked) L.push(`| 3c. one changed source byte changes it | ${P(Object.values(i.mutations).every((m) => m.changed))} | ${Object.entries(i.mutations).map(([k, m]) => `${k} → \`${m.printed}\` (${m.filesChanged} files)`).join(' · ')} |`)
    if (!h.blocked) L.push(`| 4. /_health = printed signature | ${P(h.healthPass)} | served by ${h.servedBy}: printed \`${h.printed}\`, /_health ${JSON.stringify(h.health)}, X-Brain-Build on / and /about: \`${h.pages['/'].xBrainBuild}\`/\`${h.pages['/about'].xBrainBuild}\` · \`brain start\`: ${h.brainStart} |`)
    if (!h.blocked) L.push(`| (side) server.mjs headers | ${P(h.swNoCache && h.staticEtagIsSignature)} | /sw.js Cache-Control \`${h.pages['/sw.js'].cacheControl}\` (§35.7: no-cache) · static HTML ETag ${h.pages['/'].etag} (§35.7: the signature) |`)
  }
  return L.join('\n') + '\n'
}

async function main() {
  if (process.argv.includes('--report-only')) {
    const sum = JSON.parse(readFileSync(join(OUT, 'summary.json'), 'utf8'))
    writeFileSync(join(OUT, 'summary.md'), md(sum))
    return console.log(md(sum))
  }
  mkdirSync(RAW, { recursive: true })
  mkdirSync(SCRATCH, { recursive: true })
  rmSync(BRAIN, { force: true }) // macOS kills a binary copied over a previous copy of itself
  cpSync(BRAIN_SRC, BRAIN)
  const bin = readFileSync(BRAIN)
  const sum = {
    date: new Date().toISOString(), runs: RUNS,
    binary: { path: relative(REPO, BRAIN_SRC), sha12: createHash('sha256').update(bin).digest('hex').slice(0, 12), version: sh(BRAIN, ['--version']).out.trim() },
    chrome: sh(process.env.BRAIN_CHROME ?? '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome', ['--version']).out.trim(),
    throttling: `Network.emulateNetworkConditions latency ${SLOW_4G.latency} ms, down ${(SLOW_4G.downloadThroughput * 8 / 1024 / 1024).toFixed(2)} Mbps, up ${(SLOW_4G.uploadThroughput * 8 / 1024).toFixed(0)} Kbps on the page and every service-worker target; CPU ${CPU_RATE}x; 412×915 mobile viewport`,
    apps: [],
  }
  console.log(`brain ${sum.binary.sha12} · ${sum.chrome}`)
  let apps = prepare()
  for (const a of apps) { const b = build(a.dir); a.routes = b.routes; a.buildLog = b.log }
  const data = apps.find((a) => a.name === 'data')
  const fresh = apps.find((a) => a.name === 'fresh')
  {
    const mf = join(fresh.dir, 'dist', 'manifest.webmanifest')
    const icons = existsSync(mf) ? (JSON.parse(readFileSync(mf, 'utf8')).icons ?? []).map((i) => i.src) : []
    sum.manifest = { icons, missing: icons.filter((u) => !existsSync(join(fresh.dir, 'dist', new URL(u, 'http://x/').pathname.slice(1)))) }
  }
  if (ONLY) apps = apps.filter((a) => ONLY.includes(a.name))
  for (const a of apps) {
    const runs = []
    for (let i = 0; i < RUNS; i++) {
      process.stdout.write(`${a.name} run ${i + 1}/${RUNS}… `)
      const r = await measureRun(a, join(a.dir, 'dist'), i)
      runs.push(r)
      console.log(`first ${r.first.server.bytes} B/${r.first.server.requests} req/LCP ${Math.round(r.first.state.lcp)} · second ${r.second.server.bytes} B/${r.second.server.requests} req/LCP ${Math.round(r.second.state.lcp)} · offline ${r.offline.filter((o) => o.ok).length}/${r.offline.length}`)
    }
    writeFileSync(join(RAW, `${a.name}.json`), JSON.stringify(runs, null, 2))
    sum.apps.push(aggregate(a, runs))
  }
  if (!process.argv.includes('--skip-dev')) { console.log('dev (brain run)…'); sum.dev = await devTest(fresh); writeFileSync(join(RAW, 'dev.json'), JSON.stringify(sum.dev, null, 2)) }
  if (!process.argv.includes('--skip-optouts')) { console.log('opt-outs…'); sum.optOuts = await optOuts(data); writeFileSync(join(RAW, 'optouts.json'), JSON.stringify(sum.optOuts, null, 2)) }
  if (!process.argv.includes('--skip-signature')) {
    console.log('build signature (§35.7)…')
    sum.signature = await runSignature({ brain: BRAIN, scratch: SCRATCH, tasksApp: join(SRC_APPS, 'tasks'), freshApp: fresh.dir })
    writeFileSync(join(RAW, 'signature.json'), JSON.stringify(sum.signature, null, 2))
  }
  writeFileSync(join(OUT, 'summary.json'), JSON.stringify(sum, null, 2))
  writeFileSync(join(OUT, 'summary.md'), md(sum))
  console.log(md(sum))
}

main().catch((e) => { console.error(e); process.exit(1) })
