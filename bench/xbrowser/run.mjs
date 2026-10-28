// Cross-browser smoke matrix (QA): proves WebKit (Safari engine) and Firefox are not broken by a Chrome-first team.
// Matrix: app × mode (dev = `brain run`, build = `brain build` + static server) × browser (webkit, firefox, chromium as
// control). Per cell: every route loads with 0 console errors / page errors / failed requests / unhandled rejections,
// the main interactions of each view change the DOM as expected, client navigation does not reload the document and
// back/forward works, the native-feature fallbacks (Navigation API, View Transitions, moveBefore, scheduler.yield) are
// recorded, a 1000-row keyed list renders/reverses correctly (chunked rendering + insertBefore fallback), and in dev
// a <style> and a <script> edit are applied by HMR without a reload (`brain:hmr` window event).
//
// Playwright is a test tool only: it is NOT a dependency of the repo. Install it in a scratch dir (see README.md) or
// in this folder (`npm i` here uses the pinned package.json); this script resolves it from either place.
//
// Usage: node bench/xbrowser/run.mjs [--browsers webkit,firefox,chromium] [--apps tasks,landing,my-project,biglist]
//                                    [--modes dev,build] [--headed]
// Env:   BRAIN (binary, default target/debug/brain) · XB_WORK (scratch dir, default ~/tmp/brainllm-xbrowser)
//        XB_APPS (source apps, default ~/tmp/brainllm/apps; they are copied, never modified)
// Output: bench/xbrowser/results/summary.json and results/summary.md
// A failed cell is run once more; the cell records that it was retried and what failed the first time. The exit code
// is 1 when any cell still fails after its retry (or its app does not build), 0 otherwise.
import { spawn, execFileSync } from 'node:child_process'
import { createServer } from 'node:http'
import { createRequire } from 'node:module'
import { cpSync, existsSync, mkdirSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs'
import { homedir } from 'node:os'
import { extname, join, normalize } from 'node:path'

const HERE = new URL('.', import.meta.url).pathname
const ROOT = new URL('../..', import.meta.url).pathname
const BRAIN = process.env.BRAIN ?? join(ROOT, 'target/debug/brain')
const WORK = process.env.XB_WORK ?? join(homedir(), 'tmp/brainllm-xbrowser')
const SRC = process.env.XB_APPS ?? join(homedir(), 'tmp/brainllm/apps')
const argv = process.argv.slice(2)
const arg = (k, d) => {
  const i = argv.indexOf(`--${k}`)
  return i >= 0 ? argv[i + 1] : d
}
const BROWSERS = arg('browsers', 'webkit,firefox,chromium').split(',')
const APPS = arg('apps', 'tasks,landing,my-project,biglist').split(',')
const MODES = arg('modes', 'dev,build').split(',')
const HEADED = argv.includes('--headed')
const sleep = ms => new Promise(r => setTimeout(r, ms))

// ---------- playwright (scratch dir first, then this folder) ----------
let pw
for (const base of [join(WORK, 'package.json'), join(HERE, 'package.json')]) {
  try {
    pw = createRequire(base)('playwright')
    break
  } catch {}
}
if (!pw) {
  console.error(`playwright not found; fix: (cd ${WORK} && npm init -y && npm i playwright@1.64.0 && npx playwright install webkit firefox chromium)`)
  process.exit(2)
}
const pwVersion = (() => {
  for (const base of [join(WORK, 'package.json'), join(HERE, 'package.json')]) {
    try {
      return createRequire(base)('playwright/package.json').version
    } catch {}
  }
  return '?'
})()

// ---------- children are killed by their own PID only ----------
const children = new Set()
const cleanup = () => {
  for (const c of children) {
    try {
      c.kill('SIGTERM')
    } catch {}
  }
  children.clear()
}
process.on('exit', cleanup)
process.on('SIGINT', () => (cleanup(), process.exit(130)))

// ---------- 1. prepare the apps (copies) and build them ----------
mkdirSync(join(WORK, 'apps'), { recursive: true })
const appDir = a => join(WORK, 'apps', a)
const brainSha = execFileSync('shasum', ['-a', '256', BRAIN]).toString().slice(0, 12)
const BIGLIST_VIEW = `<meta seo title="Big list probe" description="Cross-browser probe: a 1000-row keyed list with reverse and remove." canonical="/">
<script>
  let items = sig([])
  const fill = () => { items = Array.from({ length: 1000 }, (_, i) => ({ id: i, label: 'Row ' + i })) }
  const reverse = () => { items = [...items].reverse() }
  const dropFirst = () => { items = items.slice(1) }
  const swapEnds = () => { const a = [...items]; const t = a[0]; a[0] = a[a.length - 1]; a[a.length - 1] = t; items = a }
</script>
<main>
  <h1>Big list probe</h1>
  <p>
    <button type="button" id="fill" on:click={fill}>Fill 1000</button>
    <button type="button" id="rev" on:click={reverse}>Reverse</button>
    <button type="button" id="drop" on:click={dropFirst}>Remove first</button>
    <button type="button" id="swap" on:click={swapEnds}>Swap ends</button>
  </p>
  <ol id="rows">
    {#each items as it (it.id)}
      <li><button type="button">{it.label}</button></li>
    {:empty}<li>Empty.</li>
    {/each}
  </ol>
</main>
`
const builds = {}
const routesOf = {}
const routesSkipped = {}
// Routes a browser can load as written: a catch-all (`*`, the 404 view) or a parametrized path (`/posts/:id`) is not a
// URL. The biglist probe tests only its own view (`/`); the rest of the scaffold it is created from is not its subject.
const loadable = (a, paths) => {
  if (a === 'biglist') return { keep: ['/'], skip: paths.filter(p => p !== '/').map(p => `${p} (scaffold route, not part of the probe)`) }
  const bad = p => p.includes('*') || /(^|\/):/.test(p) || /\[/.test(p)
  return { keep: paths.filter(p => !bad(p)), skip: paths.filter(bad).map(p => `${p} (catch-all or parametrized)`) }
}
for (const a of APPS) {
  rmSync(appDir(a), { recursive: true, force: true })
  if (a === 'biglist') {
    execFileSync(BRAIN, ['new', 'biglist'], { cwd: join(WORK, 'apps'), stdio: 'ignore' })
    writeFileSync(join(appDir(a), 'views/home.brain'), BIGLIST_VIEW)
  } else {
    cpSync(join(SRC, a), appDir(a), { recursive: true })
  }
  rmSync(join(appDir(a), '.brain/cache'), { recursive: true, force: true })
  rmSync(join(appDir(a), 'dist'), { recursive: true, force: true })
  const mapped = loadable(a, JSON.parse(execFileSync(BRAIN, ['map', '--json'], { cwd: appDir(a) }).toString()).routes.map(r => r.path))
  routesOf[a] = mapped.keep
  routesSkipped[a] = mapped.skip
  if (MODES.includes('build')) {
    // The current binary builds; if it fails, the cell is BLOCKED unless BRAIN_BUILD (a frozen older binary) builds it.
    const build = bin => {
      const t0 = Date.now()
      try {
        execFileSync(bin, ['build'], { cwd: appDir(a), stdio: 'pipe' })
        return { ok: true, ms: Date.now() - t0, binary: bin === BRAIN ? 'current' : `${bin} (sha256 ${execFileSync('shasum', ['-a', '256', bin]).toString().slice(0, 12)})` }
      } catch (e) {
        return { ok: false, ms: Date.now() - t0, error: String(e.stderr || e.stdout || e.message).trim().split('\n').slice(-3).join('\n') }
      }
    }
    builds[a] = build(BRAIN)
    if (!builds[a].ok && process.env.BRAIN_BUILD && existsSync(process.env.BRAIN_BUILD)) {
      const blocked = builds[a].error
      rmSync(join(appDir(a), '.brain/cache'), { recursive: true, force: true })
      builds[a] = { ...build(process.env.BRAIN_BUILD), blockedWithCurrent: blocked }
    }
  }
  console.log(`[prep] ${a}: routes ${routesOf[a].join(' ')}${routesSkipped[a].length ? ` (not loaded: ${routesSkipped[a].join(', ')})` : ''}${builds[a] ? ` · build ${builds[a].ok ? 'ok' : 'FAILED: ' + builds[a].error}` : ''}`)
}

// ---------- 2. servers ----------
// Static server for dist/, emulating the generated nginx snippet (INTERNALS §10): MIME by extension (including the own
// .brain/.jsbrain/.cssbrain), `index` resolution for clean URLs, and 404 otherwise (`try_files $uri $uri/ =404`).
const MIME = {
  '.html': 'text/html; charset=utf-8', '.brain': 'text/html; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8', '.mjs': 'text/javascript; charset=utf-8', '.jsbrain': 'text/javascript; charset=utf-8',
  '.css': 'text/css; charset=utf-8', '.cssbrain': 'text/css; charset=utf-8',
  '.json': 'application/json', '.txt': 'text/plain; charset=utf-8', '.xml': 'application/xml', '.svg': 'image/svg+xml',
  '.png': 'image/png', '.jpg': 'image/jpeg', '.webp': 'image/webp', '.ico': 'image/x-icon', '.webmanifest': 'application/manifest+json',
}
function staticServer(dir, port) {
  const file = p => {
    try {
      return statSync(p).isFile() ? p : null
    } catch {
      return null
    }
  }
  const srv = createServer((req, res) => {
    const u = new URL(req.url, 'http://x')
    const p = normalize(join(dir, decodeURIComponent(u.pathname)))
    if (!p.startsWith(dir)) return res.writeHead(403).end()
    const hit = file(p) || file(join(p, 'index.brain')) || file(join(p, 'index.html'))
    if (!hit) return res.writeHead(404, { 'content-type': 'text/plain' }).end('not found')
    const cache = u.pathname.startsWith('/_b/') ? 'public, max-age=31536000, immutable' : 'no-cache'
    res.writeHead(200, { 'content-type': MIME[extname(hit)] || 'application/octet-stream', 'cache-control': cache })
    res.end(readFileSync(hit))
  })
  return new Promise(r => srv.listen(port, '127.0.0.1', () => r({ url: `http://127.0.0.1:${port}`, stop: () => srv.close() })))
}
async function devServer(dir, port) {
  const c = spawn(BRAIN, ['run', '--port', String(port)], { cwd: dir, stdio: ['ignore', 'pipe', 'pipe'] })
  children.add(c)
  let log = ''
  c.stdout.on('data', d => (log += d))
  c.stderr.on('data', d => (log += d))
  const url = `http://127.0.0.1:${port}`
  for (let i = 0; i < 100; i++) {
    try {
      if ((await fetch(url + '/')).ok) break
    } catch {}
    await sleep(100)
  }
  return { url, log: () => log, stop: () => (c.kill('SIGTERM'), children.delete(c)) }
}

// ---------- 3. per-cell checks ----------
const INIT = `(() => {
  const x = (window.__xb = { rej: [], err: [], push: 0, vt: 0, vtDone: 0, vtSkipped: [], nav: 0, hmr: [] })
  addEventListener('unhandledrejection', e => x.rej.push(String((e.reason && (e.reason.stack || e.reason.message)) || e.reason)))
  addEventListener('brain:hmr', e => x.hmr.push(e.detail))
  const ps = history.pushState
  history.pushState = function (...a) { x.push++; return ps.apply(this, a) }
  if (document.startViewTransition) {
    const svt = document.startViewTransition.bind(document)
    document.startViewTransition = (cb) => {
      x.vt++
      const t = svt(cb)
      t.finished.then(() => x.vtDone++, e => x.vtSkipped.push(String(e && e.name)))
      return t
    }
  }
  if (window.navigation && navigation.addEventListener) navigation.addEventListener('navigatesuccess', () => x.nav++)
})()`

const waitFn = async (page, fn, a, timeout = 5000) => {
  try {
    await page.waitForFunction(fn, a, { timeout })
    return true
  } catch {
    return false
  }
}
const texts = (page, sel) => page.$$eval(sel, els => els.map(e => e.textContent.trim()))
const eq = (a, b) => JSON.stringify(a) === JSON.stringify(b)
const h1Is = (page, t) => waitFn(page, t => document.querySelector('h1')?.textContent.trim() === t, t)

// Interactions per app: each step returns true or a string with the failed assertion.
// Selectors never use the apps' own class names: `brain build` renames component classes (#260, `.plan` → `.b`), so
// they hold only in dev. Elements are found by role, label, text, ARIA and structure instead.
const INTERACTIONS = {
  tasks: [
    ['/', 'empty state', async p => (await texts(p, 'section ul li')).join('|') === 'No tasks yet.' || 'expected "No tasks yet."'],
    ['/', 'add 3 tasks (type + submit)', async p => {
      const input = p.locator('input[placeholder="Write something"]')
      for (const t of ['Alpha', 'Beta', 'Gamma']) {
        await input.fill(t)
        await p.getByRole('button', { name: 'Add' }).click()
      }
      const ok = await waitFn(p, () => document.querySelectorAll('section ul li span').length === 3)
      const got = await texts(p, 'section ul li span')
      if (!ok || !eq(got, ['Alpha', 'Beta', 'Gamma'])) return `items ${JSON.stringify(got)} ≠ ["Alpha","Beta","Gamma"]`
      if ((await input.inputValue()) !== '') return 'input not cleared after submit'
      return (await p.locator('section p').textContent()).trim().startsWith('3 left') || 'counter is not "3 left"'
    }],
    ['/', 'empty submit is ignored', async p => {
      await p.locator('input[placeholder="Write something"]').fill('   ')
      await p.getByRole('button', { name: 'Add' }).click()
      await sleep(150)
      return (await p.$$('section ul li span')).length === 3 || 'a blank task was added'
    }],
    ['/', 'toggle checkbox → done + count', async p => {
      await p.getByRole('checkbox', { name: 'Complete Beta' }).check()
      const ok = await waitFn(p, () => [...document.querySelectorAll('section ul li')].find(l => l.textContent.includes('Beta'))?.classList.contains('done'))
      if (!ok) return 'Beta row has no class "done"'
      return (await waitFn(p, () => document.querySelector('section p').textContent.trim().startsWith('2 left'))) || 'counter is not "2 left"'
    }],
    ['/', 'filters Done / Open / All (keyed re-insert in the middle)', async p => {
      const sel = 'section ul li span'
      await p.getByRole('button', { name: 'Done', exact: true }).click()
      if (!(await waitFn(p, () => document.querySelectorAll('section ul li span').length === 1))) return `Done: ${JSON.stringify(await texts(p, sel))}`
      if (!eq(await texts(p, sel), ['Beta'])) return `Done: ${JSON.stringify(await texts(p, sel))} ≠ ["Beta"]`
      await p.getByRole('button', { name: 'Open', exact: true }).click()
      await waitFn(p, () => document.querySelectorAll('section ul li span').length === 2)
      if (!eq(await texts(p, sel), ['Alpha', 'Gamma'])) return `Open: ${JSON.stringify(await texts(p, sel))} ≠ ["Alpha","Gamma"]`
      await p.getByRole('button', { name: 'All', exact: true }).click()
      await waitFn(p, () => document.querySelectorAll('section ul li span').length === 3)
      return eq(await texts(p, sel), ['Alpha', 'Beta', 'Gamma']) || `All: ${JSON.stringify(await texts(p, sel))} ≠ ["Alpha","Beta","Gamma"]`
    }],
    ['/', 'remove first row', async p => {
      await p.getByRole('button', { name: 'Remove Alpha' }).click({ timeout: 5000 })
      await waitFn(p, () => document.querySelectorAll('section ul li span').length === 2)
      const got = await texts(p, 'section ul li span')
      return eq(got, ['Beta', 'Gamma']) || `${JSON.stringify(got)} ≠ ["Beta","Gamma"]`
    }],
    ['/', 'counter click + derived + class:hot', async p => {
      const b = p.locator('button', { hasText: 'Focus sessions' })
      await b.click()
      if (!(await waitFn(p, () => /2 × 2 = 4/.test([...document.querySelectorAll('button')].find(b => b.textContent.includes('Focus sessions'))?.textContent))))
        return `counter text "${(await b.textContent()).trim()}" ≠ "… 2 × 2 = 4"`
      // class:hot adds a class at n > 10: `hot` in dev, a renamed class in build — so compare with the classes at n = 2
      const before = await b.evaluate(el => [...el.classList])
      for (let i = 0; i < 9; i++) await b.click()
      return (await waitFn(p, before => { const el = [...document.querySelectorAll('button')].find(b => b.textContent.includes('Focus sessions')); return !!el && [...el.classList].some(c => !before.includes(c)) }, before)) || `class:hot added no class at n = 11 (classes before: ${JSON.stringify(before)})`
    }],
  ],
  landing: [
    ['/', 'pricing plans (nested keyed each)', async p => {
      const names = await texts(p, 'section[aria-labelledby="pricing"] article h3'), perks = await texts(p, 'section[aria-labelledby="pricing"] article li')
      return (eq(names, ['Free', 'Pro']) && perks.length === 5) || `plans ${JSON.stringify(names)}, perks ${perks.length}`
    }],
    ['/', 'waitlist: invalid email does not submit', async p => {
      await p.locator('#em').fill('nobody')
      await p.getByRole('button', { name: 'Join' }).click()
      await sleep(150)
      return (await p.locator('text=Thanks!').count()) === 0 || 'thanks shown for an invalid email'
    }],
    ['/', 'waitlist: valid email → thanks (if/else)', async p => {
      await p.locator('#em').fill('ada@example.com')
      await p.getByRole('button', { name: 'Join' }).click()
      return (await waitFn(p, () => document.body.textContent.includes('Thanks! We will write to ada@example.com.'))) || 'thanks message not shown'
    }],
    ['/', 'counter click + derived', async p => {
      await p.locator('button', { hasText: 'Demo clicks' }).click()
      return (await waitFn(p, () => /Demo clicks: 1 × 2 = 2/.test(document.body.textContent))) || 'counter is not "Demo clicks: 1 × 2 = 2"'
    }],
  ],
  'my-project': [
    ['/', 'starter prompt in textarea (bind:value)', async p => ((await p.locator('#prompt').inputValue()).includes('brain spec') || 'textarea has no prompt')],
    ['/', 'steps list', async p => ((await p.$$('main ol > li')).length === 3 || 'expected 3 steps')],
    ['/', 'copy button → "Copied" (async handler + clipboard)', async p => {
      await p.getByRole('button', { name: 'Copy prompt' }).click()
      return (await waitFn(p, () => [...document.querySelectorAll('button')].some(b => b.textContent.trim() === 'Copied'))) || 'button did not change to "Copied"'
    }],
  ],
  biglist: [
    ['/', 'empty state', async p => (eq(await texts(p, '#rows li'), ['Empty.']) || 'expected "Empty."')],
    ['/', 'fill 1000 keyed rows (chunked render)', async p => {
      // Click inside the page to sample how many rows exist synchronously and on the next frames: chunked rendering
      // (>500 rows, ≤5 ms slices) shows fewer than 1000 rows right after the click.
      const r = await p.evaluate(async () => {
        const n = () => document.querySelectorAll('#rows li button').length
        const t0 = performance.now()
        document.getElementById('fill').click()
        const sync = n()
        const frames = []
        while (n() < 1000 && performance.now() - t0 < 10000) {
          await new Promise(r => requestAnimationFrame(r))
          frames.push(n())
        }
        return { sync, frames: frames.length, ms: Math.round(performance.now() - t0), final: n() }
      })
      p.__bigFill = r
      if (r.final !== 1000) return `rendered ${r.final} rows ≠ 1000`
      const all = await texts(p, '#rows li button')
      const want = Array.from({ length: 1000 }, (_, i) => 'Row ' + i)
      return eq(all, want) || 'rows out of order after fill'
    }],
    ['/', 'reverse keeps node identity and order', async p => {
      const r = await p.evaluate(async () => {
        const rows = () => [...document.querySelectorAll('#rows li')]
        const mark = rows()[500]
        mark.__xb = 1
        const btn = mark.querySelector('button')
        btn.focus()
        document.getElementById('rev').click() // .click() does not move focus
        for (let i = 0; i < 300 && rows()[0].textContent.trim() !== 'Row 999'; i++) await new Promise(r => requestAnimationFrame(r))
        const now = rows()
        return {
          labels: now.map(l => l.textContent.trim()),
          sameNode: now.find(l => l.textContent.trim() === 'Row 500') === mark,
          focusKept: document.activeElement === btn,
          moveBefore: typeof Element.prototype.moveBefore === 'function',
        }
      })
      p.__bigRev = r
      const want = Array.from({ length: 1000 }, (_, i) => 'Row ' + (999 - i))
      if (!eq(r.labels, want)) return `order wrong after reverse: first ${r.labels.slice(0, 3)} last ${r.labels.slice(-3)}`
      return r.sameNode || 'row "Row 500" was recreated instead of moved (keyed identity lost)'
    }],
    ['/', 'swap ends + remove first', async p => {
      await p.locator('#swap').click()
      await waitFn(p, () => document.querySelector('#rows li')?.textContent.trim() === 'Row 0')
      let got = await texts(p, '#rows li button')
      const want = ['Row 0', ...Array.from({ length: 998 }, (_, i) => 'Row ' + (998 - i)), 'Row 999']
      if (!eq(got, want)) return `swap: first ${got.slice(0, 3)} last ${got.slice(-2)}`
      await p.locator('#drop').click()
      await waitFn(p, () => document.querySelectorAll('#rows li button').length === 999)
      got = await texts(p, '#rows li button')
      return eq(got, want.slice(1)) || `remove first: ${got.length} rows, first ${got.slice(0, 2)}`
    }],
  ],
}

// First interaction on a fresh load. A real click holds the button ~100-150 ms: the island must survive the
// mousedown → focus → mouseup sequence. Text typed before the island resumes must be kept.
const press = async (page, loc) => {
  const b = await loc.boundingBox()
  await page.mouse.move(b.x + b.width / 2, b.y + b.height / 2)
  await page.mouse.down()
  await sleep(150)
  await page.mouse.up()
}
const typedBeforeResume = sel => async p => {
  const v = await p.evaluate(async sel => {
    const i = document.querySelector(sel)
    i.focus()
    i.value = 'Typed early'
    i.dispatchEvent(new InputEvent('input', { bubbles: true, inputType: 'insertText', data: 'Typed early' }))
    await new Promise(r => setTimeout(r, 400))
    return document.querySelector(sel).value
  }, sel)
  return v === 'Typed early' || `input value after the island resumed is ${JSON.stringify(v)} (typed text lost)`
}
const FIRST = {
  tasks: [
    ['human-speed click (150 ms press) on the counter', async p => {
      await press(p, p.locator('button', { hasText: 'Focus sessions' }))
      return (await waitFn(p, () => /2 × 2 = 4/.test(document.body.textContent), null, 2000)) || 'the first click was lost (counter still at 1)'
    }],
    ['text typed before the island resumes is kept', typedBeforeResume('input[placeholder="Write something"]')],
  ],
  landing: [
    ['human-speed click (150 ms press) on the counter', async p => {
      await press(p, p.locator('button', { hasText: 'Demo clicks' }))
      return (await waitFn(p, () => /Demo clicks: 1 × 2 = 2/.test(document.body.textContent), null, 2000)) || 'the first click was lost (counter still at 0)'
    }],
    ['text typed before the island resumes is kept', typedBeforeResume('#em')],
  ],
  'my-project': [
    ['human-speed click (150 ms press) on "Copy prompt"', async p => {
      await press(p, p.getByRole('button', { name: 'Copy prompt' }))
      return (await waitFn(p, () => [...document.querySelectorAll('button')].some(b => b.textContent.trim() === 'Copied'), null, 2000)) || 'the first click was lost (button still "Copy prompt")'
    }],
  ],
  biglist: [
    ['human-speed click (150 ms press) on "Fill 1000"', async p => {
      await press(p, p.locator('#fill'))
      return (await waitFn(p, () => document.querySelectorAll('#rows li button').length === 1000, null, 3000)) || 'the first click was lost (list still empty)'
    }],
  ],
}

// HMR edits (dev): one <style> line and one <script> line of a component; then the original is restored.
const HMR = {
  tasks: { file: 'comp/counter.brain', label: 'Focus sessions', n: 2 },
  landing: { file: 'comp/counter.brain', label: 'Demo clicks', n: 1 },
}
const STYLE_FROM = 'button { padding: var(--space-2) var(--space-4);'
const STYLE_TO = 'button { letter-spacing: 3px; padding: var(--space-2) var(--space-4);'
const SCRIPT_FROM = 'let dbl = drv(() => n * 2)'
const SCRIPT_TO = 'let dbl = drv(() => n * 3)'

async function runCell(app, mode, bname, base) {
  const cell = { app, mode, browser: bname, routes: [], interactions: [], errors: [], notes: [], nav: null, features: null, hmr: null, list: null }
  const browser = await pw[bname].launch({ headless: !HEADED })
  cell.browserVersion = browser.version()
  const ctx = await browser.newContext()
  if (bname === 'chromium') await ctx.grantPermissions(['clipboard-read', 'clipboard-write'], { origin: base }).catch(() => {})
  await ctx.addInitScript(INIT)
  const page = await ctx.newPage()
  let step = 'load'
  const isFavicon = u => /\/favicon\.ico$/.test(u || '')
  page.on('console', m => {
    if (m.type() !== 'error') return
    if (isFavicon(m.location()?.url)) return cell.notes.push(`${step}: console 404 favicon.ico (app has none)`)
    cell.errors.push({ step, kind: 'console', text: m.text(), at: m.location()?.url })
  })
  page.on('pageerror', e => cell.errors.push({ step, kind: 'pageerror', text: String(e.stack || e.message) }))
  page.on('requestfailed', r => {
    if (isFavicon(r.url())) return
    cell.errors.push({ step, kind: 'requestfailed', text: `${r.failure()?.errorText} ${r.url()}` })
  })
  page.on('response', r => {
    if (r.status() < 400) return
    if (isFavicon(r.url())) return cell.notes.push(`${step}: HTTP ${r.status()} favicon.ico (app has none)`)
    cell.errors.push({ step, kind: 'http', text: `${r.status()} ${r.url()}` })
  })
  const drain = async () => {
    const x = await page.evaluate(() => {
      const r = { rej: window.__xb.rej.splice(0) }
      return r
    }).catch(() => ({ rej: [] }))
    for (const t of x.rej) cell.errors.push({ step, kind: 'unhandledrejection', text: t })
  }
  const settle = async () => {
    await page.waitForLoadState('networkidle', { timeout: 4000 }).catch(() => {})
  }
  const mark = () => page.evaluate(() => (window.__probe = Math.random().toString(36).slice(2)))
  const same = async tok => (await page.evaluate(() => window.__probe).catch(() => undefined)) === tok

  try {
    // 3.1 every route
    for (const r of routesOf[app]) {
      step = `route ${r}`
      const resp = await page.goto(base + r, { waitUntil: 'load' })
      await settle()
      const ok = await waitFn(page, () => {
        const v = document.getElementById('brain-view') || document.querySelector('main')
        return !!(v && v.querySelector('h1'))
      })
      cell.routes.push({ path: r, status: resp && resp.status(), ok, why: ok ? '' : 'no <h1> mounted in the view region' })
      await drain()
    }
    // 3.2 interactions (fresh load of each route they need; one sequence per app shares the page state)
    let cur = null, tok = null
    for (const [r, name, fn] of INTERACTIONS[app] || []) {
      step = `interaction ${name}`
      if (cur !== r) {
        await page.goto(base + r, { waitUntil: 'load' })
        await settle()
        cur = r
        tok = await mark()
      }
      let res
      try {
        res = await fn(page)
      } catch (e) {
        res = String(e.message).split('\n')[0]
      }
      const reloaded = !(await same(tok))
      if (reloaded) (res = res === true ? 'the document reloaded during the interaction' : res + ' (document reloaded)'), (tok = await mark())
      cell.interactions.push({ name, ok: res === true, why: res === true ? '' : res })
      await drain()
    }
    if (app === 'biglist') cell.list = { fill: page.__bigFill || null, reverse: page.__bigRev ? { sameNode: page.__bigRev.sameNode, focusKept: page.__bigRev.focusKept } : null }

    // 3.2b first interaction on a fresh load, the way a person does it (build pages resume islands on the first event)
    for (const [name, fn] of FIRST[app] || []) {
      step = `first interaction ${name}`
      await page.goto(base + '/', { waitUntil: 'load' })
      await settle()
      let res
      try {
        res = await fn(page)
      } catch (e) {
        res = String(e.message).split('\n')[0]
      }
      cell.interactions.push({ name: 'first: ' + name, ok: res === true, why: res === true ? '' : res })
      await drain()
    }

    // 3.3 client navigation
    step = 'navigation'
    cell.nav = await navTest(app, page, base, mark, same, settle)
    await drain()

    // 3.4 feature availability
    step = 'features'
    cell.features = await page.evaluate(() => ({
      navigationAPI: 'navigation' in window,
      viewTransitions: typeof document.startViewTransition === 'function',
      moveBefore: typeof Element.prototype.moveBefore === 'function',
      schedulerYield: !!(globalThis.scheduler && typeof scheduler.yield === 'function'),
      schedulerPostTask: !!(globalThis.scheduler && typeof scheduler.postTask === 'function'),
    }))

    // 3.5 HMR (dev only)
    if (mode === 'dev' && HMR[app]) {
      step = 'hmr'
      cell.hmr = await hmrTest(app, page, base, mark, same, settle)
      await drain()
    }
  } catch (e) {
    cell.errors.push({ step, kind: 'harness', text: String(e.stack || e.message).split('\n').slice(0, 3).join(' | ') })
  } finally {
    await browser.close()
  }
  return cell
}

async function navTest(app, page, base, mark, same, settle) {
  const nav = { ok: false, steps: [], router: null, viewTransitions: null }
  const add = (name, ok, why = '') => nav.steps.push({ name, ok, why })
  const link = href => page.evaluate(h => {
    let a = document.getElementById('__xbnav')
    if (!a) (a = document.createElement('a')), (a.id = '__xbnav'), (a.textContent = 'xb nav'), document.body.append(a)
    a.href = h
  }, href)
  if (app === 'tasks') {
    await page.goto(base + '/about', { waitUntil: 'load' })
    await settle()
    const tok = await mark()
    await page.getByRole('link', { name: 'Back to tasks' }).click()
    add('link /about → / (view link)', (await h1Is(page, 'My tasks')) && (await same(tok)) && new URL(page.url()).pathname === '/', 'h1, URL or document identity wrong')
    const live = await (async () => {
      await page.locator('input[placeholder="Write something"]').fill('Delta')
      await page.getByRole('button', { name: 'Add' }).click()
      return waitFn(page, () => [...document.querySelectorAll('section ul li span')].some(s => s.textContent === 'Delta'))
    })()
    add('view is interactive after client nav (add task)', live, 'adding a task after navigation did nothing')
    await link('/about')
    await page.locator('#__xbnav').click()
    add('link / → /about (injected link)', (await h1Is(page, 'About')) && (await same(tok)), 'h1 or document identity wrong')
    await page.goBack({ waitUntil: 'commit' }).catch(() => {})
    const back = (await h1Is(page, 'My tasks')) && (await same(tok)) && new URL(page.url()).pathname === '/'
    add('back → /', back, `h1 "${await page.locator('h1').first().textContent().catch(() => '?')}", url ${page.url()}`)
    // DOM consistency after returning to a page whose store changed while away (prerendered markup vs live store)
    const cons = await page.evaluate(() => {
      const items = [...document.querySelectorAll('section ul li')].filter(l => l.querySelector('span'))
      const open = items.filter(l => !l.classList.contains('done')).length
      const left = parseInt(document.querySelector('section p')?.textContent, 10)
      return { rows: items.map(l => l.querySelector('span').textContent), left, consistent: open === left }
    }).catch(() => null)
    nav.storeAfterBack = cons
    add('list and "N left" agree after back', !!cons && cons.consistent, `rows ${JSON.stringify(cons && cons.rows)} vs left ${cons && cons.left}`)
    await page.goForward({ waitUntil: 'commit' }).catch(() => {})
    add('forward → /about', (await h1Is(page, 'About')) && (await same(tok)), 'h1 or document identity wrong')
  } else {
    // Single-route apps: navigate to the same view with a query string (the router still fetches and swaps).
    await page.goto(base + '/', { waitUntil: 'load' })
    await settle()
    // A build with one route ships no client router (by design): a link is a normal document load → n/a.
    if (!(await page.evaluate(() => /__brainDev|\/@brain\/client\.js|router\(/.test(document.documentElement.outerHTML))))
      return { ok: null, steps: [], router: 'none shipped (single route)', viewTransitions: 'n/a' }
    const tok = await mark()
    const h1 = (await page.locator('h1').first().textContent()).trim()
    await link('/?xb=1')
    await page.locator('#__xbnav').click()
    const ok1 = (await waitFn(page, () => location.search === '?xb=1')) && (await h1Is(page, h1)) && (await same(tok))
    add('link / → /?xb=1 (injected link)', ok1, `url ${page.url()}, reloaded ${!(await same(tok))}`)
    await page.goBack({ waitUntil: 'commit' }).catch(() => {})
    add('back → /', (await waitFn(page, () => location.search === '')) && (await h1Is(page, h1)) && (await same(tok)), `url ${page.url()}`)
    await page.goForward({ waitUntil: 'commit' }).catch(() => {})
    add('forward → /?xb=1', (await waitFn(page, () => location.search === '?xb=1')) && (await same(tok)), `url ${page.url()}`)
  }
  await sleep(300)
  const x = await page.evaluate(() => ({ push: window.__xb.push, nav: window.__xb.nav, vt: window.__xb.vt, vtDone: window.__xb.vtDone, vtSkipped: window.__xb.vtSkipped, hasNav: 'navigation' in window, hasVT: !!document.startViewTransition })).catch(() => null)
  if (x) {
    nav.router = x.push > 0 ? 'History API (pushState)' : x.hasNav && x.nav > 0 ? 'Navigation API' : 'unknown'
    nav.viewTransitions = !x.hasVT ? 'unsupported (swap without VT)' : x.vt === 0 ? 'available, not used' : `${x.vt} started, ${x.vtDone} finished, ${x.vtSkipped.length} skipped${x.vtSkipped.length ? ' (' + [...new Set(x.vtSkipped)].join(',') + ')' : ''}`
    nav.counters = x
  }
  nav.ok = nav.steps.every(s => s.ok)
  return nav
}

async function hmrTest(app, page, base, mark, same, settle) {
  const cfg = HMR[app]
  const file = join(appDir(app), cfg.file)
  const orig = readFileSync(file, 'utf8')
  const out = { ok: false, style: null, script: null, restore: null }
  const btn = `[...document.querySelectorAll('button')].find(b => b.textContent.includes(${JSON.stringify(cfg.label)}))`
  const events = () => page.evaluate(() => window.__xb.hmr.splice(0)).catch(() => null)
  try {
    await page.goto(base + '/', { waitUntil: 'load' })
    await settle()
    await sleep(500) // WebSocket open
    for (let i = 0; i < cfg.n; i++) await page.locator('button', { hasText: cfg.label }).click()
    const n = cfg.n + (app === 'tasks' ? 1 : 0) // tasks' counter starts at 1
    const tok = await mark()
    await events()
    // <style> edit
    writeFileSync(file, orig.replace(STYLE_FROM, STYLE_TO))
    const sOk = await waitFn(page, new Function(`const b = ${btn}; return !!b && getComputedStyle(b).letterSpacing === '3px'`), null, 8000)
    const sEv = await events()
    out.style = { ok: sOk && (await same(tok)), applied: sOk, noReload: await same(tok), events: (sEv || []).map(e => e.kind) }
    // <script> edit: the derived value changes, the sig `n` keeps its value (state preserved)
    writeFileSync(file, orig.replace(STYLE_FROM, STYLE_TO).replace(SCRIPT_FROM, SCRIPT_TO))
    const want = `${n} × 2 = ${n * 3}`
    const cOk = await waitFn(page, new Function(`const b = ${btn}; return !!b && b.textContent.includes(${JSON.stringify(want)})`), null, 8000)
    const cEv = await events()
    out.script = { ok: cOk && (await same(tok)), applied: cOk, noReload: await same(tok), expected: want, got: await page.evaluate(new Function(`return ${btn}?.textContent`)).catch(() => null), events: (cEv || []).map(e => e.kind) }
    // restore
    writeFileSync(file, orig)
    const rOk = await waitFn(page, new Function(`const b = ${btn}; return !!b && b.textContent.includes(${JSON.stringify(`${n} × 2 = ${n * 2}`)}) && getComputedStyle(b).letterSpacing !== '3px'`), null, 8000)
    // The restore is one save that changes <style> and <script> together (server sends js:true css:true).
    const rGot = await page.evaluate(new Function(`const b = ${btn}; return b && { text: b.textContent, letterSpacing: getComputedStyle(b).letterSpacing }`)).catch(() => null)
    out.restore = { ok: rOk && (await same(tok)), kind: 'style + script in one save', textOk: !!rGot && rGot.text.includes(`${n} × 2 = ${n * 2}`), styleOk: !!rGot && rGot.letterSpacing !== '3px', got: rGot, noReload: await same(tok), events: ((await events()) || []).map(e => e.kind) }
    out.ok = out.style.ok && out.script.ok && out.restore.ok
  } finally {
    writeFileSync(file, orig)
    await sleep(400)
  }
  return out
}

// Every failed check of a cell, one line each (shared by the retry decision, the report and the exit code).
const cellFailures = c => {
  const out = []
  if (c.skipped) return [`- ${c.app} · ${c.mode} · ${c.browser} · skipped: ${c.skipped.split('\n')[0]}`]
  for (const r of c.routes) if (!r.ok) out.push(`- ${c.app} · ${c.mode} · ${c.browser} · route ${r.path}: ${r.why}`)
  for (const i of c.interactions) if (!i.ok) out.push(`- ${c.app} · ${c.mode} · ${c.browser} · ${i.name}: ${i.why}`)
  for (const s of c.nav?.steps || []) if (!s.ok) out.push(`- ${c.app} · ${c.mode} · ${c.browser} · nav ${s.name}: ${s.why}`)
  if (c.hmr && !c.hmr.ok) out.push(`- ${c.app} · ${c.mode} · ${c.browser} · hmr: ${JSON.stringify(c.hmr)}`)
  for (const e of c.errors) out.push(`- ${c.app} · ${c.mode} · ${c.browser} · ${e.step} · ${e.kind}: ${e.text.split('\n')[0].slice(0, 300)}`)
  return out
}

// ---------- 4. run the matrix ----------
const cells = []
let port = 5340
for (const app of APPS) {
  for (const mode of MODES) {
    if (mode === 'build' && !builds[app]?.ok) {
      for (const b of BROWSERS) cells.push({ app, mode, browser: b, skipped: `brain build failed: ${builds[app]?.error}` })
      continue
    }
    port += 2 // dev uses P and P+1 (inspector)
    const srv = mode === 'dev' ? await devServer(appDir(app), port) : await staticServer(join(appDir(app), 'dist'), port)
    if (mode === 'build') {
      // Sanity: the static server must serve the chunks as JavaScript.
      const html = await (await fetch(srv.url + '/')).text()
      const js = (html.match(/\/_b\/[\w.-]+\.(?:js|jsbrain)/) || [])[0]
      if (js) builds[app].jsContentType = (await fetch(srv.url + js)).headers.get('content-type')
    }
    for (const b of BROWSERS) {
      let t0 = Date.now()
      let c = await runCell(app, mode, b, srv.url)
      c.ms = Date.now() - t0
      const first = cellFailures(c)
      if (first.length) {
        // One retry: a cell that fails only under machine load passes the second time; the first failure is kept.
        console.log(`[cell] ${app} · ${mode} · ${b}: ${first.length} failure(s), retrying once: ${first[0].slice(2, 200)}`)
        t0 = Date.now()
        c = await runCell(app, mode, b, srv.url)
        c.ms = Date.now() - t0
        c.retried = { firstAttemptFailures: first.map(f => f.slice(2)), passedOnRetry: cellFailures(c).length === 0 }
      }
      c.routesSkipped = routesSkipped[app]
      cells.push(c)
      const errs = c.errors.length, fails = c.interactions.filter(i => !i.ok).length + c.routes.filter(r => !r.ok).length
      console.log(`[cell] ${app} · ${mode} · ${b} ${c.browserVersion}: routes ${c.routes.filter(r => r.ok).length}/${c.routes.length}, interactions ${c.interactions.filter(i => i.ok).length}/${c.interactions.length}, errors ${errs}, nav ${c.nav?.ok === null ? 'n/a' : c.nav?.ok ? 'ok' : 'FAIL'}${c.hmr ? `, hmr ${c.hmr.ok ? 'ok' : 'FAIL'}` : ''} (${c.ms} ms)${fails || errs ? '  <-- check' : ''}`)
    }
    if (mode === 'dev') {
      const l = srv.log(); if (process.env.XB_DEBUG) console.log(l)
      if (/panic|error/i.test(l)) cells.filter(c => c.app === app && c.mode === 'dev').forEach(c => c.notes.push('dev server log: ' + l.split('\n').filter(x => /panic|error/i.test(x)).slice(0, 3).join(' | ')))
    }
    srv.stop()
  }
}

// ---------- 5. results ----------
const versions = {}
for (const c of cells) if (c.browserVersion) versions[c.browser] = c.browserVersion
const summary = {
  date: new Date().toISOString(),
  playwright: pwVersion,
  browsers: versions,
  brain: { path: BRAIN.replace(ROOT, '').replace(homedir(), '~'), sha256_12: brainSha },
  node: process.version,
  builds,
  cells,
}
mkdirSync(join(HERE, 'results'), { recursive: true })
writeFileSync(join(HERE, 'results/summary.json'), JSON.stringify(summary, null, 2).replaceAll(homedir(), '~') + '\n')

const yn = v => (v ? 'yes' : 'no')
const rows = [
  '| app | mode | browser | routes OK | interactions OK | console/page/net errors | nav without reload (router) | Navigation API | View Transitions | moveBefore | scheduler.yield | HMR style / script / both-in-one-save |',
  '|---|---|---|---|---|---|---|---|---|---|---|---|',
]
for (const c of cells) {
  if (c.skipped) {
    rows.push(`| ${c.app} | ${c.mode} | ${c.browser} | skipped | — | — | — | — | — | — | — | ${c.skipped.split('\n')[0].slice(0, 80)} |`)
    continue
  }
  const f = c.features || {}
  const hmr = c.hmr ? `${c.hmr.style?.ok ? 'ok' : 'FAIL'} / ${c.hmr.script?.ok ? 'ok' : 'FAIL'} / ${c.hmr.restore?.ok ? 'ok' : 'FAIL'}` : c.mode === 'dev' ? 'not run' : 'n/a'
  rows.push(
    `| ${c.app} | ${c.mode} | ${c.browser} ${c.browserVersion} | ${c.routes.filter(r => r.ok).length}/${c.routes.length} | ${c.interactions.filter(i => i.ok).length}/${c.interactions.length} | ${c.errors.length} | ${c.nav ? (c.nav.ok === null ? 'n/a' : c.nav.ok ? 'yes' : 'NO') + ` (${c.nav.router})` : '—'} | ${yn(f.navigationAPI)} | ${yn(f.viewTransitions)}${c.nav?.viewTransitions && c.mode === 'build' ? ` (${c.nav.viewTransitions})` : ''} | ${yn(f.moveBefore)} | ${yn(f.schedulerYield)} | ${hmr} |`,
  )
}
const fails = cells.flatMap(cellFailures)
const retried = cells.filter(c => c.retried).map(c => `- ${c.app} · ${c.mode} · ${c.browser}: ${c.retried.passedOnRetry ? 'passed on retry' : 'failed again'}; first attempt: ${c.retried.firstAttemptFailures.join(' / ').slice(0, 400)}`)
const notLoaded = Object.entries(routesSkipped).filter(([, v]) => v.length).map(([a, v]) => `- ${a}: ${v.join(', ')}`)
const lists = cells.filter(c => c.list).map(c => `| ${c.mode} | ${c.browser} | ${c.list.fill ? `${c.list.fill.sync} → ${c.list.fill.final} rows in ${c.list.fill.frames} frames, ${c.list.fill.ms} ms` : '—'} | ${c.list.reverse ? yn(c.list.reverse.sameNode) : '—'} | ${c.list.reverse ? yn(c.list.reverse.focusKept) : '—'} |`)
const md = [
  `# Cross-browser matrix — ${summary.date.slice(0, 10)}`,
  '',
  `playwright ${pwVersion} · ${Object.entries(versions).map(([k, v]) => `${k} ${v}`).join(' · ')} · brain sha256 ${brainSha} · node ${process.version}`,
  '',
  ...rows,
  '',
  '## Big list (1000 keyed rows)',
  '',
  '| mode | browser | fill: rows right after click → final | reverse keeps node identity | reverse keeps focus |',
  '|---|---|---|---|---|',
  ...lists,
  '',
  `## Failures after retry (${fails.length})`,
  '',
  ...(fails.length ? fails : ['none']),
  '',
  `## Retried cells (${retried.length})`,
  '',
  ...(retried.length ? retried : ['none']),
  '',
  '## Routes not loaded',
  '',
  ...(notLoaded.length ? notLoaded : ['none']),
  '',
]
writeFileSync(join(HERE, 'results/summary.md'), md.join('\n').replaceAll(homedir(), '~'))
console.log(md.join('\n'))
cleanup()
process.exitCode = fails.length ? 1 : 0
