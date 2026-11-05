// A real multilingual build keeps one local breadcrumb root and the same image identity in HTML and JSON-LD.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, rmSync, existsSync } from 'node:fs'
import { join } from 'node:path'
import { tmpdir } from 'node:os'
const brain = process.env.BRAIN_BIN ?? new URL('../../target/debug/brain', import.meta.url).pathname
const nodes = html => [...html.matchAll(/<script[^>]*type="application\/ld\+json">([\s\S]*?)<\/script>/g)].flatMap(m => { const n = JSON.parse(m[1]); return n['@graph'] ?? [n] })
test('static and ordinary builds localize breadcrumb roots and resolve image metadata with the final canonical policy', { timeout: 180000 }, () => {
  const root = mkdtempSync(join(tmpdir(), 'brain-localized-seo-'))
  const cli = (args, cwd) => { const r = spawnSync(brain, args, { cwd, encoding: 'utf8', timeout: 60000, maxBuffer: 4 * 1024 * 1024 }); assert.equal(r.status, 0, r.stdout + r.stderr) }
  try {
    for (const mode of ['static', 'ordinary']) {
      cli(['new', mode, ...(mode === 'static' ? ['--static'] : []), '--json'], root)
      const app = join(root, mode), configPath = join(app, 'brain.json'), config = JSON.parse(readFileSync(configPath, 'utf8'))
      config.base = '/app/'; config.i18n = { default: 'en', locales: ['en', 'es-MX'] }; config.seo = { ...config.seo, trailingSlash: 'always' }
      writeFileSync(configPath, JSON.stringify(config)); mkdirSync(join(app, 'i18n'), { recursive: true })
      const dict = existsSync(join(app, 'i18n/en.json')) ? readFileSync(join(app, 'i18n/en.json'), 'utf8') : '{}'
      writeFileSync(join(app, 'i18n/en.json'), dict); writeFileSync(join(app, 'i18n/es-MX.json'), dict)
      writeFileSync(join(app, 'views/about.brain'), '<meta seo title="Example metadata page" description="" keyword="example" schema="Article" lastmod="" og:image="cover.svg"><main><h1>Example metadata page</h1><p>Read the example project information.</p></main>')
      mkdirSync(join(app, 'views/facts'), { recursive: true })
      writeFileSync(join(app, 'views/facts/[slug].brain'), `<meta seo title="Example optional facts" description={props.optional} keyword="example" schema="Article" lastmod={props.optional} og:image={props.optional}>
<script>export function paths(){return [{slug:'blank'}]}export async function load(){return {optional:'   '}}</script><main><h1>Example optional facts</h1><p>Read the example project information.</p></main>`)
      for (const locale of ['', 'es-MX/']) {
        mkdirSync(join(app, 'public', locale, 'about'), { recursive: true })
        writeFileSync(join(app, 'public', locale, 'about/cover.svg'), '<svg xmlns="http://www.w3.org/2000/svg" width="1200" height="630"><rect width="1200" height="630" fill="navy"/></svg>')
      }
      cli(['build', '--site', 'https://example.test', '--json'], app)
      for (const locale of ['', 'es-MX/']) {
        const html = readFileSync(join(app, 'dist', locale, 'about/index.html'), 'utf8'), graph = nodes(html)
        const image = `https://example.test/app/${locale}about/cover.svg`
        assert.ok(html.includes(`property="og:image" content="${image}"`), html)
        const article = graph.find(n => n['@type'] === 'Article'); assert.ok(article, html)
        assert.equal(article.image, image)
        assert.equal(Object.hasOwn(article, 'dateModified'), false)
        assert.equal(Object.hasOwn(article, 'description'), false)
        const crumb = graph.find(n => n['@type'] === 'BreadcrumbList'); assert.ok(crumb, html)
        assert.deepEqual(crumb.itemListElement.map(n => n.item), [`https://example.test/app/${locale}`, `https://example.test/app/${locale}about/`])
        assert.ok(graph.some(n => n['@id'] === 'https://example.test/app/#website'))
        if (locale) assert.ok(html.includes('property="og:locale" content="es_MX"'))
      }
      for (const locale of ['', 'es-MX/']) {
        const html = readFileSync(join(app, 'dist', locale, 'facts/blank/index.html'), 'utf8')
        assert.doesNotMatch(html, /<meta[^>]*content="\s*"/)
        const article = nodes(html).find(n => n['@type'] === 'Article'); assert.ok(article, html)
        assert.equal(Object.hasOwn(article, 'dateModified'), false, html)
        if (article.description != null) assert.ok(article.description.trim(), html)
        if (article.image != null) assert.notEqual(article.image, `https://example.test/app/${locale}facts/blank/`, html)
      }
      assert.doesNotMatch(readFileSync(join(app, 'dist/sitemap.xml'), 'utf8'), /<lastmod>\s*<\/lastmod>/)
      const home = nodes(readFileSync(join(app, 'dist/es-MX/index.html'), 'utf8'))
      assert.ok(!home.some(n => n['@type'] === 'BreadcrumbList'), 'localized home must not add a redundant default home crumb')
    }
  } finally { rmSync(root, { recursive: true, force: true }) }
})

