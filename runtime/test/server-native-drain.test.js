// A backend that is draining or restarting during an autodeploy: a read waits for it within the load budget (closed
// port, `503 DRAINING`/`OVERLOADED` or a bare 502/503/504, honoring Retry-After), a write is never repeated, and an
// answer the backend gave on purpose (its own error code) or a wait longer than the budget fails at once.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { EventEmitter } from 'node:events'
import { createServer } from 'node:http'
import { nativeAuthBridge } from '../server/server.mjs'

const freePort = async () => {
  const probe = createServer(); await new Promise(r => probe.listen(0, '127.0.0.1', r))
  const port = probe.address().port; await new Promise(r => probe.close(r)); return port
}
const listen = async (handler, port = 0) => { const s = createServer(handler); await new Promise(r => s.listen(port, '127.0.0.1', r)); return s }
const stop = s => new Promise(r => { s.close(r); s.closeAllConnections() })
const resource = (bridge, method) => {
  const req = new EventEmitter(), res = new EventEmitter()
  req.method = method; req.headers = { host: 'app.example.test', origin: 'https://app.example.test' }
  res.writableEnded = false; res.destroyed = false
  return bridge.resource(req, res, '00000000-0000-4000-8000-000000000007', 'notes')
}
const answer = (res, status, code, retryAfter) => {
  res.writeHead(status, { 'content-type': 'application/json', ...(retryAfter ? { 'retry-after': String(retryAfter) } : {}) })
  res.end(code ? JSON.stringify({ error: { code, message: 'x' } }) : '')
}

test('a read outlasts a restart longer than the old 3.75 s retry window', async () => {
  const port = await freePort()
  const bridge = nativeAuthBridge(`http://127.0.0.1:${port}`, undefined, undefined, undefined, undefined, 8000)
  let up
  const later = setTimeout(async () => { up = await listen((req, res) => { req.resume(); res.writeHead(200, { 'content-type': 'application/json' }); res.end('{"items":[]}') }, port) }, 4500)
  try { assert.deepEqual(await resource(bridge, 'GET').list({ limit: 1 }), { items: [] }) }
  finally { clearTimeout(later); bridge.close(); if (up) await stop(up) }
})

test('a read retries a draining or overloaded answer after its Retry-After; a write is sent once', async () => {
  const seen = []
  const backend = await listen((req, res) => {
    req.resume(); seen.push({ method: req.method, at: Date.now() })
    const reads = seen.filter(s => s.method === 'GET').length
    if (req.method !== 'GET') return answer(res, 503, 'DRAINING', 1)
    if (reads === 1) return answer(res, 503, 'DRAINING', 1)
    if (reads === 2) return answer(res, 502)
    if (reads === 3) return answer(res, 503, 'OVERLOADED', 1)
    res.writeHead(200, { 'content-type': 'application/json' }); res.end('{"items":[1]}')
  })
  const bridge = nativeAuthBridge(`http://127.0.0.1:${backend.address().port}`, undefined, undefined, undefined, undefined, 8000)
  try {
    assert.deepEqual(await resource(bridge, 'GET').list({ limit: 1 }), { items: [1] })
    const reads = seen.filter(s => s.method === 'GET')
    assert.equal(reads.length, 4)
    assert.ok(reads[1].at - reads[0].at >= 950, 'the first retry waited for Retry-After: 1')
    await assert.rejects(resource(bridge, 'POST').create({ title: 'x' }), e => e.status === 503 && e.code === 'DRAINING' && e.temporary === true)
    assert.equal(seen.filter(s => s.method === 'POST').length, 1)
  } finally { bridge.close(); await stop(backend) }
})

test('an answer with its own error code, or a Retry-After beyond the budget, fails without waiting', async () => {
  let reads = 0, mode = 'own'
  const backend = await listen((req, res) => {
    req.resume(); reads++
    if (mode === 'own') return answer(res, 503, 'BATCH_TOO_SLOW')
    return answer(res, 503, 'OVERLOADED', 30)
  })
  const bridge = nativeAuthBridge(`http://127.0.0.1:${backend.address().port}`, undefined, undefined, undefined, undefined, 2000)
  try {
    await assert.rejects(resource(bridge, 'GET').list(), e => e.code === 'BATCH_TOO_SLOW')
    assert.equal(reads, 1)
    mode = 'busy'; reads = 0
    const t0 = Date.now()
    await assert.rejects(resource(bridge, 'GET').list(), e => e.status === 503 && e.code === 'OVERLOADED' && e.retryAfter === 30)
    assert.equal(reads, 1)
    assert.ok(Date.now() - t0 < 1000, 'a 30 s Retry-After cannot fit a 2 s budget: fail now')
  } finally { bridge.close(); await stop(backend) }
})

test('a closed port past the budget fails as temporary', async () => {
  const port = await freePort()
  const bridge = nativeAuthBridge(`http://127.0.0.1:${port}`, undefined, undefined, undefined, undefined, 600)
  const t0 = Date.now()
  try { await assert.rejects(resource(bridge, 'GET').list(), e => e.status === 503 && e.temporary === true) }
  finally { bridge.close() }
  assert.ok(Date.now() - t0 < 2000, 'the budget bounds the wait')
})

test('a file read through ctx.backend.files waits for a draining backend too; an upload is sent once', async () => {
  let lists = 0, uploads = 0
  const backend = await listen((req, res) => {
    req.resume()
    if (req.method === 'POST') { uploads++; return answer(res, 503, 'DRAINING', 1) }
    if (++lists === 1) return answer(res, 503, 'DRAINING', 1)
    res.writeHead(200, { 'content-type': 'application/json' }); res.end('{"items":[]}')
  })
  const bridge = nativeAuthBridge(`http://127.0.0.1:${backend.address().port}`, undefined, 'synthetic-service-key', undefined, undefined, 8000)
  const req = new EventEmitter(), res = new EventEmitter()
  req.method = 'GET'; req.headers = { host: 'app.example.test' }; res.writableEnded = false; res.destroyed = false
  const files = bridge.backend(req, res, '00000000-0000-4000-8000-000000000008').files('docs')
  try {
    assert.deepEqual(await files.list(), { items: [] })
    assert.equal(lists, 2)
    await assert.rejects(files.upload({ filename: 'a.txt', data: 'x' }), e => e.status === 503 && e.temporary === true)
    assert.equal(uploads, 1)
  } finally { bridge.close(); await stop(backend) }
})
