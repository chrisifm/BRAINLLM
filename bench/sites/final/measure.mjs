// #21 final — measures one generated app (bench/sites/PROTOCOL.md §3), on a scratch copy so the generated sources stay intact.
// doctor + test · build cold/warm (hyperfine) · dist rules (INTERNALS §10) · Content-Type and encoding served by
// `brain start` · bytes per page raw/gz/br (resources really loaded in headless Chrome) · HMR on one component
// (style + script edits, bench/hmr method) · Lighthouse a11y/SEO + `brain seo audit` per page · functional smoke.
//
// Usage: node bench/sites/final/measure.mjs --app <generated app dir> --site landing|panel --label sonnet
//          [--port 5310] [--reps 20] [--runs 10] [--scratch ~/tmp/brainllm-final21] [--skip lighthouse,hmr]
// Binary: $BRAIN (default target/release/brain). Output: bench/sites/final/results/<label>-<site>.json
import { execFileSync, spawn } from 'node:child_process'
import { readFileSync, readdirSync, statSync, existsSync, writeFileSync, mkdirSync, rmSync, cpSync } from 'node:fs'
import { join, relative, extname, resolve } from 'node:path'
import { homedir } from 'node:os'
import { gzipSync, brotliCompressSync, constants } from 'node:zlib'
import { createHash } from 'node:crypto'
import { launch, openPage } from '../../lib/cdp.mjs'
import { smoke } from './smoke.mjs'

const ROOT = new URL('../../..', import.meta.url).pathname
const HERE = join(ROOT, 'bench/sites/final')
const argv = process.argv.slice(2)
const arg = (k, d) => { const i = argv.indexOf(`--${k}`); return i >= 0 ? argv[i + 1] : d }
const SRC = resolve(arg('app')), SITE = arg('site'), LABEL = arg('label', 'run')
const PORT = Number(arg('port', 5310)), REPS = Number(arg('reps', 20)), RUNS = arg('runs', '10')
const SKIP = (arg('skip', '') || '').split(',')
const BRAIN = process.env.BRAIN ?? join(ROOT, 'target/release/brain')
const SCRATCH = arg('scratch', join(homedir(), 'tmp/brainllm-final21'))
const APP = join(SCRATCH, `${LABEL}-${SITE}`)
const LH_VERSION = '13.5.0' // same pin as bench/a11y-seo/lighthouse.sh
const sleep = (ms) => new Promise((r) => setTimeout(r, ms))
const home = (s) => (typeof s === 'string' ? s.split(homedir()).join('~') : s)
const sha = (buf) => createHash('sha256').update(buf).digest('hex').slice(0, 12)
const gz9 = (b) => gzipSync(b, { level: constants.Z_BEST_COMPRESSION }).length
const br11 = (b) => brotliCompressSync(b, { params: { [constants.BROTLI_PARAM_QUALITY]: 11, [constants.BROTLI_PARAM_SIZE_HINT]: b.length } }).length
const walk = (d) => readdirSync(d).flatMap((f) => { const p = join(d, f); return statSync(p).isDirectory() ? walk(p) : [p] })
const sh = (cmd, args, opts = {}) => {
  try { return { ok: true, code: 0, out: execFileSync(cmd, args, { cwd: APP, maxBuffer: 64 << 20, ...opts }).toString() } }
  catch (e) { return { ok: false, code: e.status, out: (e.stdout ?? '').toString(), err: (e.stderr ?? '').toString() } }
}
const stats = (xs) => {
  if (!xs.length) return null
  const s = [...xs].sort((a, b) => a - b), q = (p) => +s[Math.min(s.length - 1, Math.ceil(p * s.length) - 1)].toFixed(2)
  return { n: s.length, median: q(0.5), p95: q(0.95), min: +s[0].toFixed(2), max: +s.at(-1).toFixed(2) }
}
// Every child process is tracked by PID and killed on exit: nothing is left running.
const children = new Set()
const track = (p) => { children.add(p); p.on('exit', () => children.delete(p)); return p }
process.on('exit', () => { for (const p of children) try { p.kill('SIGTERM') } catch {} })
for (const s of ['SIGINT', 'SIGTERM']) process.on(s, () => process.exit(130))

