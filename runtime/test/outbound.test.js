import { test } from 'node:test'
import assert from 'node:assert/strict'
import { createServer } from 'node:http'
import { httpCall, HttpCallError } from '../brain-outbound.js'

function serve(handler) {
  return new Promise(resolve => {
    const server = createServer(handler)
    server.listen(0, '127.0.0.1', () => resolve({ server, url: `http://127.0.0.1:${server.address().port}` }))
  })
}
const fast = { allowPrivate: true, backoff: 1 }

test('json request, response parsing and header shape', async () => {
  const { server, url } = await serve((req, res) => {
    let body = ''
    req.on('data', c => (body += c))
    req.on('end', () => {
      res.setHeader('content-type', 'application/json')
      res.end(JSON.stringify({ method: req.method, type: req.headers['content-type'], body: JSON.parse(body) }))
    })
  })
  const out = await httpCall(`${url}/quotes`, { method: 'POST', json: { items: 2 }, ...fast })
  assert.equal(out.status, 200)
  assert.equal(out.ok, true)
  assert.deepEqual(out.json, { method: 'POST', type: 'application/json', body: { items: 2 } })
  assert.equal(out.attempts, 1)
  server.close()
})

test('idempotent retries reuse one Idempotency-Key and honor Retry-After', async () => {
  const keys = []
  let calls = 0
  const { server, url } = await serve((req, res) => {
    keys.push(req.headers['idempotency-key'])
    calls++
    if (calls < 3) { res.statusCode = calls === 1 ? 503 : 429; res.setHeader('retry-after', '0'); return res.end() }
    res.end('done')
  })
  const out = await httpCall(`${url}/charges`, { method: 'POST', json: { amount: 42 }, idempotencyKey: true, retries: 3, ...fast })
  assert.equal(out.text, 'done')
  assert.equal(out.attempts, 3)
  assert.equal(new Set(keys).size, 1)
  assert.match(keys[0], /^[0-9a-f-]{36}$/)
  server.close()
})

test('POST without an idempotency key is never retried', async () => {
  let calls = 0
  const { server, url } = await serve((req, res) => { calls++; res.statusCode = 503; res.end() })
  const err = await httpCall(`${url}/orders`, { method: 'POST', json: {}, retries: 3, ...fast }).catch(e => e)
  assert.ok(err instanceof HttpCallError)
  assert.equal(err.code, 'HTTP_STATUS')
  assert.equal(err.status, 503)
  assert.equal(calls, 1)
  server.close()
})

test('4xx answers return without retry and without throwing', async () => {
  let calls = 0
  const { server, url } = await serve((req, res) => { calls++; res.statusCode = 404; res.end('{}') })
  const out = await httpCall(`${url}/missing`, { retries: 3, ...fast })
  assert.equal(out.status, 404)
  assert.equal(out.ok, false)
  assert.equal(calls, 1)
  server.close()
})

test('timeouts and oversized bodies fail with redacted errors', async () => {
  const { server, url } = await serve((req, res) => {
    if (req.url.startsWith('/slow')) return setTimeout(() => res.end('late'), 500)
    res.end('x'.repeat(4096))
  })
  const slow = await httpCall(`${url}/slow?token=SECRETVALUE`, { timeout: 50, retries: 0, headers: { authorization: 'Bearer SECRETVALUE' }, ...fast }).catch(e => e)
  assert.equal(slow.code, 'TIMEOUT')
  assert.ok(!JSON.stringify({ m: slow.message, u: slow.url }).includes('SECRETVALUE'))
  assert.ok(!slow.url.includes('?'))
  const big = await httpCall(`${url}/big`, { maxBytes: 1024, ...fast }).catch(e => e)
  assert.equal(big.code, 'TOO_LARGE')
  server.close()
})

test('unsafe targets are refused before any request', async () => {
  for (const target of ['ftp://example.test/a', 'http://user:pass@example.test/', 'http://127.0.0.1/', 'http://localhost:9/', 'http://10.1.2.3/', 'http://169.254.169.254/latest', 'http://[::1]/', 'http://192.168.0.4/']) {
    const err = await httpCall(target, { retries: 0 }).catch(e => e)
    assert.equal(err.code, 'UNSAFE_URL', target)
  }
  await assert.rejects(httpCall('https://example.test/', { timeout: 0 }), /timeout/)
  await assert.rejects(httpCall('https://example.test/', { retries: 9 }), /retries/)
})