test('compiled literal SEO keeps the streamed head available while its load is held', { timeout: 180000 }, async () => {
  const { createServer: httpServer, get } = await import('node:http')
  const { pathToFileURL } = await import('node:url')
  const root = mkdtempSync(join(tmpdir(), 'brain-static-seo-stream-'))
  let appServer, barrier, held, entered, released = false, timer
  const enteredPromise = new Promise(resolve => { entered = resolve })
  const release = () => { released = true; if (held) held.end('ready') }
  try {
    barrier = httpServer((req, res) => { held = res; entered(); if (released) res.end('ready') })
    await new Promise(resolve => barrier.listen(0, '127.0.0.1', resolve))
    const cli = (args, cwd) => { const r = spawnSync(brain, args, { cwd, encoding: 'utf8', timeout: 60000, maxBuffer: 4 * 1024 * 1024 }); assert.equal(r.status, 0, r.stdout + r.stderr) }
    cli(['new', 'app', '--json'], root)
    const app = join(root, 'app'); mkdirSync(join(app, 'views/stream'), { recursive: true })
    writeFileSync(join(app, 'views/stream/[id].brain'), `<meta seo title="Streamed metadata example" description="Read the example metadata while its independent page data loads, preserving the early document head and the final rendered information." keyword="example">
<script>export async function load(){await fetch('http://127.0.0.1:${barrier.address().port}/hold');return {value:'Ready'}}</script><main><h1>Streamed metadata example</h1><p>{props.value}</p></main>`)
    cli(['build', '--site', 'https://example.test', '--json'], app)
    const manifest = JSON.parse(readFileSync(join(app, 'dist/_server/manifest.json'), 'utf8'))
    assert.equal(manifest.routes.find(r => r.path === '/stream/:id')?.stream, true)
    const { createServer } = await import(pathToFileURL(join(app, 'dist/server.mjs')).href)
    appServer = createServer({ dist: join(app, 'dist'), log() {}, streamHoldMs: 0 })
    await new Promise(resolve => appServer.listen(0, '127.0.0.1', resolve))
    let header, rejectHead
    const head = new Promise((resolve, reject) => { header = resolve; rejectHead = reject })
    const complete = new Promise((resolve, reject) => {
      const req = get(`http://127.0.0.1:${appServer.address().port}/stream/example`, res => {
        let body = ''
        res.on('data', chunk => { body += chunk; if (body.length > 256 * 1024) res.destroy(Error('Unbounded fixture output')); if (body.includes('</head>')) header(body) })
        res.on('end', () => resolve(body)); res.on('error', reject)
      })
      req.on('error', error => { rejectHead(error); reject(error) })
    })
    complete.catch(() => {})
    const deadline = new Promise((_, reject) => { timer = setTimeout(() => reject(Error('Compiled static SEO waited for held load before sending its head')), 10000) })
    const [, initial] = await Promise.race([Promise.all([enteredPromise, head]), deadline])
    assert.equal(released, false)
    assert.ok(initial.includes('<title>Streamed metadata example'), initial)
    release()
    const final = await complete
    assert.ok(final.includes('Ready'), final)
  } finally {
    clearTimeout(timer); release()
    if (appServer) await new Promise(resolve => { appServer.close(resolve); appServer.closeAllConnections() })
    if (barrier) await new Promise(resolve => { barrier.close(resolve); barrier.closeAllConnections() })
    rmSync(root, { recursive: true, force: true })
  }
})
