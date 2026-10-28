// Summarizes the build benchmark: hyperfine medians + shipped bytes (raw, gzip -9, brotli -q 11).
// Usage: node summarize.mjs <work-dir> <results-dir>
//
// Bytes are measured the same way for the three apps, from what the browser receives for the dashboard route:
// - route payload: the route's HTML document + the stylesheets it links + the JS it fetches before it is interactive
//   (<script src>, <link rel=modulepreload>, inline `import … from`, then every static import, recursively).
// - client files: every .js/.css/.html file in dist/ a browser can be served. For BRAINLLM, server-only output
//   (dist/server.mjs, dist/_server/, dist/_brain/) is excluded, and the service worker + offline page that
//   `brain build` adds by default are reported in their own column (the references have no equivalent).
// The BRAINLLM route has `load()`, so `brain build` emits no static HTML for it: its HTML is the document the
// production server (dist/server.mjs) actually returns for GET /dashboard (server-rendered, with the data). The
// references are static Vite builds: their HTML is dist/index.html and they fetch the data from the client.
import { readFileSync, writeFileSync, readdirSync, statSync, existsSync } from 'node:fs'
import { execFileSync, spawn } from 'node:child_process'
import { createServer } from 'node:net'
import { homedir } from 'node:os'
import { join, relative, extname, dirname, resolve, sep } from 'node:path'

const [work, out] = process.argv.slice(2)
if (!work || !out) { console.error('usage: node summarize.mjs <work-dir> <results-dir>'); process.exit(2) }

const apps = [
  { id: 'brain', label: 'BRAINLLM', dist: join(work, 'app/dist'), route: '/dashboard', ssr: true },
  { id: 'svelte', label: 'Svelte 5 + Vite', dist: join(work, 'svelte/dist'), route: '/', html: 'index.html' },
  { id: 'react', label: 'React 19 + Vite', dist: join(work, 'react/dist'), route: '/', html: 'index.html' },
]

const walk = (dir) => readdirSync(dir).flatMap((n) => {
  const p = join(dir, n)
  return statSync(p).isDirectory() ? walk(p) : [p]
})
const gzBuf = (b) => execFileSync('gzip', ['-9', '-n', '-c'], { input: b, maxBuffer: 1 << 28 }).length
const brBuf = (b) => execFileSync('brotli', ['-q', '11', '-c'], { input: b, maxBuffer: 1 << 28 }).length
const sizesOf = (b) => ({ raw: b.length, gz: gzBuf(b), br: brBuf(b) })
const sizes = (f) => sizesOf(readFileSync(f))
const zero = { raw: 0, gz: 0, br: 0 }
const sum = (rows) => rows.reduce((a, r) => ({ raw: a.raw + r.raw, gz: a.gz + r.gz, br: a.br + r.br }), zero)

// Server-only output of `brain build`: never sent to a browser.
const serverOnly = (rel) => rel === 'server.mjs' || rel.startsWith('_server' + sep) || rel.startsWith('_brain' + sep)
// Offline support `brain build` adds by default ("cache": { "sw", "offline" }); the references ship none.
const offlineExtra = (rel) => rel === 'sw.js' || rel === '_offline.html'

const freePort = () => new Promise((r) => { const s = createServer(); s.listen(0, '127.0.0.1', () => { const p = s.address().port; s.close(() => r(p)) }) })

// GET <route> from the app's own production server, started from dist/ and stopped by its PID.
async function ssrHtml(dist, route) {
  const port = await freePort()
  const child = spawn(process.execPath, [join(dist, 'server.mjs')], { cwd: dirname(dist), env: { ...process.env, PORT: String(port), HOST: '127.0.0.1' }, stdio: ['ignore', 'pipe', 'pipe'] })
  let log = ''
  child.stdout.on('data', (d) => (log += d))
  child.stderr.on('data', (d) => (log += d))
  try {
    for (let i = 0; i < 100 && !/brain:ready/.test(log); i++) await new Promise((r) => setTimeout(r, 50))
    if (!/brain:ready/.test(log)) throw new Error(`dist/server.mjs did not start: ${log.trim().slice(-400)}`)
    const res = await fetch(`http://127.0.0.1:${port}${route}`)
    if (res.status !== 200) throw new Error(`GET ${route} → HTTP ${res.status}`)
    return Buffer.from(await res.arrayBuffer())
  } finally {
    child.kill('SIGTERM')
  }
}

