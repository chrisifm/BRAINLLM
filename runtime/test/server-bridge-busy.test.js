// A busy or unreachable backend is a temporary answer: the caller reads a stable code, the fix and when to retry.
import { test, after } from 'node:test'
import assert from 'node:assert/strict'
import { createServer as http } from 'node:http'
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { createServer } from '../server/server.mjs'
const dist = mkdtempSync(join(tmpdir(), 'brain-bridge-busy-')); mkdirSync(join(dist, '_server'))
writeFileSync(join(dist, '_server/manifest.json'), JSON.stringify({ routes: [], rpc: {} }))
const held = new Set()
const backend = http((req, res) => { held.add(res); res.once('close', () => held.delete(res)) })
await new Promise((resolve) => backend.listen(0, '127.0.0.1', resolve))
const logs = []
const server = createServer({ dist, backendURL: `http://127.0.0.1:${backend.address().port}`, log: (level, msg, extra) => logs.push({ level, msg, extra }) })
await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve))
const base = `http://127.0.0.1:${server.address().port}`
after(async () => {
  for (const res of held) res.destroy()
  server.closeAllConnections?.(); backend.closeAllConnections?.()
  await new Promise((resolve) => server.close(resolve)); await new Promise((resolve) => backend.close(resolve))
  rmSync(dist, { recursive: true, force: true })
})

test('a saturated bridge answers a stable code, the fix and a retry delay', async () => {
  const controller = new AbortController()
  const waiting = Array.from({ length: 8 }, (_, i) => fetch(`${base}/api/items?i=${i}`, { signal: controller.signal }).catch(() => null))
  for (let i = 0; i < 100 && held.size < 8; i++) await new Promise((resolve) => setTimeout(resolve, 20))
  assert.equal(held.size, 8)
  const res = await fetch(`${base}/api/items`)
  const body = await res.json()
  assert.equal(res.status, 503)
  assert.equal(res.headers.get('retry-after'), '1')
  assert.equal(body.error.code, 'BRIDGE_BUSY')
  assert.match(body.error.fix, /Retry-After/)
  assert.ok(body.rid)
  const line = logs.find((entry) => entry.msg === 'backend bridge busy')
  assert.equal(line?.level, 'warn', 'a busy bridge is expected load, not an unhandled error')
  assert.equal(logs.some((entry) => entry.msg === 'unhandled'), false)
  controller.abort(); await Promise.all(waiting)
})

test('an API path stays machine-readable for a caller that prefers html', async () => {
  const controller = new AbortController()
  const waiting = Array.from({ length: 8 }, (_, i) => fetch(`${base}/api/items?h=${i}`, { signal: controller.signal }).catch(() => null))
  for (let i = 0; i < 100 && held.size < 8; i++) await new Promise((resolve) => setTimeout(resolve, 20))
  const res = await fetch(`${base}/api/items`, { headers: { accept: 'text/html' } })
  assert.equal(res.status, 503)
  assert.equal(res.headers.get('retry-after'), '1')
  assert.match(res.headers.get('content-type'), /application\/json/)
  assert.equal((await res.json()).error.code, 'BRIDGE_BUSY')
  controller.abort(); await Promise.all(waiting)
})
