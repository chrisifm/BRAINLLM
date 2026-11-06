import { test, after } from 'node:test'
import assert from 'node:assert/strict'
import { Readable } from 'node:stream'
import { EventEmitter } from 'node:events'
import { createServer } from 'node:http'
import { nativeAuthBridge } from '../server/server.mjs'
const seen = []
const backend = createServer((req, res) => {
  const chunks = []
  req.on('data', chunk => chunks.push(chunk))
  req.on('end', () => {
    seen.push({ path: req.url, headers: req.headers, body: Buffer.concat(chunks).toString() })
    res.writeHead(204, { 'set-cookie': '__Host-brain=synthetic; Path=/; Secure; HttpOnly' }); res.end()
  })
})
await new Promise(resolve => backend.listen(0, '127.0.0.1', resolve))
const bridge = nativeAuthBridge(`http://127.0.0.1:${backend.address().port}`)
after(async () => { bridge.close(); await new Promise(resolve => { backend.close(resolve); backend.closeAllConnections() }) })
function request(body) {
  const req = Readable.from([Buffer.from(body)])
  req.method = 'POST'; req.headers = { origin: 'https://example.test', 'content-type': 'application/json', 'content-length': String(Buffer.byteLength(body)), authorization: 'Bearer private', 'x-api-key': 'private', cookie: '__Host-brain=private', 'user-agent': 'private-agent', referer: 'https://private.example.test/path', 'x-forwarded-for': '203.0.113.5', 'cf-ipcountry': 'US', dnt: '1', 'sec-gpc': '1' }
  return req
}
test('native performance proxy preserves privacy signals and Origin while dropping all identity and response cookies', async () => {
  const body = JSON.stringify({ route: '/items/:id', consent: true, metrics: [{ name: 'TTFB', value: 42 }] })
  const req = request(body), res = new EventEmitter(); res.destroyed = false
  let answer
  await bridge.proxy(req, res, new URL('https://example.test/_brain/vitals'), 'synthetic-request-id', (_res, status, bytes, headers) => { answer = { status, headers } })
  assert.equal(answer.status, 204); assert.equal(answer.headers['set-cookie'], undefined)
  assert.equal(seen.at(-1).path, '/_brain/vitals'); assert.equal(seen.at(-1).body, body)
  for (const key of ['authorization','x-api-key','cookie','user-agent','referer','x-forwarded-for','x-request-id','cf-ipcountry']) assert.equal(seen.at(-1).headers[key], undefined, key)
  for (const key of ['origin','content-type','dnt','sec-gpc']) assert.equal(seen.at(-1).headers[key], req.headers[key])
})
test('native performance proxy rejects over-budget bodies before opening an upstream request', () => {
  const before = seen.length, req = request('x'.repeat(4097)), res = new EventEmitter()
  assert.throws(() => bridge.proxy(req, res, new URL('https://example.test/_brain/vitals'), 'ref', () => {}), error => error.status === 413)
  assert.equal(seen.length, before)
})
