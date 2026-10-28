// Build signature cases of INTERNALS §35.7 (QA #124 after TOOLCHAIN #130):
// 1. deploy A→B under an open tab · 2. a deleted chunk reloads once, never in a loop · 3. identical rebuild keeps the
// signature, one changed source byte changes it · 4. /_health returns the signature `brain build` printed.
import { spawn, execFileSync } from 'node:child_process'
import { cpSync, rmSync, mkdirSync, writeFileSync, readFileSync, existsSync, readdirSync, statSync } from 'node:fs'
import { join, relative } from 'node:path'
import { createHash } from 'node:crypto'
import { startStatic } from './static.mjs'
import { launchBrowser, sleep } from './browser.mjs'

const sha = (b) => createHash('sha256').update(b).digest('hex').slice(0, 16)

export function buildSig(brain, dir) {
  rmSync(join(dir, 'dist'), { recursive: true, force: true })
  let out
  try { out = execFileSync(brain, ['build'], { cwd: dir, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] }) }
  catch (e) { throw new Error(`brain build failed in ${dir}:\n${e.stdout}${e.stderr}`) }
  const last = out.trim().split('\n').pop()
  const m = /^build ([a-z0-9]+)$/.exec(last)
  return { printed: m ? m[1] : null, lastLine: last }
}

function fileHashes(dist) {
  const out = {}
  const walk = (d) => { for (const f of readdirSync(d)) { const p = join(d, f); statSync(p).isDirectory() ? walk(p) : (out[relative(dist, p)] = sha(readFileSync(p))) } }
  walk(dist)
  return out
}

// every place §35.7 puts the signature, read back from dist/
function signaturePlaces(dist) {
  const html = readFileSync(join(dist, 'index.html'), 'utf8')
  const sw = existsSync(join(dist, 'sw.js')) ? readFileSync(join(dist, 'sw.js'), 'utf8') : ''
  const hdr = existsSync(join(dist, '_headers')) ? readFileSync(join(dist, '_headers'), 'utf8') : ''
  return {
    meta: /<meta name="brain-build" content="([^"]+)">/.exec(html)?.[1] ?? null,
    sw: /BRAIN_BUILD=\{hash:"([^"]+)"/.exec(sw)?.[1] ?? null,
    headers: [...new Set([...hdr.matchAll(/X-Brain-Build: (\S+)/g)].map((m) => m[1]))],
    nginx: existsSync(join(dist, '_brain', 'nginx.conf')) ? /BRAINLLM build (\S+):/.exec(readFileSync(join(dist, '_brain', 'nginx.conf'), 'utf8'))?.[1] ?? null : null,
  }
}

const edit = (f, from, to) => {
  const s = readFileSync(f, 'utf8')
  if (!s.includes(from)) throw new Error(`${f}: "${from}" not found`)
  writeFileSync(f, s.replace(from, to))
}

const STATE = `(async()=>({url:location.href,h1:document.querySelector('h1')?.textContent?.trim()??null,
  build:document.querySelector('meta[name=brain-build]')?.content??null,
  regs:(await navigator.serviceWorker.getRegistrations()).length,controller:navigator.serviceWorker.controller?.scriptURL??null,
  caches:await caches.keys()}))()`
const SW_READY = `(async()=>{const t=Date.now();while(Date.now()-t<15000){const r=await navigator.serviceWorker.getRegistration();if(r&&r.active&&r.active.state==='activated'&&navigator.serviceWorker.controller)return 'activated+controlling';await new Promise(r=>setTimeout(r,100))}return 'timeout'})()`
// a link click the router intercepts, whether or not the page has a link to that route
const follow = (to) => `(()=>{const a=document.createElement('a');a.href=${JSON.stringify(to)};document.body.append(a);a.click();return true})()`
const waitH1 = (text, ms = 15000) => `(async()=>{const t=Date.now();while(Date.now()-t<${ms}){if(document.querySelector('h1')?.textContent?.trim()===${JSON.stringify(text)})return Date.now()-t;await new Promise(r=>setTimeout(r,50))}return null})()`

// polled from Node: a full load in the middle would destroy an in-page wait
async function pollH1(p, text, ms = 20000) {
  const t = Date.now()
  while (Date.now() - t < ms) {
    const h = await p.evaluate(`document.querySelector('h1')?.textContent?.trim()??null`).catch(() => null)
    if (h === text) return Date.now() - t
    await sleep(50)
  }
  return null
}

