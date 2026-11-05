// Custom asset folders must remain a portable artifact, with honest payload accounting and discovery files.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { createServer } from 'node:http'
import { createHash } from 'node:crypto'
import { spawnSync } from 'node:child_process'
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, readdirSync, statSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join, resolve, sep, extname } from 'node:path'
import { gunzipSync, brotliDecompressSync } from 'node:zlib'
import { launch, openPage } from '../../bench/lib/cdp.mjs'
const brain = process.env.BRAIN_BIN ?? new URL('../../target/debug/brain', import.meta.url).pathname
const cli = (args, cwd) => spawnSync(brain, args, { cwd, encoding: 'utf8', timeout: 40000, maxBuffer: 2 * 1024 * 1024 })
const ok = (args, cwd) => { const o = cli(args, cwd); assert.equal(o.status, 0, (o.stderr || '') + (o.stdout || '')); return JSON.parse(o.stdout) }
function inventory(root, at = root, out = new Map()) {
  for (const entry of readdirSync(at, { withFileTypes: true })) {
    const path = join(at, entry.name)
    if (entry.isDirectory()) inventory(root, path, out)
    else out.set(path.slice(root.length + 1).split(sep).join('/'), readFileSync(path))
  }
  return out
}
const snapshot = root => [...inventory(root)].map(([path, bytes]) => [path, createHash('sha256').update(bytes).digest('hex')]).sort()
function prepare(root) {
  ok(['new', 'site', '--static', '--json'], root)
  const app = join(root, 'site'), cfg = JSON.parse(readFileSync(join(app, 'brain.json'), 'utf8'))
  cfg.base = '/site/'
  cfg.staticOutput = { js: 'src/js', css: 'css', images: 'img', assets: 'src/assets', inlineCss: false }
  writeFileSync(join(app, 'brain.json'), JSON.stringify(cfg))
  mkdirSync(join(app, 'lib'), { recursive: true })
  mkdirSync(join(app, 'views/docs'), { recursive: true })
  mkdirSync(join(app, 'public/js'), { recursive: true })
  mkdirSync(join(app, 'public/vendor'), { recursive: true })
  writeFileSync(join(app, 'public/vendor/site.css'), 'body{background-image:url("../logo.svg")}')
  const shell = join(app, 'app.brain')
  writeFileSync(shell, readFileSync(shell, 'utf8').replace('</head>', '<link rel="stylesheet" href="/vendor/site.css"></head>'))
  writeFileSync(join(app, 'public/js/unhashed.js'), 'export const publicValue = 1')
  writeFileSync(join(app, 'public/logo.svg'), '<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><rect width="8" height="8" fill="blue"/></svg>')
  writeFileSync(join(app, 'public/payload.wasm'), Buffer.from([0, 97, 115, 109, 1, 0, 0, 0]))
  writeFileSync(join(app, 'lib/calculate.worker.jsbrain'), 'const twice = n => n * 2\nexport default { twice }\n')
  writeFileSync(join(app, 'views/docs/index.brain'), `<meta seo title="Custom static documentation" description="Portable static documentation with independently configured asset folders, a real worker and complete discovery files for readers and assistants.">
<script>
import calculate from '../../lib/calculate.worker.jsbrain'
const asset = new URL('../../public/payload.wasm', import.meta.url)
let count = sig(0)
let result = sig('ready')
const run = async () => { result = (await calculate.twice(21)) + ' ' + (await (await fetch(asset)).arrayBuffer()).byteLength }
</script>
<main class="page"><h1>Custom static documentation</h1><img src="/logo.svg" width="8" height="8" alt="Blue square"><button id="counter" on:click={() => count++}>Count {count}</button><button id="worker" on:click={run}>Calculate</button><p id="result">{result}</p><a id="about" href="/about">About</a></main>
<style>.page { background-image: url('/logo.svg'); padding: var(--space-4); }</style>`)
  return { app, cfg, dist: join(app, 'dist') }
}

