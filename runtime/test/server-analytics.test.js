// The production server adds the behavior collector to HTML pages only when a native backend is attached, as an
// async module the backend serves; the build output (and so every route budget) is untouched.
import { test, after } from 'node:test'
import assert from 'node:assert/strict'
import { createServer as http } from 'node:http'
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { createServer } from '../server/server.mjs'

const dir = mkdtempSync(join(tmpdir(), 'brain-analytics-server-'))
mkdirSync(join(dir, '_server'))
writeFileSync(join(dir, '_server/manifest.json'), JSON.stringify({ routes: [], rpc: {}, base: '/shop/' }))
const PAGE = '<!doctype html><html><head><title>Shop</title></head><body><main data-b="k3x9">Hi</main></body></html>'
writeFileSync(join(dir, 'index.html'), PAGE)
const backend = http((req, res) => {
  res.writeHead(200, { 'content-type': 'text/javascript; charset=utf-8' })
  res.end(req.url === '/_brain/analytics.js' ? 'export const collector = 1\n' : '')
})
await new Promise(r => backend.listen(0, '127.0.0.1', r))
const backendURL = `http://127.0.0.1:${backend.address().port}`
const servers = []
const serve = async o => {
  const s = createServer({ dist: dir, log() {}, ...o })
  await new Promise(r => s.listen(0, '127.0.0.1', r))
  servers.push(s)
  return `http://127.0.0.1:${s.address().port}`
}
after(async () => {
  for (const s of servers) await new Promise(r => (s.close(r), s.closeAllConnections()))
  await new Promise(r => (backend.close(r), backend.closeAllConnections()))
  rmSync(dir, { recursive: true, force: true })
})

test('with a native backend: one async module tag before </head>, nonce applied, collector proxied under the base', async () => {
  const base = await serve({ backendURL })
  const res = await fetch(base + '/shop/')
  const html = await res.text()
  const tags = html.match(/<script[^>]*_brain\/analytics\.js[^>]*><\/script>/g)
  assert.equal(tags.length, 1)
  assert.match(tags[0], /type="module" async src="\/shop\/_brain\/analytics\.js"/)
  assert.match(tags[0], /nonce="/)
  assert.ok(html.indexOf(tags[0]) < html.indexOf('</head>'))
  const js = await fetch(base + '/shop/_brain/analytics.js')
  assert.equal(await js.text(), 'export const collector = 1\n')
  assert.equal(readFileSync(join(dir, 'index.html'), 'utf8'), PAGE) // the built file is never rewritten
})

test('no native backend, or BRAIN_ANALYTICS=off: pages are served exactly as built', async () => {
  for (const o of [{}, { backendURL, env: { BRAIN_ANALYTICS: 'off' } }, { backendURL, analytics: false }]) {
    const html = await (await fetch((await serve(o)) + '/shop/')).text()
    assert.ok(!html.includes('_brain/analytics.js'), JSON.stringify(o))
  }
})
