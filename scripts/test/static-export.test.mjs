// Prove the artifact works on an ordinary directory-index HTTP server, without any BRAINLLM gateway.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { createServer } from 'node:http'
import { spawnSync } from 'node:child_process'
import { mkdtempSync, writeFileSync, readFileSync, readdirSync, statSync, rmSync, mkdirSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join, extname, resolve, sep } from 'node:path'
import { gunzipSync, brotliDecompressSync } from 'node:zlib'
import { launch, openPage } from '../../bench/lib/cdp.mjs'
const brain = process.env.BRAIN_BIN ?? new URL('../../target/debug/brain', import.meta.url).pathname

test('static export works with generic HTTP hosting, direct nested URLs, browser state and compression', { timeout: 120000 }, async () => {
  const root = mkdtempSync(join(tmpdir(), 'brain-static-http-'))
  let chrome, page, server
  try {
    const run = (args, cwd) => {
      const o = spawnSync(brain, args, { cwd, encoding: 'utf8', timeout: 40000, maxBuffer: 1024 * 1024 })
      assert.equal(o.status, 0, o.stderr || o.stdout || o.error?.message)
      return JSON.parse(o.stdout)
    }
    run(['new', 'site', '--static', '--json'], root)
    const app = join(root, 'site'), dist = join(app, 'dist')
    mkdirSync(join(app, 'views/docs'))
    writeFileSync(join(app, 'views/docs/index.brain'), `<meta seo title="Static documentation" description="Static documentation with a browser-only counter, complete HTML and portable assets that work on ordinary file hosting." canonical="/docs">
<script>let count = sig(0)</script>
<main><h1>Static documentation</h1><button id="counter" on:click={() => count++}>Count {count}</button><a id="about" href="/about">About</a></main>`)
    mkdirSync(join(app, 'views/items'))
    writeFileSync(join(app, 'views/items/[slug].brain'), `<meta seo title={props.title} description="A statically generated page whose international and percent-encoded URL resolves through ordinary HTTP file hosting.">
<script>
export async function paths() { return ['a b', 'caf\\u00e9', 'percent%2F'].map(slug => ({slug})) }
export async function load(ctx) { return {title: ctx.params.slug} }
</script>
<main><h1>{props.title}</h1></main>`)
    const result = run(['build', '--static', '--site', 'https://example.com', '--json'], app)
    assert.equal(result.outputMode, 'static')
    let gzip = 0, brotli = 0
    const walk = dir => { for (const e of readdirSync(dir, { withFileTypes: true })) {
      const p = join(dir, e.name)
      if (e.isDirectory()) { assert.notEqual(e.name, '_server'); walk(p); continue }
      assert.notEqual(e.name, 'server.mjs')
      if (p.endsWith('.gz') || p.endsWith('.br')) {
        const bytes = readFileSync(p), plain = readFileSync(p.slice(0, -3))
        const decoded = p.endsWith('.gz') ? (gzip++, gunzipSync(bytes)) : (brotli++, brotliDecompressSync(bytes))
        assert.deepEqual(decoded, plain, p)
        assert.ok(bytes.length < plain.length, 'precompressed output is useful')
      }
    } }
    walk(dist); assert.ok(gzip > 0 && brotli > 0)
    const requests = []
    // Deliberately no framework rewrites, SSR, precompression or generated host-configuration support.
    server = createServer((req, res) => {
      try {
        const path = new URL(req.url, 'http://localhost').pathname
        requests.push(path)
        let file = resolve(dist, '.' + decodeURIComponent(path))
        if (!file.startsWith(dist + sep) && file !== dist) { res.writeHead(403).end(); return }
        if (statSync(file).isDirectory()) file = join(file, 'index.html')
        res.setHeader('Content-Type', ({ '.html': 'text/html; charset=utf-8', '.js': 'text/javascript', '.css': 'text/css', '.svg': 'image/svg+xml', '.png': 'image/png' })[extname(file)] ?? 'application/octet-stream')
        res.end(readFileSync(file))
      } catch { res.writeHead(404).end('Not found') }
    })
    await new Promise(r => server.listen(0, '127.0.0.1', r))
    const origin = `http://127.0.0.1:${server.address().port}`
    for (const route of ['/', '/docs/', '/about/', '/404.html']) {
      const response = await fetch(origin + route)
      assert.equal(response.status, 200, route)
      assert.match(await response.text(), /<h1>/)
    }
    for (const [url, title] of [['/items/a%20b/', 'a b'], ['/items/caf%C3%A9/', 'caf\u00e9'], ['/items/percent%252F/', 'percent%2F']]) {
      const response = await fetch(origin + url)
      assert.equal(response.status, 200, url)
      const html = await response.text()
      const heading = html.match(/<h1\b[^>]*>([\s\S]*?)<\/h1>/)?.[1]?.replace(/<!--[\s\S]*?-->/g, '')
      assert.equal(heading, title, url)
      const markdown = await fetch(origin + url.slice(0, -1) + '.md')
      assert.equal(markdown.status, 200, url + ' Markdown')
      assert.ok((await markdown.text()).includes(title), url)
    }
    chrome = await launch(); page = await openPage(chrome.port, origin + '/docs/')
    const until = async expression => {
      for (let i = 0; i < 100; i++) {
        try { if (await page.evaluate(expression)) return } catch {}
        await new Promise(r => setTimeout(r, 50))
      }
      assert.fail(expression)
    }
    await until("document.readyState === 'complete' && document.$h === 1 && document.querySelector('#counter')?.textContent === 'Count 0'")
    await page.evaluate("document.querySelector('#counter').click()")
    await until("document.querySelector('#counter')?.textContent === 'Count 1'")
    await page.evaluate("document.querySelector('#about').click()")
    await until("location.pathname === '/about' && document.querySelector('h1')?.textContent === 'About site'")
    await page.send('Page.navigate', { url: origin + '/docs/' })
    await until("document.readyState === 'complete' && document.$h === 1 && document.querySelector('#counter')?.textContent === 'Count 0'")
    await page.evaluate("document.querySelector('#counter').click()")
    await until("document.querySelector('#counter')?.textContent === 'Count 1'")
    assert.ok(!requests.some(p => p.startsWith('/_brain/') || p.startsWith('/_server/')), requests.join('\n'))
  } finally {
    page?.close(); chrome?.close()
    if (server) await new Promise(r => { server.close(r); server.closeAllConnections() })
    rmSync(root, { recursive: true, force: true })
  }
})