test('custom static folders work on ordinary HTTP with workers, SRI, compression and LLM discovery', { timeout: 120000 }, async () => {
  const root = mkdtempSync(join(tmpdir(), 'brain-static-layout-'))
  let server, chrome, page
  try {
    const { app, dist } = prepare(root)
    const report = ok(['build', '--site', 'https://example.test', '--json'], app)
    const files = inventory(dist)
    const landing = 'https://chrisifm.github.io/BRAINLLM/'
    assert.equal(JSON.parse(files.get('brain-build.json')).generator.url, landing)
    assert.ok(files.get('robots.txt').toString().startsWith('# by BRAINLLM ' + landing))
    for (const [path, bytes] of files) if (path.endsWith('.xml')) {
      const xml = bytes.toString()
      assert.ok(xml.startsWith('<?xml version="1.0" encoding="UTF-8"?>\n<!-- by BRAINLLM ' + landing + ' -->'), path)
      assert.equal((xml.match(/<\?xml/g) ?? []).length, 1, path)
    }
    const outputHash = bytes => {
      let hash = 0xcbf29ce484222325n
      for (const value of bytes) hash = BigInt.asUintN(64, (hash ^ BigInt(value)) * 0x100000001b3n)
      let out = ''
      for (let i = 0; i < 8; i++, hash /= 36n) out += (hash % 36n).toString(36)
      return out
    }
    let attributed = 0
    for (const [path, bytes] of files) if (/^(src\/js|css)\/[a-z0-9]{8}\.(js|css)$/.test(path)) {
      assert.ok(bytes.toString().includes('/* by BRAINLLM ' + landing + ' */'), path)
      assert.equal(path.split('/').at(-1).split('.')[0], outputHash(bytes), 'attribution must precede the final content hash: ' + path)
      attributed++
    }
    assert.ok(attributed > 0)
    for (const file of ['index.html', 'docs/index.html', 'robots.txt', 'sitemap.xml', 'llms.txt', 'llms-full.txt']) assert.ok(files.has(file), file)
    for (const prefix of ['src/js/', 'css/', 'img/', 'src/assets/']) assert.ok([...files.keys()].some(key => key.startsWith(prefix)), prefix)
    assert.ok(![...files.keys()].some(key => key.startsWith('_b/')), 'no generated asset remains in the default namespace')
    assert.ok(files.has('logo.svg') && files.has('payload.wasm') && files.has('js/unhashed.js'), 'public originals retain stable authored URLs')
    const html = files.get('docs/index.html').toString()
    assert.match(html, /\/site\/img\//)
    assert.match(html, /\/site\/css\//)
    for (const tag of html.match(/<link\b[^>]*>/g) ?? []) {
      if (!/rel="(?:stylesheet|modulepreload)"/.test(tag)) continue
      const href = tag.match(/href="([^"]+)"/)?.[1], sum = tag.match(/integrity="([^"]+)"/)?.[1]
      assert.ok(sum && href?.startsWith('/site/'), tag)
      assert.equal(sum, 'sha384-' + createHash('sha384').update(files.get(href.slice('/site/'.length))).digest('base64'), tag)
    }
    let encodings = 0
    for (const [path, bytes] of files) if (/\.(gz|br)$/.test(path)) {
      assert.deepEqual(path.endsWith('.gz') ? gunzipSync(bytes) : brotliDecompressSync(bytes), files.get(path.slice(0, -3)), path)
      encodings++
    }
    assert.ok(encodings > 0)
    const headers = files.get('_headers').toString()
    assert.ok(!headers.includes('/site/src/js/*') && !headers.includes('/site/img/*'), 'unhashed originals must never inherit a broad immutable policy')
    assert.ok(headers.includes('/site/src/js/') && headers.includes('immutable'))
    assert.ok(files.get('llms.txt').toString().includes('Custom static documentation'))
    assert.ok(files.get('llms-full.txt').toString().includes('Custom static documentation'))
    const ledgers = report.ledger ?? report.routes
    assert.ok(Array.isArray(ledgers) && ledgers.some(route => route.assets?.some(asset => asset.kind === 'js' && asset.path.startsWith('src/js/'))), 'custom JavaScript must be charged to emitted payload budgets')
    const requests = [], missing = []
    server = createServer((req, res) => {
      const url = new URL(req.url, 'http://localhost'), path = url.pathname
      requests.push(path)
      try {
        if (!path.startsWith('/site/')) throw Error('outside base')
        let file = resolve(dist, '.' + decodeURIComponent(path.slice('/site'.length)))
        if (!file.startsWith(dist + sep) && file !== dist) throw Error('outside export')
        if (statSync(file).isDirectory()) file = join(file, 'index.html')
        res.setHeader('Content-Type', ({ '.html': 'text/html; charset=utf-8', '.js': 'text/javascript', '.css': 'text/css', '.svg': 'image/svg+xml', '.txt': 'text/plain' })[extname(file)] ?? 'application/octet-stream')
        res.end(readFileSync(file))
      } catch { missing.push(path); res.writeHead(404).end('Not found') }
    })
    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve))
    const origin = `http://127.0.0.1:${server.address().port}`
    for (const path of ['llms.txt', 'llms-full.txt', 'robots.txt', 'sitemap.xml']) assert.equal((await fetch(`${origin}/site/${path}`)).status, 200)
    chrome = await launch(); page = await openPage(chrome.port, `${origin}/site/docs/`)
    const until = async expression => {
      for (let i = 0; i < 120; i++) { try { if (await page.evaluate(expression)) return } catch {}; await new Promise(resolve => setTimeout(resolve, 50)) }
      assert.fail(expression)
    }
    await until("document.readyState === 'complete' && document.$h === 1 && document.querySelector('#counter')?.textContent === 'Count 0'")
    assert.equal(await page.evaluate("Promise.all(['sitemap.xml','rss.xml'].map(p=>fetch('/site/'+p).then(r=>r.text()).then(t=>!new DOMParser().parseFromString(t,'application/xml').querySelector('parsererror')))).then(v=>v.every(Boolean))"), true, 'attributed XML remains parseable')
    await page.evaluate("document.querySelector('#counter').click()")
    await until("document.querySelector('#counter')?.textContent === 'Count 1'")
    await page.evaluate("document.querySelector('#worker').click()")
    await until("document.querySelector('#result')?.textContent === '42 8'")
    await page.evaluate("document.querySelector('#about').click()")
    await until("location.pathname === '/site/about' && document.querySelector('h1')?.textContent === 'About site'")
    assert.deepEqual(missing.filter(path => !path.endsWith('/favicon.ico')), [])
    assert.ok(requests.includes('/site/vendor/site.css') && requests.includes('/site/logo.svg'), 'raw public CSS keeps its relative image URL on ordinary HTTP')
    assert.ok(requests.some(path => path.startsWith('/site/src/js/')) && requests.some(path => path.startsWith('/site/src/assets/')))
  } finally {
    page?.close(); chrome?.close()
    if (server) await new Promise(resolve => { server.close(resolve); server.closeAllConnections() })
    rmSync(root, { recursive: true, force: true })
  }
})