// 0. Scratch copy: sources only.
rmSync(APP, { recursive: true, force: true }); mkdirSync(APP, { recursive: true })
cpSync(SRC, APP, { recursive: true, filter: (p) => !/(^|\/)(dist|\.brain|node_modules|logs)(\/|$)/.test(relative(SRC, p)) })
const R = { site: SITE, label: LABEL, at: new Date().toISOString(), src: home(SRC), scratch: home(APP),
  brainSha: sha(readFileSync(BRAIN)), specSha: sha(execFileSync(BRAIN, ['spec'])), brain: home(BRAIN) }
const log = (m) => console.error(`[${LABEL}-${SITE}] ${m}`)

// 1. doctor + test.
log('doctor')
{
  const t0 = Date.now(), d = sh(BRAIN, ['doctor', '--json'], { timeout: 600000 })
  let j = null; try { j = JSON.parse(d.out) } catch {}
  R.doctor = { exit: d.code, ms: Date.now() - t0, errors: j?.check?.errors, warnings: j?.check?.warnings,
    diags: (j?.check?.diags ?? []).map((x) => `${x.code} ${x.file}:${x.line} ${x.msg}`).slice(0, 30),
    seo: j?.seo?.pages?.filter((p) => p.indexable).map((p) => ({ route: p.route, score: p.score })) }
  if (!j) R.doctor.raw = (d.out + d.err).slice(-600)
}
log('test')
{
  const t = sh(BRAIN, ['test', '--json', '.'], { timeout: 300000 })
  try {
    const j = JSON.parse(t.out), all = j.flatMap((f) => f.tests ?? [])
    R.test = { exit: t.code, files: j.length, tests: all.length, passed: all.filter((x) => x.ok).length,
      failed: all.filter((x) => !x.ok).map((x) => x.name), a11y: j.flatMap((f) => f.a11y ?? []).map((a) => a.code ?? a) }
  } catch { R.test = { exit: t.code, raw: (t.out + t.err).slice(-600) } }
}