test('MCP creates and builds a static site using the persistent output policy', { timeout: 60000 }, () => {
  const root = mkdtempSync(join(tmpdir(), 'brain-static-mcp-'))
  try {
    const rpc = (cwd, name, args) => {
      const input = [
        { jsonrpc: '2.0', id: 0, method: 'initialize', params: { protocolVersion: '2025-11-25', capabilities: {}, clientInfo: { name: 'static-export-test', version: '1' } } },
        { jsonrpc: '2.0', method: 'notifications/initialized' },
        { jsonrpc: '2.0', id: 1, method: 'tools/call', params: { name, arguments: args } },
      ].map(JSON.stringify).join('\n') + '\n'
      const o = spawnSync(brain, ['mcp'], { cwd, input, encoding: 'utf8', timeout: 40000, maxBuffer: 1024 * 1024 })
      assert.equal(o.status, 0, o.stderr)
      const result = o.stdout.trim().split('\n').map(JSON.parse).find(r => r.id === 1)
      assert.ok(result && !result.error && !result.result?.isError, JSON.stringify(result))
      return result.result.structuredContent ?? JSON.parse(result.result.content.find(c => c.type === 'text').text)
    }
    const app = join(root, 'site')
    rpc(root, 'brain_new', { name: 'site', dir: app, static: true })
    assert.equal(JSON.parse(readFileSync(join(app, 'brain.json'), 'utf8')).output, 'static')
    const result = rpc(app, 'brain_build', { site: 'https://example.com' })
    assert.equal(result.outputMode, 'static')
    assert.ok(readFileSync(join(app, 'dist/index.html'), 'utf8').includes('Your static website'))
    const config = JSON.parse(readFileSync(join(app, 'brain.json'), 'utf8'))
    config.output = 'auto'
    writeFileSync(join(app, 'brain.json'), JSON.stringify(config))
    const forced = rpc(app, 'brain_build', { static: true, site: 'https://example.com' })
    assert.equal(forced.outputMode, 'static')
    assert.equal(JSON.parse(readFileSync(join(app, 'brain.json'), 'utf8')).output, 'auto')
  } finally { rmSync(root, { recursive: true, force: true }) }
})