test('invalid static layouts preserve the previous artifact and ordinary mode rejects static-only layout', { timeout: 120000 }, () => {
  const root = mkdtempSync(join(tmpdir(), 'brain-static-layout-refuse-'))
  try {
    const { app, cfg, dist } = prepare(root)
    ok(['build', '--site', 'https://example.test', '--json'], app)
    const before = snapshot(dist)
    for (const layout of [{ js: '../escape' }, { js: '/absolute' }, { js: '_server/code' }, { js: 'src//js' }, { js: 42 }, { unknown: 'folder' }, { js: 'index.html' }]) {
      writeFileSync(join(app, 'brain.json'), JSON.stringify({ ...cfg, staticOutput: layout }))
      const o = cli(['build', '--site', 'https://example.test', '--json'], app)
      assert.notEqual(o.status, 0, JSON.stringify(layout))
      assert.deepEqual(snapshot(dist), before, 'failed layout never overwrites the previous export')
    }
    writeFileSync(join(app, 'brain.json'), JSON.stringify({ ...cfg, output: 'auto' }))
    const o = cli(['build', '--site', 'https://example.test', '--json'], app)
    assert.notEqual(o.status, 0)
    assert.match(o.stderr + o.stdout, /staticOutput requires static export/)
    assert.deepEqual(snapshot(dist), before)
  } finally { rmSync(root, { recursive: true, force: true }) }
})

