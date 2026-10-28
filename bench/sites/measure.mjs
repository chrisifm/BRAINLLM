// #21 — per-site measurements after generation: build time (cold/warm, hyperfine), bytes gz per file and per view,
// runtime size, SPEC §10 dist rules (no .html/.js/.css, .br/.gz siblings, Content-Type served), `brain test` + AX.
// Usage: node bench/sites/measure.mjs --app <dir> [--port 5192]
import { execFileSync, spawn } from 'node:child_process'
import { readFileSync, readdirSync, statSync, existsSync, writeFileSync, mkdirSync } from 'node:fs'
import { join, relative, extname, basename, resolve } from 'node:path'
import { gzipSync, constants } from 'node:zlib'

const ROOT = new URL('../..', import.meta.url).pathname
const BRAIN = process.env.BRAIN ?? join(ROOT, 'target/debug/brain')
const argv = process.argv.slice(2)
const arg = (k, d) => { const i = argv.indexOf(`--${k}`); return i >= 0 ? argv[i + 1] : d }
const APP = arg('app') && resolve(arg('app')), PORT = Number(arg('port', 5192))
if (!APP) throw Error('--app <dir> is required')
const DIST = join(APP, 'dist')
const sleep = (ms) => new Promise((r) => setTimeout(r, ms))
const gz = (buf) => gzipSync(buf, { level: constants.Z_BEST_COMPRESSION }).length
const walk = (d) => readdirSync(d).flatMap((f) => { const p = join(d, f); return statSync(p).isDirectory() ? walk(p) : [p] })
const sh = (cmd, args, opts = {}) => { try { return { ok: true, out: execFileSync(cmd, args, { cwd: APP, ...opts }).toString() } } catch (e) { return { ok: false, code: e.status, out: (e.stdout ?? '').toString() + (e.stderr ?? '').toString() } } }
const result = { app: APP, at: new Date().toISOString(), brain: BRAIN }

// 1. Build time: cold = dist and caches removed before every run; warm = repeated build over an existing dist.
const hf = join(APP, '.qa-hyperfine.json')
const cold = sh('hyperfine', ['-N', '--warmup', '1', '--runs', '10', '--prepare', 'rm -rf dist .brain', '--export-json', hf, `${BRAIN} build`])
result.buildCold = cold.ok ? JSON.parse(readFileSync(hf, 'utf8')).results[0] : { error: cold.out.slice(-400) }
const warm = sh('hyperfine', ['-N', '--warmup', '2', '--runs', '10', '--export-json', hf, `${BRAIN} build`])
result.buildWarm = warm.ok ? JSON.parse(readFileSync(hf, 'utf8')).results[0] : { error: warm.out.slice(-400) }
for (const k of ['buildCold', 'buildWarm']) if (result[k].mean) result[k] = { meanMs: +(result[k].mean * 1000).toFixed(1), stddevMs: +(result[k].stddev * 1000).toFixed(1), minMs: +(result[k].min * 1000).toFixed(1), runs: result[k].times.length }

// 2. dist/ rules (SPEC §10) and sizes.
if (existsSync(DIST)) {
  const files = walk(DIST).map((p) => relative(DIST, p))
  const content = files.filter((f) => !/\.(br|gz)$/.test(f))
  result.dist = {
    files: content.length,
    forbidden: content.filter((f) => ['.html', '.js', '.css'].includes(extname(f))),
    missingPrecompressed: content.filter((f) => /\.(brain|jsbrain|cssbrain)$/.test(f) && !(files.includes(`${f}.br`) && files.includes(`${f}.gz`))),
    hasNginxConf: files.includes('brain.nginx.conf'),
    sizes: Object.fromEntries(content.filter((f) => /\.(brain|jsbrain|cssbrain)$/.test(f)).map((f) => { const b = readFileSync(join(DIST, f)); return [f, { raw: b.length, gz: gz(b) }] })),
  }
  // per view: the page + every /_b/ asset it references
  result.views = Object.fromEntries(content.filter((f) => basename(f) === 'index.brain').map((page) => {
    const html = readFileSync(join(DIST, page), 'utf8')
    const assets = [...new Set([...html.matchAll(/\/_b\/[\w.-]+\.(?:jsbrain|cssbrain)/g)].map((m) => m[0].slice(1)))]
    const total = [page, ...assets].filter((a) => existsSync(join(DIST, a))).reduce((s, a) => s + gz(readFileSync(join(DIST, a))), 0)
    return [`/${page.replace(/index\.brain$/, '')}`, { assets: assets.length, gzTotal: total }]
  }))
} else result.dist = { error: 'dist/ does not exist (brain build did not generate it)' }

// 3. Runtime size: shipped (minified) runtime chunk if present, else source runtime for reference.
const rt = join(ROOT, 'runtime/brain.js')
result.runtime = { sourceRaw: statSync(rt).size, sourceGz: gz(readFileSync(rt)) }

// 4. Content-Type served by `brain dev` for each dist-style extension and for the app's real routes.
const dev = spawn(BRAIN, ['dev', '--port', String(PORT)], { cwd: APP, stdio: 'ignore' })
try {
  for (let i = 0; i < 100; i++) { try { await fetch(`http://127.0.0.1:${PORT}/`); break } catch { await sleep(100) } }
  const want = { brain: 'text/html', jsbrain: 'text/javascript', cssbrain: 'text/css' }
  const urls = ['/', ...(result.dist?.sizes ? Object.keys(result.dist.sizes).filter((f) => f.startsWith('_b/')).slice(0, 10).map((f) => `/${f}`) : [])]
  result.contentType = []
  for (const u of urls) {
    const r = await fetch(`http://127.0.0.1:${PORT}${u}`)
    const ext = u === '/' ? 'brain' : extname(u).slice(1)
    const ct = r.headers.get('content-type') ?? ''
    result.contentType.push({ url: u, status: r.status, contentType: ct, ok: r.ok && ct.startsWith(want[ext] ?? '?'), poweredBy: r.headers.get('x-powered-by') })
  }
} finally { dev.kill() }

// 5. brain test (+ AX audit) as JSON.
const t = sh(BRAIN, ['test', '--json', '.'], { timeout: 180000 })
try { const j = JSON.parse(t.out); result.test = { exit: t.ok ? 0 : t.code, files: j.length, tests: j.flatMap((f) => f.tests ?? []).length, passed: j.flatMap((f) => f.tests ?? []).filter((x) => x.ok).length, a11y: j.flatMap((f) => f.a11y ?? []).map((a) => a.code) } }
catch { result.test = { exit: t.code, raw: t.out.slice(-400) } }

const out = join(ROOT, 'bench/sites/results'); mkdirSync(out, { recursive: true })
writeFileSync(join(out, `measure-${basename(APP)}.json`), JSON.stringify(result, null, 2))
console.log(JSON.stringify(result, null, 2))
