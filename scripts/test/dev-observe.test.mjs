// The dev observability panel in real Chrome, and its absence from a production build.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { spawn, spawnSync } from 'node:child_process'
import { mkdtempSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs'
import { createServer } from 'node:http'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { launch, openPage } from '../../bench/lib/cdp.mjs'

const brain = process.env.BRAIN_BIN ?? new URL('../../target/debug/brain', import.meta.url).pathname
const walk = dir => readdirSync(dir).flatMap(name => { const p = join(dir, name); return statSync(p).isDirectory() ? walk(p) : [p] })
const until = async (read, label, ms = 30000) => {
  const end = Date.now() + ms
  for (;;) {
    const value = await read().catch(() => null)
    if (value) return value
    if (Date.now() > end) throw Error(`${label}: deadline`)
    await new Promise(r => setTimeout(r, 200))
  }
}

test('brain run shows the panel on request and a build holds none of it', { timeout: 240000 }, async () => {
  const root = mkdtempSync(join(tmpdir(), 'brain-observe-'))
  let run, chrome, page
  try {
    assert.equal(spawnSync(brain, ['new', 'app'], { cwd: root, stdio: 'ignore' }).status, 0)
    const app = join(root, 'app')
    const port = 41000 + (process.pid % 2000)
    run = spawn(brain, ['run', '--port', String(port)], { cwd: app, stdio: 'ignore' })
    const url = `http://127.0.0.1:${port}/`
    await until(async () => (await fetch(url)).ok, 'dev server', 90000)
    chrome = await launch()
    page = await openPage(chrome.port, url)
    await until(() => page.evaluate("typeof __brainObserve === 'object'"), 'observe module')
    assert.equal(await page.evaluate("document.querySelector('brain-observe')"), null, 'closed until asked for')
    await page.evaluate("fetch('/').then(() => dispatchEvent(new CustomEvent('brain:net', { detail: { method: 'GET', url: '/probe', status: 500, ms: 12 } })))")
    const snap = await until(async () => {
      const s = await page.evaluate('__brainObserve.snapshot()')
      return s.requests.length && s.vitals.FCP && s.components.length ? s : null
    }, 'measured data')
    assert.equal(snap.requests[0].url, '/probe')
    assert.ok(snap.vitals.TTFB.value >= 0)
    assert.ok(snap.components.some(c => c.file?.endsWith('.brain')), JSON.stringify(snap.components))
    assert.equal(await page.evaluate(`(() => { dispatchEvent(new KeyboardEvent('keydown', { altKey: true, shiftKey: true, code: 'KeyO' })); return !!document.querySelector('brain-observe') })()`), true)
    const text = await page.evaluate("document.querySelector('brain-observe').shadowRoot.textContent")
    assert.match(text, /Web Vitals/)
    assert.match(text, /\/probe/)
    const picked = await page.evaluate(`new Promise(resolve => {
      addEventListener('brain:observe-pick', ev => resolve(ev.detail), { once: true })
      const button = document.querySelector('brain-observe').shadowRoot.querySelector('button')
      button.click()
      document.querySelector('#brain-view h1, #brain-view *').click()
    })`)
    assert.ok(picked?.file?.endsWith('.brain'), JSON.stringify(picked))
    page.close(); page = null
    chrome.close(); chrome = null
    run.kill(); await new Promise(r => run.once('exit', r)); run = null

    const built = spawnSync(brain, ['build'], { cwd: app, encoding: 'utf8' })
    assert.equal(built.status, 0, built.stdout + built.stderr)
    const files = walk(join(app, 'dist'))
    assert.ok(files.length > 5)
    for (const file of files) {
      const body = readFileSync(file, 'latin1')
      for (const marker of ['brain-observe', '__brainObserve', 'brain:observe', 'brain-vitals', 'largest-contentful-paint']) assert.ok(!body.includes(marker), `${marker} in ${file}`)
    }
  } finally {
    page?.close(); chrome?.close(); run?.kill()
    rmSync(root, { recursive: true, force: true })
  }
})

test('reportVitals sends real Chrome metrics by beacon when the page is hidden, without the query string', { timeout: 60000 }, async () => {
  const beacons = []
  const script = readFileSync(new URL('../../runtime/brain-vitals.js', import.meta.url))
  const server = createServer((req, res) => {
    if (req.url === '/vitals') { let body = ''; req.on('data', c => (body += c)); req.on('end', () => { beacons.push(JSON.parse(body)); res.end() }); return }
    res.setHeader('Content-Type', req.url === '/brain-vitals.js' ? 'text/javascript' : 'text/html')
    res.end(req.url === '/brain-vitals.js' ? script : '<!doctype html><title>Vitals</title><h1>Largest text on this page</h1>')
  })
  let chrome, page
  try {
    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve))
    chrome = await launch()
    page = await openPage(chrome.port, `http://127.0.0.1:${server.address().port}/?secret=1`)
    await page.evaluate(`import('/brain-vitals.js').then(m => {
      m.reportVitals('/vitals')
      return new Promise(r => setTimeout(r, 500))
    }).then(() => {
      Object.defineProperty(document, 'visibilityState', { value: 'hidden', configurable: true })
      document.dispatchEvent(new Event('visibilitychange'))
    })`)
    const sent = await until(async () => beacons[0], 'beacon', 10000)
    assert.equal(sent.path, '/')
    const names = sent.metrics.map(m => m.name)
    for (const name of ['TTFB', 'FCP', 'LCP']) assert.ok(names.includes(name), names.join())
    for (const m of sent.metrics) assert.match(m.rating, /^(good|needs-improvement|poor)$/)
    assert.equal(JSON.stringify(sent).includes('secret'), false)
    await assert.rejects(page.evaluate(`import('/brain-vitals.js').then(m => m.reportVitals('https://elsewhere.example/'))`))
  } finally {
    page?.close(); chrome?.close()
    await new Promise(resolve => server.close(resolve))
  }
})