test('static packaging omits only selected generated extras and preserves required binary assets across rebuilds', { timeout: 120000 }, () => {
  const root = mkdtempSync(join(tmpdir(), 'brain-static-packaging-'))
  try {
    const { app, cfg, dist } = prepare(root)
    writeFileSync(join(app, 'public/font.woff2'), Buffer.alloc(512, 42))
    const view = join(app, 'views/docs/index.brain')
    writeFileSync(view, readFileSync(view, 'utf8').replace("const asset = new URL", "const font = new URL('../../public/font.woff2', import.meta.url)\nconst asset = new URL").replace("let result = sig('ready')", "let result = sig(String(font).length > 0 ? 'ready' : 'missing')"))
    cfg.staticOutput.fonts = 'fonts'
    cfg.staticOutput.compression = 'fast'
    writeFileSync(join(app, 'brain.json'), JSON.stringify(cfg))
    ok(['build', '--site', 'https://example.test', '--json'], app)
    let files = inventory(dist)
    assert.ok([...files.keys()].some(path => path.startsWith('fonts/') && path.endsWith('.woff2')))
    let variants = 0
    for (const [path, bytes] of files) if (/\.(gz|br)$/.test(path)) {
      assert.deepEqual(path.endsWith('.gz') ? gunzipSync(bytes) : brotliDecompressSync(bytes), files.get(path.slice(0, -3)), path)
      variants++
    }
    assert.ok(variants > 0)
    assert.ok(files.has('_headers') && files.has('_brain/nginx.conf'))
    cfg.budget = { ...cfg.budget, viewKB: 128 } // Raw delivery is intentionally measured without compressed siblings.
    cfg.staticOutput.precompressed = false
    cfg.staticOutput.hostFiles = false
    cfg.seo = { ...cfg.seo, markdown: false, manifest: false, feed: false, sections: false }
    cfg.cache = { ...cfg.cache, sw: false }
    writeFileSync(join(app, 'brain.json'), JSON.stringify(cfg))
    ok(['build', '--site', 'https://example.test', '--json'], app)
    files = inventory(dist)
    assert.ok(![...files.keys()].some(path => /\.(br|gz|zst|md)$/.test(path) || path.startsWith('_brain/') || path.startsWith('_server/')), [...files.keys()].join('\n'))
    for (const path of ['_headers', '_redirects', 'server.mjs', 'manifest.webmanifest', 'rss.xml', 'sections.json']) assert.ok(!files.has(path), path)
    assert.match(files.get('sw.js').toString(), /registration\.unregister/, 'cache.sw:false emits only the small cleanup worker for previous visitors')
    assert.ok(!files.get('sw.js').toString().includes('BRAIN_BUILD'))
    assert.ok([...files.keys()].some(path => path.startsWith('src/assets/') && path.endsWith('.wasm')), 'required programmatic Wasm resource stays present')
    assert.ok(files.has('payload.wasm') && files.has('js/unhashed.js'), 'explicit public URLs remain available for dynamic or external references')
    for (const [path, bytes] of files) if (path.endsWith('.html')) {
      const html = bytes.toString()
      assert.ok(!/manifest.webmanifest|rss.xml|sections.json|text\/markdown/.test(html), path)
    }
    assert.ok(!files.get('llms.txt').toString().includes('.md)'))
  } finally { rmSync(root, { recursive: true, force: true }) }
})

test('static doctor audits fresh output and reports missing CSS resources with their emitted source file', { timeout: 180000 }, () => {
  const root = mkdtempSync(join(tmpdir(), 'brain-static-doctor-'))
  try {
    const { app } = prepare(root)
    const first = cli(['doctor', '--json'], app)
    const healthy = JSON.parse(first.stdout)
    assert.equal(healthy.staticArtifact?.ok, true, JSON.stringify(healthy.staticArtifact))
    assert.ok(!readdirSync(app).includes('dist'), 'doctor must not create deployment output')
    assert.ok(healthy.staticArtifact.pages >= 3)
    writeFileSync(join(app, 'public/extra.css'), '.missing{background:url("/site/not-emitted.png")}')
    const shell = join(app, 'app.brain')
    writeFileSync(shell, readFileSync(shell, 'utf8').replace('</head>', '<link rel="stylesheet" href="/extra.css"></head>'))
    const second = cli(['doctor', '--json'], app)
    const broken = JSON.parse(second.stdout)
    assert.notEqual(second.status, 0)
    assert.equal(broken.staticArtifact?.ok, false, JSON.stringify(broken))
    assert.ok(broken.failed.includes('staticArtifact'))
    assert.ok(!readdirSync(app).includes('dist'), 'failed doctor output is isolated too')
    assert.ok(broken.staticArtifact.issues.some(issue => issue.file.endsWith('.css') && issue.reference.includes('not-emitted.png')), JSON.stringify(broken.staticArtifact))
  } finally { rmSync(root, { recursive: true, force: true }) }
})