// 1b. Production build of the app exactly as generated. If it fails only on broken links/anchors (B1819/B1820), record
// the failure and rewrite exactly those hrefs to `/` in the scratch copy, so the rest can still be measured. The patch is recorded verbatim.
{
  const g = sh(BRAIN, ['build'])
  R.buildAsGenerated = { exit: g.code, errors: [...new Set((g.out + g.err).split('\n').filter((l) => /^B\d{4}/.test(l)))].slice(0, 20) }
  const bad = [...new Set([...(g.out + g.err).matchAll(/B18(?:19|20) \S+: (?:anchor|link) `([^`]+)`/g)].map((m) => m[1]))]
  const other = R.buildAsGenerated.errors.filter((l) => !/^B18(19|20)/.test(l))
  if (!g.ok && bad.length && !other.length) {
    const edits = []
    for (const f of walk(APP).filter((p) => p.endsWith('.brain') && !p.includes('/dist/') && !p.includes('/.brain/'))) {
      const s = readFileSync(f, 'utf8'); let n = 0
      const t = s.replace(/href="([^"]+)"/g, (m, h) => bad.includes(h) ? (n++, 'href="/"') : m)
      if (n) { writeFileSync(f, t); edits.push({ file: relative(APP, f), hrefsRewritten: n }) }
    }
    R.patch = { reason: 'brain build fails with B1819/B1820 on the app as generated (doctor exits 0)', rewrite: bad.map((h) => `${h} → /`), edits }
    rmSync(join(APP, 'dist'), { recursive: true, force: true })
  }
}

// 2. Build time (hyperfine -N, RUNS runs). Cold: dist and the .brain cache removed before every run.
log('build timing')
const hf = join(APP, '.qa-hyperfine.json')
const hyper = (extra) => {
  const r = sh('hyperfine', ['-N', '--runs', RUNS, ...extra, '--export-json', hf, `${BRAIN} build`], { timeout: 900000 })
  if (!r.ok) return { error: (r.out + r.err).slice(-400) }
  const x = JSON.parse(readFileSync(hf, 'utf8')).results[0], ms = x.times.map((t) => t * 1000)
  return { meanMs: +(x.mean * 1000).toFixed(1), stddevMs: +(x.stddev * 1000).toFixed(1), ...stats(ms) }
}
R.buildCold = hyper(['--warmup', '1', '--prepare', 'rm -rf dist .brain'])
R.buildWarm = hyper(['--warmup', '2'])
rmSync(hf, { force: true })
const b = sh(BRAIN, ['build', '--json'])
let build = null; try { build = JSON.parse(b.out.trim().split('\n').at(-1)) } catch {}
R.buildId = build?.build ?? null
if (!b.ok) R.buildError = (b.out + b.err).slice(-600)

// 3. dist rules (INTERNALS §10, current text): web extensions by default; no source extensions shipped; a .gz next to
// every text file; nginx/Caddy/_headers snippets present; no source maps.
const DIST = join(APP, 'dist')
const files = existsSync(DIST) ? walk(DIST).map((p) => relative(DIST, p)) : []
const TEXT = /\.(html|js|mjs|css|json|txt|xml|md|svg|webmanifest)$/
const content = files.filter((f) => !/\.(br|gz)$/.test(f))
R.dist = {
  files: content.length,
  sourceExtLeaked: content.filter((f) => /\.(brain|jsbrain|cssbrain)$/.test(f)),
  sourceMaps: content.filter((f) => f.endsWith('.map')),
  missingGz: content.filter((f) => TEXT.test(f) && !f.startsWith('_server/') && !files.includes(`${f}.gz`)).map((f) => { const b = readFileSync(join(DIST, f)); return { file: f, raw: b.length, gz9: gz9(b) } }).filter((x) => x.gz9 < x.raw),
  brSiblings: files.filter((f) => f.endsWith('.br')).length,
  snippets: { nginx: files.includes('_brain/nginx.conf'), caddy: files.includes('_brain/Caddyfile'), headers: files.includes('_headers') },
  totalShipped: (() => { const c = content.filter((f) => !f.startsWith('_server/')); const raw = c.reduce((s, f) => s + statSync(join(DIST, f)).size, 0); return { files: c.length, raw } })(),
}
R.dist.ok = !R.dist.sourceExtLeaked.length && !R.dist.sourceMaps.length && !R.dist.missingGz.length && R.dist.snippets.nginx

// Routes: every prerendered page except the error/offline ones.
const pages = content.filter((f) => /(^|\/)index\.html$/.test(f) && !/^(_|403\/|404)/.test(f))
  .map((f) => '/' + f.replace(/index\.html$/, '').replace(/\/$/, ''))
R.routes = pages

// 4. Production server: `brain start`.
const startServer = async () => {
  const p = track(spawn(BRAIN, ['start', '--port', String(PORT)], { cwd: APP, stdio: ['ignore', 'pipe', 'pipe'] }))
  let up = false
  for (let i = 0; i < 150 && !up; i++) { try { await fetch(`http://127.0.0.1:${PORT}/`); up = true } catch { await sleep(100) } }
  return { p, up }
}
const stopServer = async (srv) => { sh(BRAIN, ['stop']); await sleep(500); try { srv.p.kill('SIGTERM') } catch {}; await sleep(300) }
const distFile = (path) => {
  const p = decodeURIComponent(path).replace(/^\//, '')
  for (const c of [p, `${p}/index.html`.replace(/^\//, ''), `${p}.html`, p === '' ? 'index.html' : null].filter(Boolean)) if (existsSync(join(DIST, c)) && statSync(join(DIST, c)).isFile()) return c
  return null
}
const kind = (f, mime) => /\.html$/.test(f ?? '') || /html/.test(mime) ? 'html' : /\.m?js$/.test(f ?? '') || /javascript/.test(mime) ? 'js' : /\.css$/.test(f ?? '') || /css/.test(mime) ? 'css' : 'other'

log('brain start')
const srv = await startServer()
R.start = { up: srv.up }
const chrome = await launch()
try {
  // 4a. Content-Type + Content-Encoding of every route and every dist asset under _b/.
  const want = { html: 'text/html', js: 'text/javascript', css: 'text/css' }
  const probe = [...pages, ...content.filter((f) => f.startsWith('_b/') && /\.(js|css)$/.test(f)).map((f) => `/${f}`)]
  R.contentType = { checked: 0, bad: [], encodings: {} }
  for (const u of probe) {
    const r = await fetch(`http://127.0.0.1:${PORT}${u}`, { headers: { 'accept-encoding': 'gzip, br' } })
    await r.arrayBuffer()
    const ct = r.headers.get('content-type') ?? '', k = u.startsWith('/_b/') ? extname(u).slice(1) : 'html'
    const enc = r.headers.get('content-encoding') ?? 'identity'
    R.contentType.encodings[enc] = (R.contentType.encodings[enc] ?? 0) + 1
    R.contentType.checked++
    if (r.status !== 200 || !ct.startsWith(want[k])) R.contentType.bad.push({ url: u, status: r.status, ct })
  }
  R.contentType.ok = R.start.up && R.contentType.bad.length === 0

  // 4b. Bytes per page: every same-origin resource Chrome loads for the route (initial load + 1.5 s idle).
  R.bytes = {}
  for (const route of pages) {
    const page = await openPage(chrome.port, 'about:blank')
    const reqs = new Map()
    page.on((m) => {
      if (m.method === 'Network.responseReceived') reqs.set(m.params.requestId, { url: m.params.response.url, mime: m.params.response.mimeType, enc: 0 })
      if (m.method === 'Network.loadingFinished' && reqs.has(m.params.requestId)) reqs.get(m.params.requestId).enc = m.params.encodedDataLength
    })
    await page.send('Network.enable'); await page.send('Network.setCacheDisabled', { cacheDisabled: true })
    await page.send('Page.navigate', { url: `http://127.0.0.1:${PORT}${route}` })
    await sleep(2500)
    const rows = []
    for (const r of reqs.values()) {
      const u = new URL(r.url); if (u.origin !== `http://127.0.0.1:${PORT}`) continue
      const f = distFile(u.pathname); if (!f) { rows.push({ path: u.pathname, missing: true }); continue }
      const buf = readFileSync(join(DIST, f))
      rows.push({ path: u.pathname, file: f, kind: kind(f, r.mime), raw: buf.length, gz: existsSync(join(DIST, `${f}.gz`)) ? statSync(join(DIST, `${f}.gz`)).size : gz9(buf), br: br11(buf), transferred: r.enc })
    }
    const by = (k) => { const x = rows.filter((r) => r.kind === k); return { files: x.length, raw: x.reduce((s, r) => s + r.raw, 0), gz: x.reduce((s, r) => s + r.gz, 0), br: x.reduce((s, r) => s + r.br, 0) } }
    const tot = (k) => rows.filter((r) => !r.missing && r.kind !== 'other').reduce((s, r) => s + r[k], 0)
    R.bytes[route] = { html: by('html'), js: by('js'), css: by('css'), other: by('other'),
      totalHtmlJsCss: { raw: tot('raw'), gz: tot('gz'), br: tot('br') }, transferred: rows.reduce((s, r) => s + (r.transferred ?? 0), 0),
      firstRouteJs: by('js'), requests: rows.length, missing: rows.filter((r) => r.missing).map((r) => r.path), resources: rows }
    page.close()
  }

  // 5. Lighthouse a11y + SEO on each page of the production build, and `brain seo audit`.
  if (!SKIP.includes('lighthouse')) {
    log('lighthouse')
    R.lighthouse = {}
    mkdirSync(join(HERE, 'results/lighthouse'), { recursive: true })
    for (const route of pages) {
      const out = join(HERE, 'results/lighthouse', `${LABEL}-${SITE}${route.replace(/\//g, '_') || '_'}.json`)
      const r = sh('npx', ['--yes', `lighthouse@${LH_VERSION}`, `http://127.0.0.1:${PORT}${route}`, '--only-categories=accessibility,seo',
        '--output=json', `--output-path=${out}`, '--chrome-flags=--headless=new --use-mock-keychain --password-store=basic', '--quiet'], { timeout: 300000 })
      try {
        const j = JSON.parse(readFileSync(out, 'utf8'))
        R.lighthouse[route] = { version: j.lighthouseVersion, a11y: Math.round(j.categories.accessibility.score * 100), seo: Math.round(j.categories.seo.score * 100),
          failing: Object.values(j.audits).filter((a) => a.score === 0).map((a) => a.id) }
        writeFileSync(out, JSON.stringify(JSON.parse(home(JSON.stringify(j))))) // homedir out of the stored report
      } catch { R.lighthouse[route] = { error: (r.out + r.err).slice(-300) } }
    }
  }
  {
    const s = sh(BRAIN, ['seo', 'audit', '--json'])
    try { const j = JSON.parse(s.out); R.brainSeo = { exit: s.code, minScore: j.minScore, pages: j.pages.filter((p) => p.indexable).map((p) => ({ route: p.route, score: p.score, findings: p.findings.map((f) => `${f.code}(${f.level})`) })) } }
    catch { R.brainSeo = { exit: s.code, raw: (s.out + s.err).slice(-300) } }
  }

  // 6. Functional smoke in headless Chrome against the production build.
  if (!SKIP.includes('smoke')) { log('smoke'); R.smoke = await smoke(SITE, chrome.port, `http://127.0.0.1:${PORT}`, pages) }
} finally { await stopServer(srv) }

// 7. HMR in dev (bench/hmr/run.mjs method): REPS style edits and REPS script edits on one component, with state.
if (!SKIP.includes('hmr')) {
  log('hmr')
  const { hmrTarget } = await import('./smoke.mjs')
  const tgt = hmrTarget(SITE, APP)
  R.hmr = { component: tgt?.file ?? null }
  if (!tgt) R.hmr.error = 'no HMR target found'
  else {
    const FILE = join(APP, tgt.file), original = readFileSync(FILE, 'utf8')
    const dev = track(spawn(BRAIN, ['run', '--port', String(PORT + 1)], { cwd: APP, stdio: ['ignore', 'pipe', 'pipe'] }))
    const compiles = [], errs = []
    const onLog = (d) => { for (const m of String(d).matchAll(/\[hmr\] \S+ ([\d.]+) ms/g)) compiles.push(Number(m[1])); for (const m of String(d).matchAll(/\[hmr\] (.+error.+)/g)) errs.push(m[1]) }
    dev.stdout.on('data', onLog); dev.stderr.on('data', onLog)
    try {
      for (let i = 0; i < 150; i++) { try { await fetch(`http://127.0.0.1:${PORT + 1}/`); break } catch { await sleep(100) } }
      const page = await openPage(chrome.port, `http://127.0.0.1:${PORT + 1}${tgt.route}`)
      await sleep(1500)
      const arm = () => page.evaluate(`window.__qa = []; window.__qa_armed || addEventListener('brain:hmr', (e) => __qa.push(e.detail)); window.__qa_armed = true; true`)
      await arm()
      const origin0 = await page.evaluate('performance.timeOrigin')
      R.hmr.stateBefore = await page.evaluate(tgt.setState)
      const waitEvent = async (timeout = 5000) => {
        for (let t = 0; t < timeout; t += 2) {
          const ev = await page.evaluate('window.__qa && __qa.length ? __qa.shift() : null').catch(() => 'gone')
          if (ev === 'gone') return { kind: 'reload-detected' }
          if (ev) return ev
          await sleep(2)
        }
        return null
      }
      for (const [k, edit] of Object.entries({ style: tgt.style, script: tgt.script })) {
        writeFileSync(FILE, original); await sleep(500); await arm()
        const rows = []
        for (let i = 0; i < REPS; i++) {
          await sleep(120)
          const c0 = compiles.length, writtenAt = Date.now()
          writeFileSync(FILE, edit(original, i))
          const ev = await waitEvent()
          for (let t = 0; t < 100 && compiles.length === c0; t += 5) await sleep(5)
          const compile = compiles.length > c0 ? compiles.at(-1) : null
          if (!ev || ev.kind === 'reload-detected') { rows.push({ i, error: ev ? ev.kind : 'timeout' }); continue }
          rows.push({ i, kind: ev.kind, total: ev.appliedAt - writtenAt, client: ev.appliedAt - ev.receivedAt, compile, tool: compile === null ? null : compile + (ev.appliedAt - ev.receivedAt) })
        }
        const ok = rows.filter((r) => r.total !== undefined)
        R.hmr[k] = { applied: [...new Set(ok.map((r) => r.kind))], tool: stats(ok.filter((r) => r.tool !== null).map((r) => r.tool)),
          compile: stats(ok.filter((r) => r.compile !== null).map((r) => r.compile)), client: stats(ok.map((r) => r.client)),
          total: stats(ok.map((r) => r.total)), errors: rows.filter((r) => r.error).length }
      }
      writeFileSync(FILE, original); await sleep(500)
      R.hmr.stateAfter = await page.evaluate(tgt.readState).catch(() => null)
      R.hmr.statePreserved = R.hmr.stateAfter === tgt.expected
      R.hmr.reloaded = (await page.evaluate('performance.timeOrigin').catch(() => null)) !== origin0
      R.hmr.compileErrors = errs.slice(0, 5)
      page.close()
    } finally { writeFileSync(FILE, original); try { dev.kill('SIGTERM') } catch {} }
  }
}
chrome.close()

mkdirSync(join(HERE, 'results'), { recursive: true })
const json = home(JSON.stringify(R, null, 1))
writeFileSync(join(HERE, 'results', `${LABEL}-${SITE}.json`), json)
console.log(json)
process.exit(0)
