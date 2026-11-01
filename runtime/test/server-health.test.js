// `/health` is reserved for the backend (B0735), so the frontend server has to answer it on its own port.
import { test, after } from 'node:test'
import assert from 'node:assert/strict'
import http from 'node:http'
import { mkdirSync, mkdtempSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { createServer } from '../server/server.mjs'

const made = []
const dist = (manifest = {}, files = {}) => {
  const root = mkdtempSync(join(tmpdir(), 'brain-health-'))
  mkdirSync(join(root, '_server'))
  writeFileSync(join(root, '_server/manifest.json'), JSON.stringify({ routes: [], rpc: {}, ...manifest }))
  for (const [name, body] of Object.entries(files)) {
    mkdirSync(join(root, name, '..'), { recursive: true })
    writeFileSync(join(root, name), body)
  }
  return root
}
const listen = async server => {
  await new Promise(ok => server.listen(0, '127.0.0.1', ok))
  made.push(server)
  return server.address().port
}
const frontend = async (opts, manifest, files) => `http://127.0.0.1:${await listen(createServer({ dist: dist(manifest, files), log: () => {}, ...opts }))}`
after(() => made.forEach(s => (s.closeAllConnections?.(), s.close())))

test('without a backend it answers a plain healthy response, with no details', async () => {
  const base = await frontend({}, { build: 'b1' })
  for (const path of ['/health', '/health/live', '/health/ready']) {
    const response = await fetch(base + path)
    assert.equal(response.status, 200, path)
    assert.deepEqual(await response.json(), { ok: true })
    assert.equal(response.headers.get('cache-control'), 'no-store')
  }
  assert.equal((await fetch(base + '/health', { method: 'HEAD' })).status, 200)
  assert.equal((await fetch(base + '/health/other')).status, 404)
  assert.equal((await fetch(base + '/health', { method: 'POST' })).status, 405)
})

test('with a native backend it forwards the probe and keeps the status', async () => {
  const seen = []
  let status = 200
  const backend = http.createServer((req, res) => {
    seen.push(req.url)
    res.writeHead(status, { 'content-type': 'application/json', 'x-private-upstream': 'never' })
    res.end(JSON.stringify(status === 200 ? { ok: true, schemaVersion: 1 } : { code: 'DATABASE_UNAVAILABLE' }))
  })
  const port = await listen(backend)
  const base = await frontend({ backendURL: `http://127.0.0.1:${port}` })
  const good = await fetch(base + '/health/ready')
  assert.equal(good.status, 200)
  assert.deepEqual(await good.json(), { ok: true, schemaVersion: 1 })
  assert.equal(good.headers.get('x-private-upstream'), null)
  assert.deepEqual(seen, ['/health/ready'])
  status = 503
  assert.equal((await fetch(base + '/health')).status, 503)
})

test('an unreachable backend is unhealthy and says nothing about where it lives', async () => {
  const closed = http.createServer()
  const port = await listen(closed)
  await new Promise(ok => closed.close(ok))
  const base = await frontend({ backendURL: `http://127.0.0.1:${port}` })
  const response = await fetch(base + '/health')
  assert.equal(response.status, 503)
  const text = await response.text()
  assert.deepEqual(JSON.parse(text), { ok: false })
  assert.doesNotMatch(text, new RegExp(String(port)))
})

test('it stays at the root under a base, and an app page at that path wins', async () => {
  const base = await frontend({}, { base: '/app/' })
  assert.deepEqual(await (await fetch(base + '/health')).json(), { ok: true })
  assert.deepEqual(await (await fetch(base + '/app/health')).json(), { ok: true })
  const own = await frontend({}, {}, { 'health/index.html': '<h1>status board</h1>' })
  assert.match(await (await fetch(own + '/health')).text(), /status board/)
})