test('a wholly zero-JavaScript static site omits build-time framework modules but preserves explicit public downloads', { timeout: 120000 }, () => {
  const root = mkdtempSync(join(tmpdir(), 'brain-static-no-unused-code-'))
  try {
    ok(['new', 'site', '--static', '--json'], root)
    const app = join(root, 'site'), dist = join(app, 'dist')
    ok(['build', '--site', 'https://example.test', '--json'], app)
    let files = inventory(dist)
    assert.deepEqual([...files.keys()].filter(path => /\.(js|mjs|wasm)$/.test(path)), ['sw.js'], 'no browser code means no unused compiled router/component/runtime files')
    const authored = 'export const example = 42;'
    writeFileSync(join(app, 'public/example.js'), authored)
    const home = join(app, 'views/home.brain')
    writeFileSync(home, readFileSync(home, 'utf8').replace('</main>', '<a href="/example.js" download>Download example source</a></main>'))
    ok(['build', '--site', 'https://example.test', '--json'], app)
    files = inventory(dist)
    assert.equal(files.get('example.js').toString(), authored)
    const html = files.get('index.html').toString()
    const download = html.match(/href="([^\"]+\.js)"[^>]*download/)?.[1]
    assert.ok(download, html)
    assert.equal(files.get(download.replace(/^\//, '')).toString(), authored)
    assert.equal(download, '/example.js', 'raw public scripts retain relative-import semantics at their authored location')
    for (const [path, bytes] of files) if (/\.(js|mjs)$/.test(path) && path !== 'sw.js') assert.equal(bytes.toString(), authored, path)
  } finally { rmSync(root, { recursive: true, force: true }) }
})

test('hydrated static pages submit native forms to an explicitly allowed external origin with browser constraints', { timeout: 120000 }, async () => {
  const root = mkdtempSync(join(tmpdir(), 'brain-static-external-form-'))
  let receiver, host, chrome, page
  const posts = []
  try {
    receiver = createServer((req, res) => {
      let body = ''
      req.on('data', part => { body += part; if (body.length > 4096) req.destroy() })
      req.on('end', () => { if (req.method === 'POST') posts.push(body); res.setHeader('Content-Type', 'text/html'); res.end('<h1>Received</h1>') })
    })
    await new Promise(resolve => receiver.listen(0, '127.0.0.1', resolve))
    const external = `http://127.0.0.1:${receiver.address().port}`
    const { app, cfg, dist } = prepare(root)
    cfg.csp = { connectSrc: [], formAction: [external] }
    writeFileSync(join(app, 'brain.json'), JSON.stringify(cfg))
    const view = join(app, 'views/docs/index.brain')
    writeFileSync(view, readFileSync(view, 'utf8').replace('</main>', `<form action="${external}/submit" method="post"><label>Email <input name="email" type="email" required></label><button id="send" type="submit">Send</button></form></main>`))
    ok(['build', '--site', 'https://example.test', '--json'], app)
    const csp = readFileSync(join(dist, '_headers'), 'utf8').match(/^\s+Content-Security-Policy: (.+)$/m)?.[1]
    assert.ok(csp?.includes(`form-action 'self' ${external}`))
    host = createServer((req, res) => {
      try {
        const path = new URL(req.url, 'http://localhost').pathname
        if (!path.startsWith('/site/')) throw Error('outside base')
        let file = resolve(dist, '.' + decodeURIComponent(path.slice('/site'.length)))
        if (!file.startsWith(dist + sep) && file !== dist) throw Error('outside export')
        if (statSync(file).isDirectory()) file = join(file, 'index.html')
        res.setHeader('Content-Type', ({ '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.svg': 'image/svg+xml' })[extname(file)] ?? 'application/octet-stream')
        res.setHeader('Content-Security-Policy', csp)
        res.end(readFileSync(file))
      } catch { res.writeHead(404).end('Not found') }
    })
    await new Promise(resolve => host.listen(0, '127.0.0.1', resolve))
    const origin = `http://127.0.0.1:${host.address().port}`
    chrome = await launch(); page = await openPage(chrome.port, `${origin}/site/docs/`)
    const until = async expression => {
      for (let i = 0; i < 120; i++) { try { if (await page.evaluate(expression)) return } catch {}; await new Promise(resolve => setTimeout(resolve, 50)) }
      assert.fail(expression)
    }
    await until("document.$h === 1 && document.querySelector('#send')")
    assert.equal(await page.evaluate("document.querySelector('input[name=email]').value='invalid';document.querySelector('#send').click();document.querySelector('form').checkValidity()"), false)
    assert.equal(posts.length, 0)
    await page.evaluate("document.querySelector('input[name=email]').value='person@example.com';document.querySelector('#send').click()")
    await until("document.querySelector('h1')?.textContent === 'Received'")
    assert.deepEqual(posts, ['email=person%40example.com'])
  } finally {
    page?.close(); chrome?.close()
    for (const server of [host, receiver]) if (server) await new Promise(resolve => { server.close(resolve); server.closeAllConnections() })
    rmSync(root, { recursive: true, force: true })
  }
})
