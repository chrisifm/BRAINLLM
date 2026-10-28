// QA #153 (INTERNALS §37): a 3-locale example site (en default, es, ar right-to-left) checked end to end.
//   1. bench/i18n/app: `brain check` → [] and `brain doctor` → 0, plus probes of the §37 features the app had to
//      work around (each probe is a scratch copy of the app with one change).
//   2. `brain build` + `brain start`, then headless Chrome (CDP) per locale and view: URL scheme, <html lang dir>,
//      texts in the locale, hreflang + x-default, mirrored layout for ar, plural / placeholder / fmt output.
//   3. Network per page opened fresh: only the current locale's dictionary chunk of that view, none on static pages,
//      the other locale's chunk only after switching language. Dictionary bytes per chunk.
//   4. LLM gate: the apps written by bench/autonomy/run.mjs for briefs-153.json (`--llm` runs the harness first).
// Repository rule: no Spanish or Arabic in files, so the es and ar locale files hold English text tagged [es] / [ar].
//
// Usage: node bench/i18n/run.mjs [--brain target/release/brain] [--port 5331] [--llm]
// Output: bench/i18n/results/summary.{json,md} · exit 1 on any FAIL (BLOCKED = the feature's command does not exist yet).
import { execFileSync, spawn, spawnSync } from 'node:child_process'
import { cpSync, existsSync, mkdirSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync } from 'node:fs'
import { homedir } from 'node:os'
import { join } from 'node:path'
import { createServer } from 'node:net'
import { createHash } from 'node:crypto'
import { launch, openPage } from '../lib/cdp.mjs'

const ROOT = new URL('../..', import.meta.url).pathname
const HERE = join(ROOT, 'bench/i18n')
const arg = (k, d) => { const i = process.argv.indexOf(`--${k}`); return i > 0 ? process.argv[i + 1] : d }
const RUN_LLM = process.argv.includes('--llm')
const SCRATCH = process.env.BRAIN_I18N_SCRATCH ?? join(homedir(), 'tmp/brainllm-i18n153')
process.setMaxListeners(64) // cdp.mjs adds one exit listener per Chrome launch (one launch per page here)
const sleep = (ms) => new Promise((r) => setTimeout(r, ms))
const home = (s) => String(s).replaceAll(homedir(), '~')
const sha = (f) => createHash('sha256').update(readFileSync(f)).digest('hex').slice(0, 12)

// Freeze the binary for the whole run (a concurrent rebuild must not change the tool mid-run).
mkdirSync(SCRATCH, { recursive: true })
const BRAIN = join(SCRATCH, 'brain')
// macOS kills (SIGKILL, code signature) a binary copied over a previous copy: remove it first.
rmSync(BRAIN, { force: true }); execFileSync('cp', [arg('brain', join(ROOT, 'target/release/brain')), BRAIN])
const BRAIN_SHA = sha(BRAIN)