// Case 1: tab on build A, deploy B on the same origin, navigate in the tab.
async function deployAB(brain, scratch, srcApp) {
  const A = join(scratch, 'sig', 'deploy-a'), B = join(scratch, 'sig', 'deploy-b')
  for (const d of [A, B]) { rmSync(d, { recursive: true, force: true }); cpSync(srcApp, d, { recursive: true }); rmSync(join(d, 'brain.json'), { force: true }) }
  edit(join(B, 'views', 'home.brain'), '<h1>My tasks</h1>', '<h1>My tasks B</h1>')
  edit(join(B, 'views', 'about.brain'), '<h1>About</h1>', '<h1>About B</h1>')
  const a = buildSig(brain, A), b = buildSig(brain, B)
  const filesA = new Set(Object.keys(fileHashes(join(A, 'dist')))), filesB = new Set(Object.keys(fileHashes(join(B, 'dist'))))
  const cls = (u) => {
    const p = new URL(u).pathname.replace(/^\//, '')
    const f = p === '' || p.endsWith('/') ? p + 'index.html' : filesA.has(p) || filesB.has(p) ? p : p + '/index.html'
    return filesA.has(f) && !filesB.has(f) ? 'A-only' : filesB.has(f) && !filesA.has(f) ? 'B-only' : filesA.has(f) ? 'both' : 'unknown'
  }
  const server = await startStatic(); server.setRoot(join(A, 'dist'), 'A')
  const base = `http://127.0.0.1:${server.port}`
  const browser = await launchBrowser(join(scratch, 'profiles', 'sig-deploy'))
  const out = { sigA: a.printed, sigB: b.printed }
  try {
    const p = await browser.newPage({ mobile: false })
    await p.goto(base + '/')
    out.swBefore = await p.evaluate(SW_READY, 20000)
    out.before = await p.evaluate(STATE)
    await sleep(500)
    // deploy B
    server.setRoot(join(B, 'dist'), 'B')
    const tDeploy = Date.now(), mark = server.mark()
    p.resetLedger()
    // next navigation: the router link to /about
    await p.evaluate(follow('/about'))
    out.aboutMs = await pollH1(p, 'About B')
    await sleep(1500)
    out.afterAbout = await p.evaluate(STATE).catch((e) => ({ error: e.message }))
    // and back to /
    await p.evaluate(follow('/')).catch(() => {})
    out.homeMs = await pollH1(p, 'My tasks B')
    await sleep(3000) // let the new worker install, activate and announce itself
    out.afterHome = await p.evaluate(STATE).catch((e) => ({ error: e.message }))
    out.navs = p.navs().filter((n) => n.t >= tDeploy).map((n) => `${n.kind}${n.url ? ' ' + new URL(n.url).pathname : ''}`)
    out.requests = p.ledger().map((r) => ({ url: new URL(r.url).pathname, status: r.status, source: r.fromSW ? 'sw' : r.fromDisk || r.fromMemory ? 'http-cache' : r.failed ? 'failed' : 'network', build: r.build, file: cls(r.url) }))
    out.server = server.since(mark).map((e) => `${e.path} ${e.status} (served from ${e.root})`)
    out.swRequests = browser.swLedger.filter((r) => r.t >= tDeploy).map((r) => ({ url: new URL(r.url).pathname, status: r.status, file: cls(r.url) }))
    out.oldFiles = out.requests.filter((r) => r.file === 'A-only' && r.status >= 200 && r.status < 400)
    out.oldHtml = out.requests.filter((r) => r.build === out.sigA)
    out.pass = !!out.aboutMs && !!out.homeMs && out.afterHome.build === out.sigB && out.oldFiles.length === 0 && out.oldHtml.length === 0 && (out.afterHome.caches || []).every((c) => !c.startsWith('brain-') || c === 'brain-' + out.sigB || c === 'brain-' + out.sigA)
  } finally { await server.close(); await browser.close() }
  return out
}

// Case 2: a deploy deletes a chunk an open tab has not imported yet. The tab is on `/about` of A, follows a router
// link to `/` (A's page is swapped in, its island chunk is imported only on interaction), then B is deployed: A's
// island chunk is gone from the server, from every worker cache and from the HTTP cache. Using the island must cost
// exactly one full document load, never a loop, and leave a working island (B's).
// The tasks app's `/` reads the tasks channel (a live module), so the build mounts it on load and ships no island
// (INTERNALS §6.1; brain-pack: live views are not islands). The criterion needs an island, so when `/` has none the
// case runs on a copy of the app whose `/` is the same task form with local state (an island). A missing island after
// that is recorded as BLOCKED with the reason, never thrown.
const ISLAND_HOME = `<meta seo title="Tasks" description="A small task manager built with BRAINLLM." canonical="/">
<script>
  let draft = sig('')
  let items = sig([])
  const add = () => {
    const text = draft.trim()
    if (text === '') return
    items = [...items, { id: Date.now(), text }]
    draft = ''
  }
</script>
<main class="home">
  <h1>My tasks</h1>
  <form on:submit={add}>
    <label for="nt">New task</label>
    <input id="nt" bind:value={draft} placeholder="Write something" />
    <button type="submit">Add</button>
  </form>
  <ul>
    {#each items as t (t.id)}<li>{t.text}</li>{:empty}<li>No tasks yet.</li>{/each}
  </ul>
  <a href="/about">About</a>
</main>
<style>
  .home { display: grid; gap: var(--space-4); padding: var(--space-5); max-width: 640px; margin: 0 auto; }
</style>
`
const islandChunk = (dir) => /type="b\/state">\{"c":"(\/_b\/[a-z0-9]+\.js)"/.exec(readFileSync(join(dir, 'dist', 'index.html'), 'utf8'))?.[1]

export async function deletedChunk(brain, scratch, srcApp) {
  const A = join(scratch, 'sig', 'chunk-a'), B = join(scratch, 'sig', 'chunk-b')
  const prepare = (island) => {
    for (const d of [A, B]) {
      rmSync(d, { recursive: true, force: true }); cpSync(srcApp, d, { recursive: true }); rmSync(join(d, 'brain.json'), { force: true })
      if (island) writeFileSync(join(d, 'views', 'home.brain'), ISLAND_HOME)
    }
    edit(join(B, 'views', 'home.brain'), '<h1>My tasks</h1>', '<h1>My tasks B</h1>')
    return [buildSig(brain, A), buildSig(brain, B)]
  }
  let [a, b] = prepare(false)
  let chunk = islandChunk(A), adapted = null
  if (!chunk) {
    adapted = "tasks `/` has no island (it reads the tasks channel, so the build mounts it on load); the case runs on a copy whose `/` is the same task form with local state"
    ;[a, b] = prepare(true)
    chunk = islandChunk(A)
  }
  const out = { sigA: a.printed, sigB: b.printed, chunk, adapted, chunkInB: chunk ? existsSync(join(B, 'dist', chunk.slice(1))) : null, navs: [], final: {} }
  if (!chunk) return { ...out, pass: false, blocked: 'precondition not met: no island chunk (`<script type="b/state">`) on / of the tasks app, nor of the local-state copy' }
  const server = await startStatic(); server.setRoot(join(A, 'dist'), 'A')
  const base = `http://127.0.0.1:${server.port}`
  const browser = await launchBrowser(join(scratch, 'profiles', 'sig-chunk'))
  const m0 = server.mark()
  try {
    const p = await browser.newPage({ mobile: false })
    await p.goto(base + '/about')
    out.sw = await p.evaluate(SW_READY, 20000)
    await p.evaluate(follow('/'))
    out.homeMs = await pollH1(p, 'My tasks')
    // precondition: the tab has not fetched the island chunk yet (else deleting it tests nothing)
    out.chunkFetchedBeforeDeploy = server.since(m0).filter((e) => e.path === chunk).map((e) => `${e.status} (served from ${e.root})`)
    out.preloaded = [...readFileSync(join(A, 'dist', 'index.html'), 'utf8').matchAll(/<link[^>]*rel="modulepreload"[^>]*href="([^"]+)"/g)].some((m) => m[1] === chunk)
    // deploy B: A's chunk is gone everywhere
    server.setRoot(join(B, 'dist'), 'B')
    out.removedFromCaches = await p.evaluate(`(async()=>{let n=0;for(const k of await caches.keys()){const c=await caches.open(k);if(await c.delete(${JSON.stringify(chunk)}))n++}return n})()`)
    await p.send('Network.clearBrowserCache')
    const t0 = Date.now(), mark = server.mark()
    await p.evaluate(`(self.__navTypes=[],self.navigation&&navigation.addEventListener('navigate',e=>__navTypes.push(e.navigationType+' '+new URL(e.destination.url).pathname)),1)`)
    // use the island (typing is a real interaction: it imports and resumes the component)
    const use = (text) => `(()=>{const i=document.querySelector('main input');if(!i)return false;i.focus();i.value=${JSON.stringify(text)};i.dispatchEvent(new InputEvent('input',{bubbles:true}));return true})()`
    out.used1 = await p.evaluate(use('first')).catch((e) => 'navigated: ' + e.message)
    await sleep(5000)
    out.navigationApiEvents = await p.evaluate('self.__navTypes ?? "document replaced"').catch(() => null)
    out.afterReload = await p.evaluate(STATE).catch((e) => ({ error: e.message }))
    // the island after the reload: add a task through the form and look for it in the list
    out.used2 = await p.evaluate(use('alive-check')).catch((e) => 'navigated: ' + e.message)
    await sleep(500)
    out.islandAlive = await p.evaluate(`(async()=>{const f=document.querySelector('main form');if(!f)return 'no form';f.requestSubmit();await new Promise(r=>setTimeout(r,800));return [...document.querySelectorAll('main li')].some(l=>l.textContent.includes('alive-check'))})()`).catch((e) => 'error: ' + e.message)
    await sleep(3000)
    out.final = await p.evaluate(STATE).catch((e) => ({ error: e.message }))
    out.navs = p.navs().filter((n) => n.t >= t0).map((n) => `${n.kind}${n.url ? ' ' + new URL(n.url).pathname : ''}`)
    out.fullLoads = out.navs.filter((n) => n.startsWith('document')).length
    out.chunkRequests = server.since(mark).filter((e) => e.path === chunk).map((e) => `${e.status} (served from ${e.root})`)
    out.reloadKeys = await p.evaluate(`Object.keys(sessionStorage).filter(k=>k.startsWith('brain:reload'))`).catch(() => null)
    out.pass = out.fullLoads === 1 && out.islandAlive === true && out.final.build === out.sigB
    if (!out.chunkRequests.length && out.chunkFetchedBeforeDeploy.some((r) => r.startsWith('2'))) {
      out.pass = false
      out.blocked = `precondition not met: the tab fetched A's island chunk ${chunk} before the deploy (${out.chunkFetchedBeforeDeploy.join(', ')}${out.preloaded ? '; A\'s / lists it in <link rel="modulepreload">' : ''}), so deleting it afterwards cannot be observed: the island resumed from memory (document loads ${out.fullLoads}, island adds a task: ${out.islandAlive})`
    }
  } finally { await server.close(); await browser.close() }
  return out
}

// Case 3: identical rebuild keeps the signature; one changed byte in a .brain, a .cssbrain or a public/ file changes it.
function identical(brain, scratch, freshApp) {
  const base = join(scratch, 'sig', 'ident-base')
  rmSync(base, { recursive: true, force: true }); cpSync(freshApp, base, { recursive: true }); rmSync(join(base, 'brain.json'), { force: true })
  writeFileSync(join(base, 'public', 'notes.txt'), 'hello\n') // an unreferenced public/ file
  const copy = (name) => { const d = join(scratch, 'sig', name); rmSync(d, { recursive: true, force: true }); cpSync(base, d, { recursive: true }); rmSync(join(d, 'dist'), { recursive: true, force: true }); rmSync(join(d, '.brain'), { recursive: true, force: true }); return d }
  const r1 = buildSig(brain, base), h1 = fileHashes(join(base, 'dist')), places = signaturePlaces(join(base, 'dist'))
  const r2 = buildSig(brain, base), h2 = fileHashes(join(base, 'dist'))
  const other = copy('ident-copy'); const r3 = buildSig(brain, other), h3 = fileHashes(join(other, 'dist'))
  const diffFiles = (x, y) => [...new Set([...Object.keys(x), ...Object.keys(y)])].filter((k) => x[k] !== y[k])
  const out = {
    printed: [r1.printed, r2.printed, r3.printed], places,
    sameDirDiff: diffFiles(h1, h2), otherDirDiff: diffFiles(h1, h3),
  }
  out.placesAgree = places.meta === r1.printed && places.sw === r1.printed && places.headers.length === 1 && places.headers[0] === r1.printed && places.nginx === r1.printed
  out.identicalPass = r1.printed && r1.printed === r2.printed && r2.printed === r3.printed && out.sameDirDiff.length === 0 && out.otherDirDiff.length === 0
  const mutations = {
    '.brain (visible text)': (d) => { const f = join(d, 'views', 'home.brain'); const s = readFileSync(f, 'utf8'); const m = /<h1>([^<]*[a-z])/.exec(s); if (!m) throw new Error('no <h1> text in views/home.brain'); writeFileSync(f, s.replace(m[0], m[0].slice(0, -1) + m[0].slice(-1).toUpperCase())) },
    '.cssbrain (one hex digit)': (d) => { const f = join(d, 'tokens.cssbrain'); const s = readFileSync(f, 'utf8'); const m = /--fg: #[0-9a-f]{5}([0-9a-f])/.exec(s); if (!m) throw new Error('no --fg color in tokens.cssbrain'); writeFileSync(f, s.replace(m[0], m[0].slice(0, -1) + (m[1] === '0' ? '1' : '0'))) },
    'public/ referenced (favicon.svg)': (d) => { const f = join(d, 'public', 'favicon.svg'); const s = readFileSync(f); s[s.length - 2] = s[s.length - 2] === 0x20 ? 0x0a : 0x20; writeFileSync(f, s) },
    'public/ unreferenced (notes.txt)': (d) => writeFileSync(join(d, 'public', 'notes.txt'), 'hellO\n'),
  }
  out.mutations = {}
  for (const [k, f] of Object.entries(mutations)) {
    const d = copy('ident-mut')
    f(d)
    const r = buildSig(brain, d)
    out.mutations[k] = { printed: r.printed, changed: r.printed !== r1.printed, filesChanged: diffFiles(h1, fileHashes(join(d, 'dist'))).length }
  }
  out.pass = out.identicalPass && out.placesAgree && Object.values(out.mutations).every((m) => m.changed)
  return out
}

// Case 4: /_health. Server apps: `node dist/server.mjs` (§34: what `brain start` runs as children). Static apps: §34 says
// `brain start` serves them from Rust; recorded whether that command exists.
async function health(brain, scratch) {
  const d = join(scratch, 'sig', 'health-srv')
  rmSync(d, { recursive: true, force: true })
  mkdirSync(join(scratch, 'sig'), { recursive: true })
  execFileSync(brain, ['new', 'health-srv'], { cwd: join(scratch, 'sig'), stdio: 'ignore' })
  writeFileSync(join(d, 'views', 'about.brain'), `<meta seo title="About" description="Server-rendered probe." canonical="/about">
<script>
  export async function load(ctx) {
    return { from: 'server' }
  }
</script>
<main><h1>About {props.from}</h1></main>
`)
  const b = buildSig(brain, d)
  const probe = await startStatic(); const port = probe.port; await probe.close()
  const proc = spawn('node', [join(d, 'dist', 'server.mjs')], { cwd: d, env: { ...process.env, PORT: String(port), HOST: '127.0.0.1' }, stdio: ['ignore', 'pipe', 'pipe'] })
  const out = { printed: b.printed, servedBy: 'node dist/server.mjs (PORT/HOST)', pid: proc.pid }
  try {
    await new Promise((ok, fail) => { let s = ''; const t = setTimeout(() => fail(new Error('server.mjs not ready: ' + s)), 10000); proc.stdout.on('data', (x) => { s += x; if (s.includes('brain:ready')) { clearTimeout(t); ok() } }); proc.stderr.on('data', (x) => (s += x)) })
    const h = await fetch(`http://127.0.0.1:${port}/_health`)
    out.health = await h.json()
    const pages = {}
    for (const p of ['/', '/about', '/sw.js']) {
      const r = await fetch(`http://127.0.0.1:${port}${p}`)
      pages[p] = { status: r.status, xBrainBuild: r.headers.get('x-brain-build'), etag: r.headers.get('etag'), cacheControl: r.headers.get('cache-control') }
    }
    out.pages = pages
  } finally {
    try { process.kill(proc.pid, 'SIGTERM') } catch {}
    await sleep(300)
    try { process.kill(proc.pid, 0); process.kill(proc.pid, 'SIGKILL') } catch {}
  }
  let start
  try { execFileSync(brain, ['start', '--help'], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] }); start = 'exists' }
  catch (e) { start = String(e.stderr || e.stdout).trim().split('\n')[0] }
  out.brainStart = start
  out.healthPass = out.health?.ok === true && out.health.build === b.printed
  out.headerPass = out.pages['/'].xBrainBuild === b.printed && out.pages['/about'].xBrainBuild === b.printed
  out.staticEtagIsSignature = (out.pages['/'].etag ?? '').replace(/"/g, '') === b.printed
  out.swNoCache = /no-cache/.test(out.pages['/sw.js'].cacheControl ?? '')
  out.pass = out.healthPass && out.headerPass
  return out
}

export async function runSignature({ brain, scratch, tasksApp, freshApp }) {
  mkdirSync(join(scratch, 'sig'), { recursive: true })
  const r = {}
  // A case that cannot run (its precondition is not met, or the harness throws) is recorded as BLOCKED with the reason.
  const guard = async (f) => { try { return await f() } catch (e) { return { pass: false, blocked: String(e?.stack || e).split('\n').slice(0, 2).join(' | ') } } }
  r.identical = await guard(() => identical(brain, scratch, freshApp))
  r.health = await guard(() => health(brain, scratch))
  r.deploy = await guard(() => deployAB(brain, scratch, tasksApp))
  r.chunk = await guard(() => deletedChunk(brain, scratch, tasksApp))
  return r
}
