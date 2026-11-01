// With BRAIN_BACKEND_SESSION=app the signed session is the identity and the visitor reaches the backend as anonymous:
// a leftover backend cookie in the browser must not make a ctx.backend call act as someone else.
import { test, after } from 'node:test'
import assert from 'node:assert/strict'
import { createServer as http } from 'node:http'
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { createServer } from '../server/server.mjs'
const dist = mkdtempSync(join(tmpdir(), 'brain-app-session-')); mkdirSync(join(dist, '_server'))
writeFileSync(join(dist, '_server/data.mjs'), `export async function read(ctx) { return ctx.backend.resource('items').list() }`)
writeFileSync(join(dist, '_server/manifest.json'), JSON.stringify({ routes: [], rpc: { data: '_server/data.mjs' } }))
const seen = []
const backend = http((req, res) => {
  if (req.url === '/_brain/auth/me') { res.writeHead(401, { 'content-type': 'application/json' }); return res.end('{}') }
  seen.push(req.headers.cookie)
  res.writeHead(200, { 'content-type': 'application/json' }); res.end(JSON.stringify({ items: [], total: 0 }))
})
await new Promise((resolve) => backend.listen(0, '127.0.0.1', resolve))
const backendURL = `http://127.0.0.1:${backend.address().port}`
const servers = [], logs = []
const start = async (options) => {
  const server = createServer({ dist, backendURL, secret: 'synthetic-own-session-secret', log: (...entry) => logs.push(entry), ...options })
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve))
  servers.push(server)
  return `http://127.0.0.1:${server.address().port}`
}
after(async () => {
  for (const server of servers) { server.closeAllConnections?.(); await new Promise((resolve) => server.close(resolve)) }
  backend.closeAllConnections?.(); await new Promise((resolve) => backend.close(resolve))
  rmSync(dist, { recursive: true, force: true })
})
const read = (at) => fetch(`${at}/_r/data/read`, { method: 'POST', headers: { origin: at, 'x-brain-rpc': '1', 'content-type': 'application/json', cookie: '__Host-brain-access=leftover' }, body: '[]' })

test('app session: ctx.backend carries no backend cookie', async () => {
  seen.length = 0
  const res = await read(await start({ backendSession: 'app' }))
  assert.equal(res.status, 200, JSON.stringify(logs.slice(-2)))
  assert.equal(seen.at(-1), undefined)
})
test('backend session: ctx.backend still acts as the signed-in visitor', async () => {
  seen.length = 0
  const res = await read(await start({}))
  assert.equal(res.status, 200, JSON.stringify(logs.slice(-2)))
  assert.match(seen.at(-1) || '', /__Host-brain-access=leftover/)
})
