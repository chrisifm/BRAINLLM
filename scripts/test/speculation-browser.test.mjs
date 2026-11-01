// Speculation Rules in real Chrome under the build's real CSP: hover fetches the target once, ahead of the click.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { createServer } from 'node:http'
import { mkdtempSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { launch, openPage } from '../../bench/lib/cdp.mjs'

const brain = process.env.BRAIN_BIN ?? new URL('../../target/debug/brain', import.meta.url).pathname
const wait = ms => new Promise(r => setTimeout(r, ms))

test('hovering a link prefetches it under the page CSP and the click uses that fetch', { timeout: 180000 }, async () => {
  const root = mkdtempSync(join(tmpdir(), 'brain-speculation-'))
  const documents = []
  let chrome, page, server
  try {
    assert.equal(spawnSync(brain, ['new', 'app'], { cwd: root, stdio: 'ignore' }).status, 0)
    const dist = join(root, 'app/dist')
    writeFileSync(join(root, 'app/views/_layout.brain'), '<nav aria-label="Main"><a href="/">Home</a> <a href="/about">About</a></nav>\n<slot />\n')
    assert.equal(spawnSync(brain, ['build'], { cwd: join(root, 'app'), stdio: 'ignore' }).status, 0)
    const headers = readFileSync(join(dist, '_headers'), 'utf8')
    const csp = headers.match(/^\s*Content-Security-Policy:\s*(.+)$/m)?.[1]
    assert.ok(csp, 'control: the build emits its policy')
    server = createServer((req, res) => {
      const path = req.url.split('?')[0]
      const clean = path === '/' ? 'index' : path.slice(1)
      const file = [join(dist, path), join(dist, clean + '.html'), join(dist, clean, 'index.html')].find(f => statSync(f, { throwIfNoEntry: false })?.isFile())
      if (req.headers['sec-fetch-dest'] === 'document' || req.headers['sec-purpose']) documents.push({ path, purpose: req.headers['sec-purpose'] ?? null })
      if (!file) { res.statusCode = 404; return res.end() }
      res.setHeader('Cache-Control', 'public, max-age=60')
      if (file.endsWith('.html')) { res.setHeader('Content-Type', 'text/html'); res.setHeader('Content-Security-Policy', csp) }
      else if (file.endsWith('.js')) res.setHeader('Content-Type', 'text/javascript')
      else if (file.endsWith('.css')) res.setHeader('Content-Type', 'text/css')
      res.end(readFileSync(file))
    })
    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve))
    const origin = `http://127.0.0.1:${server.address().port}`
    chrome = await launch()
    page = await openPage(chrome.port, `${origin}/about`)
    await page.evaluate(`new Promise(r => document.readyState === 'complete' ? r() : addEventListener('load', r, { once: true }))`)
    assert.equal(await page.evaluate(`HTMLScriptElement.supports('speculationrules')`), true, 'control: this Chrome implements the feature')
    assert.ok(await page.evaluate(`!!document.querySelector('script[type=speculationrules]')`))
    await page.evaluate(`(() => { globalThis.violations = []; addEventListener('securitypolicyviolation', e => violations.push(e.violatedDirective)) })()`)
    const box = await page.evaluate(`(() => { let a = document.querySelector('a[href="/"]'); if (!a) { a = Object.assign(document.createElement('a'), { href: '/', textContent: 'Home' }); a.style.cssText = 'position:fixed;left:10px;top:10px;padding:20px;z-index:9999;background:#fff'; document.body.append(a) } a.scrollIntoView(); const r = a.getBoundingClientRect(); return { x: r.x + r.width / 2, y: r.y + r.height / 2 } })()`)
    const before = documents.length
    await page.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: box.x, y: box.y })
    let prefetched
    for (let i = 0; i < 40 && !prefetched; i++) { await wait(250); prefetched = documents.slice(before).find(d => d.path === '/' && d.purpose?.includes('prefetch')) }
    assert.ok(prefetched, `hover must prefetch the target: ${JSON.stringify(documents)}`)
    assert.deepEqual(await page.evaluate('violations'), [])
    const seen = documents.length
    await page.send('Input.dispatchMouseEvent', { type: 'mousePressed', x: box.x, y: box.y, button: 'left', clickCount: 1 })
    await page.send('Input.dispatchMouseEvent', { type: 'mouseReleased', x: box.x, y: box.y, button: 'left', clickCount: 1 })
    await wait(1500)
    assert.equal(new URL(await page.evaluate('location.href')).pathname, '/')
    assert.equal(documents.slice(seen).filter(d => d.path === '/').length, 0, `the click reused the prefetch: ${JSON.stringify(documents.slice(seen))}`)
  } finally {
    page?.close(); chrome?.close()
    if (server) await new Promise(resolve => { server.close(resolve); server.closeAllConnections() })
    rmSync(root, { recursive: true, force: true })
  }
})