// CSS + JS the route's HTML fetches; JS follows static imports (dynamic import() is not followed).
const routeAssets = (dist, html) => {
  const toFile = (url, base) => url.startsWith('/') ? join(dist, url) : resolve(dirname(base), url)
  const css = new Set()
  for (const m of html.matchAll(/<link[^>]*rel="stylesheet"[^>]*href="([^"]+)"/g)) css.add(toFile(m[1], join(dist, 'index.html')))
  for (const m of html.matchAll(/<link[^>]*href="([^"]+)"[^>]*rel="stylesheet"/g)) css.add(toFile(m[1], join(dist, 'index.html')))
  const urls = new Set()
  for (const m of html.matchAll(/<script[^>]*\ssrc="([^"]+)"/g)) urls.add(m[1])
  for (const m of html.matchAll(/<link[^>]*rel="modulepreload"[^>]*href="([^"]+)"/g)) urls.add(m[1])
  for (const m of html.matchAll(/(?:from|import)\s*["']([^"']+\.js)["']/g)) urls.add(m[1])
  const seen = new Set()
  const queue = [...urls].map((u) => toFile(u, join(dist, 'index.html')))
  while (queue.length) {
    const f = queue.pop()
    if (seen.has(f) || !existsSync(f)) continue
    seen.add(f)
    const src = readFileSync(f, 'utf8')
    for (const m of src.matchAll(/(?:^|[;\s}])(?:import|export)\s*(?:[^'"();]*?\sfrom\s*)?["']([^"']+\.js)["']/g)) queue.push(toFile(m[1], f))
  }
  return { css: [...css].filter(existsSync), js: [...seen] }
}

const bytes = {}
for (const a of apps) {
  const files = walk(a.dist)
    .filter((f) => ['.js', '.css', '.html'].includes(extname(f)))
    .filter((f) => !serverOnly(relative(a.dist, f)))
    .sort()
  const rows = files.map((f) => ({ file: relative(a.dist, f), ...sizes(f) }))
  const main = rows.filter((r) => !offlineExtra(r.file))
  const byExt = (e) => sum(main.filter((r) => extname(r.file) === e))
  const htmlBuf = a.ssr ? await ssrHtml(a.dist, a.route) : readFileSync(join(a.dist, a.html))
  const assets = routeAssets(a.dist, htmlBuf.toString('utf8'))
  const rel = (fs) => fs.map((f) => relative(a.dist, f)).sort()
  const pick = (fs) => sum(fs.map((f) => sizes(f)))
  const route = {
    path: a.route,
    htmlSource: a.ssr ? `GET ${a.route} from dist/server.mjs` : `dist/${a.html}`,
    html: sizesOf(htmlBuf),
    css: pick(assets.css),
    js: pick(assets.js),
    cssFiles: rel(assets.css),
    jsFiles: rel(assets.js),
  }
  route.total = sum([route.html, route.css, route.js])
  bytes[a.id] = {
    label: a.label,
    files: rows,
    total: sum(main),
    js: byExt('.js'),
    css: byExt('.css'),
    html: byExt('.html'),
    offlineExtra: sum(rows.filter((r) => offlineExtra(r.file))),
    route,
    // Kept for continuity with earlier summaries: the route's JS only.
    firstRoute: { files: route.jsFiles, js: route.js },
  }
}

const timing = {}
for (const f of readdirSync(out).filter((n) => n.endsWith('.json') && n.startsWith('hf-'))) {
  const r = JSON.parse(readFileSync(join(out, f), 'utf8')).results[0]
  const ms = (s) => +(s * 1000).toFixed(1)
  timing[f.slice(3, -5)] = { command: r.command.replaceAll(homedir(), '~'), runs: r.times.length, median: ms(r.median), mean: ms(r.mean), stddev: ms(r.stddev ?? 0), min: ms(r.min), max: ms(r.max) }
}

const versions = JSON.parse(readFileSync(join(out, 'versions.json'), 'utf8'))
const summary = { date: new Date().toISOString(), versions, timing, bytes }
writeFileSync(join(out, 'summary.json'), JSON.stringify(summary, null, 2) + '\n')

// Markdown tables.
const kb = (n) => (n / 1024).toFixed(2)
const t = (k) => timing[k] ? `${timing[k].median} ± ${timing[k].stddev}` : 'n/a'
const ratio = (k, base) => timing[k] && timing[base] ? `${(timing[k].median / timing[base].median).toFixed(1)}×` : 'n/a'
let md = ''
md += `## Build time (ms, median ± stddev, ${Object.values(timing)[0]?.runs ?? '?'} runs)\n\n`
md += '| Tool | Cold | Warm | Warm min | Warm median vs BRAINLLM |\n|---|---:|---:|---:|---:|\n'
for (const a of apps) md += `| ${a.label} | ${t(`${a.id}-cold`)} | ${t(`${a.id}-warm`)} | ${timing[`${a.id}-warm`]?.min ?? 'n/a'} | ${ratio(`${a.id}-warm`, 'brain-warm')} |\n`
md += '\n## Tool cold start (ms, median ± stddev)\n\n| Command | Time |\n|---|---:|\n'
for (const k of ['brain-start', 'vite-start']) if (timing[k]) md += `| \`${timing[k].command.replace(join(work, 'bin') + '/', '')}\` | ${t(k)} |\n`
md += '\n## Route payload — what the browser receives for the dashboard route (KiB; each file compressed on its own)\n\n'
md += '| App | Route | HTML gz | CSS gz | JS gz | Total raw | Total gzip -9 | Total brotli -q 11 | JS files |\n|---|---|---:|---:|---:|---:|---:|---:|---:|\n'
for (const a of apps) {
  const r = bytes[a.id].route
  md += `| ${a.label} | \`${r.path}\` (${r.htmlSource}) | ${kb(r.html.gz)} | ${kb(r.css.gz)} | ${kb(r.js.gz)} | ${kb(r.total.raw)} | ${kb(r.total.gz)} | ${kb(r.total.br)} | ${r.jsFiles.length} |\n`
}
md += '\n## Client files in dist/ (KiB; JS+CSS+HTML a browser can be served, each file compressed on its own)\n\n'
md += '| App | Files | Raw | gzip -9 | brotli -q 11 | + service worker and offline page (gz) |\n|---|---:|---:|---:|---:|---:|\n'
for (const a of apps) {
  const b = bytes[a.id]
  const n = b.files.filter((r) => !offlineExtra(r.file)).length
  md += `| ${a.label} | ${n} | ${kb(b.total.raw)} | ${kb(b.total.gz)} | ${kb(b.total.br)} | ${b.offlineExtra.raw ? kb(b.offlineExtra.gz) : '—'} |\n`
}
md += '\n## Per file (bytes)\n\n| App | File | Raw | gzip -9 | brotli -q 11 |\n|---|---|---:|---:|---:|\n'
for (const a of apps) for (const r of bytes[a.id].files) md += `| ${a.id} | \`${r.file}\` | ${r.raw} | ${r.gz} | ${r.br} |\n`
md += '\n## Versions\n\n| Item | Value |\n|---|---|\n'
for (const [k, v] of Object.entries(versions)) md += `| ${k} | ${v} |\n`
writeFileSync(join(out, 'summary.md'), md)
process.stdout.write(md)