const results = []
let stopServer = () => {}
let dicts = []
const net = []
const llm = []
const rec = (id, group, status, evidence, owner = null) => {
  results.push({ id, group, status, evidence: home(evidence), owner })
  console.log(`${status.padEnd(7)} ${id.padEnd(30)} ${home(evidence)}${owner && status !== 'PASS' ? `  [${owner}]` : ''}`)
}
const brain = (cwd, args, timeout = 300_000) => {
  const r = spawnSync(BRAIN, args, { cwd, encoding: 'utf8', timeout })
  return { code: r.status, out: `${r.stdout ?? ''}${r.stderr ?? ''}` }
}
const parseJson = (s) => { try { return JSON.parse(s.slice(s.search(/[[{]/))) } catch { return null } }
const copyApp = (name) => {
  const dir = join(SCRATCH, name)
  rmSync(dir, { recursive: true, force: true })
  cpSync(join(HERE, 'app'), dir, { recursive: true, filter: (s) => !/\/(dist|\.brain|logs)(\/|$)/.test(s) })
  // .brain_env is never committed: the production section gives the site URL used by canonical/hreflang
  writeFileSync(join(dir, '.brain_env'), 'PUBLIC_SITE_NAME=i18n153\nPUBLIC_SITE_URL=http://localhost:5180\n\n[production]\nPUBLIC_SITE_URL=https://example.com\n')
  return dir
}
const edit = (dir, file, f) => writeFileSync(join(dir, file), f(readFileSync(join(dir, file), 'utf8')))

const LOCALES = ['en', 'es', 'ar']
const TAG = { en: '', es: ' [es]', ar: ' [ar]' }
const URLS = {
  en: { home: '/', about: '/about', counter: '/counter' },
  es: { home: '/es', about: '/es/about-us', counter: '/es/counter' },
  ar: { home: '/ar', about: '/ar/about-us', counter: '/ar/counter' },
}
const H1 = { home: 'A site in three languages', about: 'About this probe', counter: 'Click counter' }
const RELEASED = '2026-03-14T12:00:00Z'

console.log(`brain ${BRAIN_SHA} · scratch ${home(SCRATCH)}\n`)

// ---------------------------------------------------------------- 1. the example app
const APP = copyApp('app')
{
  const c = brain(APP, ['check', '--json', '.'])
  const diags = parseJson(c.out)
  rec('check', '1 app', Array.isArray(diags) && diags.length === 0 ? 'PASS' : 'FAIL', `brain check --json . → ${Array.isArray(diags) ? JSON.stringify(diags.map((d) => d.code)) : c.out.slice(0, 200)}`, 'COMPILER')
  const d = brain(APP, ['doctor', '--json'])
  const dj = parseJson(d.out)
  const seo = dj?.seo?.pages?.filter((p) => p.score != null).map((p) => p.score) ?? []
  rec('doctor', '1 app', d.code === 0 ? 'PASS' : 'FAIL', `brain doctor → exit ${d.code}${seo.length ? ` · SEO ${Math.min(...seo)}–${Math.max(...seo)} over ${seo.length} pages` : ''}`, 'TOOLCHAIN')
}

// Probes: the §37 features the example had to work around, each in its own scratch copy.
{
  // a) localized <title>/description: the only way is t() inside <meta seo>
  const p = copyApp('probe-seo-t')
  edit(p, 'views/home.brain', (s) => s.replace(/<meta seo title="[^"]*"/, "<meta seo title={t('home.title')}"))
  const b = brain(p, ['build'])
  const threw = b.out.match(/prerender threw[^\n]*/g) ?? []
  const esTitle = existsSync(join(p, 'dist/es/index.html')) ? (readFileSync(join(p, 'dist/es/index.html'), 'utf8').match(/<title>([^<]*)<\/title>/)?.[1] ?? '(no <title>)') : '(no page)'
  const ok = b.code === 0 && !threw.length && esTitle.includes('[es]')
  rec('probe: t() in <meta seo>', '1 app', ok ? 'PASS' : 'FAIL',
    ok ? `es <title> "${esTitle}"` : `<meta seo title={t('home.title')}>: build exit ${b.code}, ${threw.length} × "${threw[0] ?? ''}" → pages prerender empty, es <title> ${esTitle}; brain check says []`, 'COMPILER (seo object does not import t)')
  rec('localized <title> per locale', '2 browser', ok ? 'PASS' : 'FAIL',
    ok ? 'via t() in <meta seo>' : 'every locale shows the English <title>/description (literal <meta seo>), since t() in <meta seo> breaks the prerender', 'COMPILER')
}
{
  // b) <LangSwitch> from the kit in the layout
  const p = copyApp('probe-langswitch')
  edit(p, 'views/_layout.brain', (s) => s.replace(/<ul class="langs"[\s\S]*?<\/ul>\n/, '<LangSwitch />\n'))
  const c = brain(p, ['check', '--json', '.'])
  const b = brain(p, ['build'])
  const ok = b.code === 0
  rec('probe: <LangSwitch> builds', '1 app', ok ? 'PASS' : 'FAIL',
    ok ? 'layout with <LangSwitch /> builds' : `check → ${c.out.trim().slice(0, 60)}, build exit ${b.code}: ${b.out.match(/error:[^\n]*/)?.[0] ?? b.out.slice(-200)}`, 'RUNTIME (runtime/ui/lang-switch.brain keeps sig() state in ui/, B0116)')
}
{
  // c) t() in the app.brain shell (where the scaffold puts the nav)
  const p = copyApp('probe-shell-t')
  edit(p, 'app.brain', (s) => s.replace('  <slot />', "  <p class=\"shell-probe\">{t('site.brand')}</p>\n  <slot />"))
  const c = brain(p, ['check', '--json', '.'])
  const diags = parseJson(c.out) ?? []
  brain(p, ['build'])
  const es = existsSync(join(p, 'dist/es/index.html')) ? readFileSync(join(p, 'dist/es/index.html'), 'utf8') : ''
  const shown = es.match(/class="shell-probe">([^<]*)/)?.[1] ?? '(absent)'
  const ok = shown === 'Polyglot probe [es]' || (Array.isArray(diags) && diags.length > 0)
  rec('probe: t() in app.brain shell', '1 app', ok ? 'PASS' : 'FAIL',
    ok ? `es shell shows "${shown}"` : `es page shows the literal "${shown}" and brain check gives ${JSON.stringify(diags)}: the shell is not compiled, so neither t() nor <LangSwitch> work there and nothing warns`, 'COMPILER/BUILD')
}
{
  // d) a plain root-relative link in a localized page (the layout uses per-locale href keys as a workaround)
  const p = copyApp('probe-links')
  edit(p, 'views/home.brain', (s) => s.replace('</main>', '  <p><a class="plain" href="/about">{t(\'nav.about\')}</a></p>\n</main>'))
  brain(p, ['build'])
  const es = existsSync(join(p, 'dist/es/index.html')) ? readFileSync(join(p, 'dist/es/index.html'), 'utf8') : ''
  const href = es.match(/<a class="plain" href="([^"]*)"/)?.[1] ?? es.match(/href="([^"]*)" class="plain"/)?.[1] ?? '(absent)'
  rec('info: in-locale links', '1 app', 'INFO', `href="/about" on /es stays "${href}" (switches the visitor to English); no localized link helper exists, so the example keeps per-locale paths in nav.*_href keys`, 'RUNTIME/BUILD')
}
{
  // e) what the model reads: the default `brain spec` (the one an agent gets from brain_spec) must teach §37
  const def = brain(ROOT, ['spec']).out, all = brain(ROOT, ['spec', '--all']).out
  const lang = all.match(/<!--planned:#\d+-->\s*\n- Languages:/)
  const inDefault = /"i18n": \{ "default"/.test(def)
  rec('spec teaches i18n', '1 app', inDefault && !lang ? 'PASS' : 'FAIL',
    inDefault ? 'brain spec documents brain.json i18n' : `brain spec has no i18n section; brain spec --all has it inside ${lang ? lang[0].split('\n')[0] : 'a planned block'} (hidden by default, read as "not built yet")`, 'TOOLCHAIN #151 (spec)')
}
{
  const p = copyApp('probe-add-locale')
  const a = brain(p, ['add', 'locale', 'fr'])
  if (/unknown kind/.test(a.out)) rec('brain add locale', '1 app', 'BLOCKED', `brain add locale fr → ${a.out.trim().split('\n')[0]}`, 'TOOLCHAIN #151')
  else {
    const cfg = JSON.parse(readFileSync(join(p, 'brain.json'), 'utf8'))
    const fr = existsSync(join(p, 'i18n/fr.json')) ? JSON.parse(readFileSync(join(p, 'i18n/fr.json'), 'utf8')) : null
    const en = JSON.parse(readFileSync(join(p, 'i18n/en.json'), 'utf8'))
    const ok = a.code === 0 && cfg.i18n?.locales?.includes('fr') && fr && Object.keys(en).every((k) => k in fr)
    rec('brain add locale', '1 app', ok ? 'PASS' : 'FAIL', `exit ${a.code}, brain.json locales ${JSON.stringify(cfg.i18n?.locales)}, i18n/fr.json ${fr ? `${Object.keys(fr).length}/${Object.keys(en).length} keys` : 'missing'}`, 'TOOLCHAIN #151')
  }
  const m = brain(p, ['i18n', '--missing', '--json'])
  if (/unknown command/.test(m.out)) rec('brain i18n --missing', '1 app', 'BLOCKED', `→ ${m.out.trim().split('\n')[0]}`, 'TOOLCHAIN #151')
  else rec('brain i18n --missing', '1 app', m.code === 0 && parseJson(m.out) ? 'PASS' : 'FAIL', `exit ${m.code}: ${m.out.slice(0, 160)}`, 'TOOLCHAIN #151')
}

// ---------------------------------------------------------------- 2–3. build, serve, browser + network
const build = brain(APP, ['build'])
if (build.code !== 0) { rec('build', '2 browser', 'FAIL', build.out.slice(-300), 'BUILD'); finish() }
const DIST = join(APP, 'dist')
dicts = readdirSync(join(DIST, '_b')).filter((f) => /^i18n-.*\.js$/.test(f)).map((f) => ({
  file: f, bytes: statSync(join(DIST, '_b', f)).size, gz: existsSync(join(DIST, '_b', `${f}.gz`)) ? statSync(join(DIST, '_b', `${f}.gz`)).size : null,
  keys: (readFileSync(join(DIST, '_b', f), 'utf8').match(/"[a-z_]+\.[a-z_.]+":/g) ?? []).length,
}))

const freePort = (p) => new Promise((r) => { const s = createServer().once('error', () => r(false)).once('listening', () => s.close(() => r(true))).listen(p, '127.0.0.1') })
let PORT = Number(arg('port', 5331))
while (!(await freePort(PORT))) PORT++
const server = spawn(BRAIN, ['start', '--port', String(PORT)], { cwd: APP, stdio: ['ignore', 'pipe', 'pipe'] })
let serverLog = ''
server.stdout.on('data', (d) => (serverLog += d)); server.stderr.on('data', (d) => (serverLog += d))
stopServer = () => {
  if (server.exitCode != null) return
  try { process.kill(server.pid, 'SIGTERM') } catch { return }
  // wait (synchronously, so it also works from an exit handler) until the saved PID is gone; SIGKILL after 5 s
  const nap = new Int32Array(new SharedArrayBuffer(4))
  for (let i = 0; i < 100; i++) { try { process.kill(server.pid, 0) } catch { return } Atomics.wait(nap, 0, 0, 50) }
  try { process.kill(server.pid, 'SIGKILL') } catch {}
}
process.once('exit', stopServer)
const BASE = `http://127.0.0.1:${PORT}`
for (let i = 0; i < 100; i++) { try { if ((await fetch(BASE + '/')).ok) break } catch {} await sleep(100) }

// One fresh Chrome per page: empty cache, no service worker, so every request of that first visit is recorded.
async function visit(path, act) {
  const ch = await launch()
  try {
    const pg = await openPage(ch.port, 'about:blank')
    const reqs = new Map()
    let loaded = 0
    pg.on((m) => {
      if (m.method === 'Network.requestWillBeSent') reqs.set(m.params.requestId, { url: m.params.request.url.replace(BASE, ''), t: Date.now(), phase: 'open' })
      if (m.method === 'Network.responseReceived' && reqs.has(m.params.requestId)) reqs.get(m.params.requestId).status = m.params.response.status
      if (m.method === 'Network.loadingFinished' && reqs.has(m.params.requestId)) reqs.get(m.params.requestId).bytes = m.params.encodedDataLength
      if (m.method === 'Page.loadEventFired') loaded++
    })
    await pg.send('Network.enable')
    await pg.send('Page.navigate', { url: BASE + path })
    for (let i = 0; i < 100 && !loaded; i++) await sleep(50)
    await sleep(1200) // hydration + idle
    const out = await act(pg, reqs)
    pg.close()
    return { ...out, reqs: [...reqs.values()] }
  } finally { ch.close() }
}
const SNAP = `(() => {
  const r = (s) => document.querySelector(s)?.getBoundingClientRect()
  const tx = (s) => document.querySelector(s)?.textContent.trim() ?? null
  const el = document.documentElement
  return {
    path: location.pathname, lang: el.getAttribute('lang'), dir: el.getAttribute('dir'), def: el.getAttribute('data-default-lang'),
    title: document.title, h1: tx('h1'), body: document.body.innerText,
    alternates: [...document.querySelectorAll('link[rel=alternate][hreflang]')].map((l) => [l.getAttribute('hreflang'), new URL(l.href).pathname.replace(/\\/$/, '') || '/']),
    // structural selectors: brain build renames classes (#260), so .top/.brand only exist in dev
    brand: r('header > a')?.left, nav: r('header nav')?.left, cssDir: getComputedStyle(document.querySelector('header') ?? el).direction,
    hello: tx('#hello'), clicks: tx('#clicks'), date: tx('#date'), number: tx('#number'), price: tx('#price'),
    want: { date: new Intl.DateTimeFormat(el.lang, { dateStyle: 'medium' }).format(new Date('${RELEASED}')),
            number: new Intl.NumberFormat(el.lang).format(1234567.5),
            price: new Intl.NumberFormat(el.lang, { style: 'currency', currency: 'USD' }).format(1999.5) },
  }
})()`
const norm = (p) => p.replace(/\/$/, '') || '/'
const RAW_KEY = /\b(site|nav|home|about|counter|notfound|offline)\.[a-z_]+\b/

for (const loc of LOCALES) {
  for (const view of ['home', 'about', 'counter']) {
    const path = URLS[loc][view]
    const v = await visit(path, async (pg) => {
      const s = await pg.evaluate(SNAP)
      if (view !== 'counter') return { s }
      await pg.evaluate("document.querySelector('#add').click()")
      await sleep(250)
      return { s, after: await pg.evaluate("document.querySelector('#clicks').textContent.trim()") }
    })
    const { s } = v
    const id = `${loc} ${view}`
    const doc = v.reqs.find((r) => norm(r.url.split('?')[0]) === path)
    rec(`url ${id}`, '2 browser', doc?.status === 200 && norm(s.path) === path ? 'PASS' : 'FAIL', `${path} → ${doc?.status ?? 'no request'}, location ${s.path}`, 'BUILD')
    const dirOk = s.lang === loc && s.dir === (loc === 'ar' ? 'rtl' : 'ltr') && s.def === 'en'
    rec(`html lang/dir ${id}`, '2 browser', dirOk ? 'PASS' : 'FAIL', `<html lang="${s.lang}" dir="${s.dir}" data-default-lang="${s.def}">`, 'BUILD')
    const textOk = s.h1 === H1[view] + TAG[loc] && !RAW_KEY.test(s.body) && (loc === 'en' || !s.body.split('\n').some((l) => /^(Home|About us|Counter)$/.test(l.trim())))
    rec(`texts ${id}`, '2 browser', textOk ? 'PASS' : 'FAIL', `h1 "${s.h1}"${RAW_KEY.test(s.body) ? `, raw key visible: ${s.body.match(RAW_KEY)[0]}` : ''}`, 'RUNTIME')
    const alt = Object.fromEntries(s.alternates)
    const altOk = LOCALES.every((l) => alt[l] === URLS[l][view]) && alt['x-default'] === URLS.en[view] && s.alternates.length === 4
    rec(`hreflang ${id}`, '2 browser', altOk ? 'PASS' : 'FAIL', s.alternates.map(([l, h]) => `${l}=${h}`).join(' '), 'SEO #152')
    const mirrored = loc === 'ar' ? s.brand > s.nav && s.cssDir === 'rtl' : s.brand < s.nav && s.cssDir === 'ltr'
    rec(`layout direction ${id}`, '2 browser', mirrored ? 'PASS' : 'FAIL', `brand x=${Math.round(s.brand)} nav x=${Math.round(s.nav)} direction ${s.cssDir} (${loc === 'ar' ? 'brand must sit right of nav' : 'brand left of nav'})`, 'BUILD/CSS')
    if (view === 'counter') {
      const one = `1 click so far${TAG[loc]}`, two = `2 clicks so far${TAG[loc]}`
      rec(`plural ${id}`, '2 browser', s.clicks === one && v.after === two ? 'PASS' : 'FAIL', `"${s.clicks}" → click → "${v.after}"`, 'RUNTIME t()')
      rec(`placeholder ${id}`, '2 browser', s.hello === `Hello Ada, welcome back${TAG[loc]}` ? 'PASS' : 'FAIL', `"${s.hello}"`, 'RUNTIME t()')
      const want = { date: `Release date: ${s.want.date}${TAG[loc]}`, number: `Large number: ${s.want.number}${TAG[loc]}`, price: `Price: ${s.want.price}${TAG[loc]}` }
      const bad = Object.keys(want).filter((k) => s[k] !== want[k])
      rec(`fmt ${id}`, '2 browser', bad.length ? 'FAIL' : 'PASS', bad.length ? bad.map((k) => `${k} "${s[k]}" ≠ Intl "${want[k]}"`).join('; ') : `${s.want.date} · ${s.want.number} · ${s.want.price}`, 'RUNTIME fmt')
      // the prerendered HTML must already hold the same formatted text the browser shows (no flash on hydration)
      const html = readFileSync(join(DIST, path.slice(1), 'index.html'), 'utf8')
      const pre = Object.fromEntries(['date', 'number', 'price'].map((k) => [k, html.match(new RegExp(`<p id="${k}">([^<]*)`))?.[1]]))
      const diff = Object.keys(pre).filter((k) => pre[k] !== s[k])
      rec(`fmt prerender = browser ${id}`, '2 browser', diff.length ? 'FAIL' : 'PASS', diff.length ? diff.map((k) => `${k}: prerender "${pre[k]}" vs browser "${s[k]}"`).join('; ') : 'identical', 'BUILD prerender (Intl data)')
    }
    // 3. network of this first visit
    const d = v.reqs.filter((r) => r.url.includes('/_b/i18n-'))
    const wantDict = view === 'counter' ? [`i18n-counter-${loc}.`] : []
    const okNet = d.length === wantDict.length && wantDict.every((w) => d.some((r) => r.url.includes(w)))
    const other = v.reqs.filter((r) => LOCALES.filter((l) => l !== loc && l !== 'en').some((l) => r.url.startsWith(`/${l}/`) || r.url === `/${l}`) || (loc !== 'en' && /^\/(about|counter)?$/.test(r.url) && r.url !== path))
    rec(`network ${id}`, '3 network', okNet && !other.length ? 'PASS' : 'FAIL',
      `${v.reqs.length} requests · dictionaries ${d.length ? d.map((r) => `${r.url.split('/').pop()} (${r.bytes} B)`).join(', ') : 'none'}${view === 'counter' ? '' : ' (static page)'}${other.length ? ` · other-locale requests: ${other.map((r) => r.url).join(', ')}` : ''}`, 'BUILD brain-pack i18n')
    net.push({ page: path, requests: v.reqs.length, dictionaries: d.map((r) => ({ url: r.url, bytes: r.bytes })) })
  }
}

// Switching language: open /counter in English, hover the es link, then click it; then from Spanish to Arabic.
{
  const v = await visit('/counter', async (pg, reqs) => {
    const center = async (loc) => pg.evaluate(`(() => { const r = document.querySelector('header a[hreflang="${loc}"]').getBoundingClientRect(); return [r.x + r.width / 2, r.y + r.height / 2] })()`)
    const before = [...reqs.keys()]
    const [x, y] = await center('es')
    await pg.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x, y })
    await sleep(1500)
    const hover = [...reqs.entries()].filter(([k]) => !before.includes(k)).map(([, r]) => r.url)
    const beforeClick = [...reqs.keys()]
    await pg.send('Input.dispatchMouseEvent', { type: 'mousePressed', x, y, button: 'left', clickCount: 1 })
    await pg.send('Input.dispatchMouseEvent', { type: 'mouseReleased', x, y, button: 'left', clickCount: 1 })
    await sleep(1500)
    const s1 = await pg.evaluate(SNAP)
    const click = [...reqs.entries()].filter(([k]) => !beforeClick.includes(k)).map(([, r]) => r.url)
    const beforeAr = [...reqs.keys()]
    await pg.evaluate(`document.querySelector('header a[hreflang="ar"]').click()`)
    await sleep(1500)
    const s2 = await pg.evaluate(SNAP)
    const toAr = [...reqs.entries()].filter(([k]) => !beforeAr.includes(k)).map(([, r]) => r.url)
    return { open: before.map((k) => reqs.get(k).url), hover, click, s1, toAr, s2 }
  })
  const dictIn = (list, loc) => list.filter((u) => u.includes(`/_b/i18n-counter-${loc}.`))
  const okOpen = !v.open.some((u) => /i18n-counter-(es|ar)\./.test(u))
  const okClick = v.s1.lang === 'es' && norm(v.s1.path) === '/es/counter' && dictIn([...v.hover, ...v.click], 'es').length === 1 && v.s1.clicks === '1 click so far [es]'
  rec('switch: nothing before', '3 network', okOpen ? 'PASS' : 'FAIL', `open /counter: ${v.open.filter((u) => u.includes('i18n-')).map((u) => u.split('/').pop()).join(', ') || 'no dictionary'} of ${v.open.length} requests`, 'BUILD brain-pack i18n')
  rec('switch en → es (link)', '3 network', okClick ? 'PASS' : 'FAIL', `→ ${v.s1.path} lang=${v.s1.lang}, "${v.s1.clicks}"; es dictionary fetched on ${dictIn(v.hover, 'es').length ? 'HOVER' : 'click'} (${[...dictIn(v.hover, 'es'), ...dictIn(v.click, 'es')].map((u) => u.split('/').pop()).join(', ') || 'never'})`, 'RUNTIME router/LangSwitch')
  rec('info: hover prerender', '3 network', 'INFO', v.hover.length ? `hovering the es link (speculation rules, eagerness moderate) already downloads ${v.hover.length} requests: ${v.hover.join(', ')}` : 'hovering the es link downloads nothing', 'BUILD')
  const okAr = v.s2.lang === 'ar' && v.s2.dir === 'rtl' && norm(v.s2.path) === '/ar/counter' && dictIn(v.toAr, 'ar').length === 1 && v.s2.brand > v.s2.nav
  rec('switch es → ar (dir flips)', '3 network', okAr ? 'PASS' : 'FAIL', `→ ${v.s2.path} lang=${v.s2.lang} dir=${v.s2.dir}, brand x=${Math.round(v.s2.brand)} nav x=${Math.round(v.s2.nav)}, ar dictionary ${dictIn(v.toAr, 'ar').length ? 'loaded then' : 'not loaded'}`, 'RUNTIME router')
}
stopServer()

// ---------------------------------------------------------------- 4. LLM gate
const MODELS = [['claude-sonnet-5-5', 'i18n153-sonnet'], ['claude-haiku-5-5', 'i18n153-haiku']]
for (const [model, tag] of MODELS) {
  const report = join(ROOT, 'bench/autonomy/results', `${tag}.json`)
  if (RUN_LLM) spawnSync('node', [join(ROOT, 'bench/autonomy/run.mjs'), '--model', model, '--mode', 'mcp', '--briefs', '../i18n/briefs-153.json', '--tag', tag], { stdio: 'inherit' })
  if (!existsSync(report)) { rec(`llm ${tag}`, '4 llm', 'BLOCKED', `no ${home(report)}: run with --llm`, 'QA'); continue }
  const rep = JSON.parse(readFileSync(report, 'utf8'))
  for (const row of rep.rows) {
    const dir = join(ROOT, 'bench/autonomy/results', tag, row.id)
    const cfg = existsSync(join(dir, 'brain.json')) ? JSON.parse(readFileSync(join(dir, 'brain.json'), 'utf8')) : {}
    const locs = cfg.i18n?.locales ?? []
    const files = ['en', 'es'].filter((l) => existsSync(join(dir, `i18n/${l}.json`)))
    const c = existsSync(join(dir, 'app.brain')) ? brain(dir, ['check', '--json', '.']) : { out: '[]' }
    const diags = parseJson(c.out) ?? []
    const codes = [...new Set(diags.map((d) => d.code))]
    const b1403 = diags.filter((d) => d.code === 'B1403').length
    const tr = existsSync(join(dir, '..', `${row.id}.transcript.jsonl`)) ? readFileSync(join(dir, '..', `${row.id}.transcript.jsonl`), 'utf8') : ''
    const uses = tr.split('\n').flatMap((l) => { try { return JSON.parse(l).message?.content ?? [] } catch { return [] } }).filter((x) => x.type === 'tool_use')
    const addLocale = uses.filter((u) => /locale/.test(JSON.stringify(u.input ?? {})) && /add/.test(u.name))
    const addLocaleErr = /unknown kind `locale`/.test(tr)
    const views = (() => { try { return readdirSync(join(dir, 'views')).filter((f) => f.endsWith('.brain') && !f.startsWith('_') && !/^(40\d|500)/.test(f)) } catch { return [] } })()
    const src = views.map((f) => readFileSync(join(dir, 'views', f), 'utf8')).join('\n') + (existsSync(join(dir, 'app.brain')) ? readFileSync(join(dir, 'app.brain'), 'utf8') : '') + (existsSync(join(dir, 'views/_layout.brain')) ? readFileSync(join(dir, 'views/_layout.brain'), 'utf8') : '')
    const sw = /<LangSwitch|alternates\(|hreflang|lang=|\/es/.test(src)
    const minScore = cfg.seo?.minScore
    const own = !cfg.i18n && /planned|not built/.test(row.final ?? '') ? 'own i18n helper (spec says planned)' : null
    const checks = {
      i18n: cfg.i18n?.default === 'en' && locs.includes('en') && locs.includes('es'),
      files: files.length === 2,
      noB1403: b1403 === 0,
      doctor: row.doctor?.exit === 0,
      routes: row.views >= row.pages,
    }
    const pass = Object.values(checks).every(Boolean)
    const ev = `i18n ${JSON.stringify(cfg.i18n ?? null)} · files ${files.join('+') || 'none'} · check ${codes.join(' ') || '[]'}${b1403 ? ` (B1403 ×${b1403})` : ''} · doctor ${row.doctor?.exit}${row.doctor?.errors?.length ? ` ${row.doctor.errors.join(' ')}` : ''} · views ${row.views}/${row.pages} · switch ${sw ? 'yes' : 'no'} · add locale ${addLocale.length ? `called ×${addLocale.length}${addLocaleErr ? ' (unknown kind)' : ''}` : 'not called'} · seo.minScore ${minScore}${minScore < 80 ? ' (lowered from 80)' : ''}${own ? ` · ${own}` : ''} · ${row.toolCalls} calls · ${Math.round(row.ms / 1000)} s`
    rec(`llm ${model}`, '4 llm', pass ? 'PASS' : 'FAIL', ev, pass ? null : !checks.i18n ? 'TOOLCHAIN #151 (spec hides §37 as planned)' : !checks.noB1403 ? 'COMPILER/LLM (hardcoded text)' : !checks.doctor ? 'TOOLCHAIN/LLM' : 'LLM')
    llm.push({ model, tag, id: row.id, brainSha: rep.brainSha, specSha: rep.specSha, checks, codes, b1403, languageSwitch: sw, addLocaleCalls: addLocale.length, seoMinScore: minScore, ownI18n: !!own, addLocaleUnknown: addLocaleErr, toolCalls: row.toolCalls, calls: row.calls, seconds: Math.round(row.ms / 1000), cost: row.cost, final: row.final })
  }
}

finish()

function finish() {
  stopServer()
  const count = (st) => results.filter((r) => r.status === st).length
  const summary = {
    task: 153, at: new Date().toISOString(), brainSha: BRAIN_SHA, scratch: home(SCRATCH),
    totals: { PASS: count('PASS'), FAIL: count('FAIL'), BLOCKED: count('BLOCKED'), INFO: count('INFO') },
    results, dictionaries: dicts, network: net, llm,
  }
  mkdirSync(join(HERE, 'results'), { recursive: true })
  writeFileSync(join(HERE, 'results/summary.json'), JSON.stringify(summary, null, 1).replaceAll(homedir(), '~') + '\n')
  const md = [
    `# QA #153 — i18n results`, ``,
    `brain \`${BRAIN_SHA}\` · ${summary.at} · PASS ${summary.totals.PASS} · FAIL ${summary.totals.FAIL} · BLOCKED ${summary.totals.BLOCKED} · INFO ${summary.totals.INFO}`, ``,
    `| group | check | status | evidence | owner |`, `|---|---|---|---|---|`,
    ...results.map((r) => `| ${r.group} | ${r.id} | ${r.status} | ${r.evidence.replaceAll('|', '\\|')} | ${r.status === 'PASS' ? '' : r.owner ?? ''} |`),
    ``, `## Dictionary chunks`, ``, `| chunk | bytes | gzip | keys |`, `|---|---:|---:|---:|`,
    ...summary.dictionaries.map((d) => `| ${d.file} | ${d.bytes} | ${d.gz ?? '—'} | ${d.keys} |`),
  ].join('\n') + '\n'
  writeFileSync(join(HERE, 'results/summary.md'), md.replaceAll(homedir(), '~'))
  console.log(`\nPASS ${summary.totals.PASS} · FAIL ${summary.totals.FAIL} · BLOCKED ${summary.totals.BLOCKED} · INFO ${summary.totals.INFO} → bench/i18n/results/summary.{json,md}`)
  process.exit(summary.totals.FAIL ? 1 : 0)
}
