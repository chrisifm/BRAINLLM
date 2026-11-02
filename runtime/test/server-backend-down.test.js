// A backend that is down is an expected 503 with its cause, not an unhandled error.
import { test, after } from 'node:test'
import assert from 'node:assert/strict'
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { createServer } from '../server/server.mjs'

const dist = mkdtempSync(join(tmpdir(), 'brain-backend-down-'))
mkdirSync(join(dist, '_server'))
writeFileSync(join(dist, '_server/manifest.json'), JSON.stringify({ routes: [] }))
const logs = []
const server = createServer({ dist, backendURL: 'http://127.0.0.1:1', secret: 'synthetic-backend-down-secret', log: (...entry) => logs.push(entry) })
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve))
const base = `http://127.0.0.1:${server.address().port}`
after(async () => {
  await new Promise(resolve => { server.close(resolve); server.closeAllConnections() })
  rmSync(dist, { recursive: true, force: true })
})

test('a backend that does not answer is a logged 503, not an unhandled error', async () => {
  const reply = await fetch(`${base}/api/categories`)
  assert.equal(reply.status, 503)
  await reply.text()
  assert.deepEqual(logs.filter(([, message]) => message === 'unhandled'), [])
  const [entry] = logs.filter(([, message]) => message === 'backend unavailable')
  assert.equal(entry[0], 'warn')
  assert.equal(entry[2].cause, 'Backend request unavailable')
})
