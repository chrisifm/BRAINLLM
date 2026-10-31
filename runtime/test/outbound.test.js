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

const toLocal = (hostname, options, callback) => {
  const list = [{ address: '127.0.0.1', family: 4 }]
  return options.all ? callback(null, list) : callback(null, '127.0.0.1', 4)
}

test('encoded, mapped and translated private literals are refused', async () => {
  for (const target of ['http://[::ffff:127.0.0.1]/', 'http://[::ffff:a9fe:a9fe]/', 'http://[::7f00:1]/', 'http://[64:ff9b::a00:1]/', 'http://[2002:a9fe:a9fe::]/', 'http://[ff02::1]/', 'http://[fec0::1]/', 'http://[fd00:ec2::254]/', 'http://0.0.0.0/', 'http://100.64.0.1/', 'http://198.18.0.1/', 'http://224.0.0.1/', 'http://0x7f.1/', 'http://metadata.google.internal/']) {
    const err = await httpCall(target, { retries: 0 }).catch(e => e)
    assert.equal(err.code, 'UNSAFE_URL', target)
  }
})

test('a public name that resolves to a private address is refused before connecting', async () => {
  let calls = 0
  const { server, url } = await serve((req, res) => { calls++; res.end('reached') })
  const port = new URL(url).port
  const mixed = (h, o, cb) => { const list = [{ address: '93.184.216.34', family: 4 }, { address: '10.0.0.7', family: 4 }]; return o.all ? cb(null, list) : cb(null, list[0].address, 4) }
  for (const lookup of [toLocal, mixed]) {
    const err = await httpCall(`http://api.example.test:${port}/v1?key=SECRET`, { retries: 0, lookup }).catch(e => e)
    assert.ok(err instanceof HttpCallError)
    assert.equal(err.code, 'UNSAFE_URL')
    assert.ok(!err.message.includes('SECRET'))
  }
  assert.equal(calls, 0)
  const ok = await httpCall(`http://api.example.test:${port}/v1`, { lookup: toLocal, allowPrivate: ['api.example.test'] })
  assert.equal(ok.text, 'reached')
  server.close()
})

test('the development opt-out is ignored in production', async () => {
  const { server, url } = await serve((req, res) => res.end('dev'))
  const before = { allow: process.env.BRAIN_OUTBOUND_ALLOW_PRIVATE, env: process.env.NODE_ENV }
  process.env.BRAIN_OUTBOUND_ALLOW_PRIVATE = '1'
  try {
    process.env.NODE_ENV = 'development'
    assert.equal((await httpCall(url, { retries: 0 })).text, 'dev')
    process.env.NODE_ENV = 'production'
    assert.equal((await httpCall(url, { retries: 0 }).catch(e => e)).code, 'UNSAFE_URL')
  } finally {
    for (const [k, v] of [['BRAIN_OUTBOUND_ALLOW_PRIVATE', before.allow], ['NODE_ENV', before.env]]) v === undefined ? delete process.env[k] : (process.env[k] = v)
    server.close()
  }
})

test('redirects are opt-in and every hop is checked again', async () => {
  const seen = []
  const { server: target, url: targetUrl } = await serve((req, res) => { seen.push(req.headers.authorization ?? null); res.end(`final ${req.method}`) })
  const port = new URL(targetUrl).port
  const { server, url } = await serve((req, res) => {
    const to = { '/same': '/landing', '/cross': `http://svc.example.test:${port}/in`, '/private': `http://localhost:${port}/in`, '/loop': '/loop', '/down': 'https://x.example.test/' }[req.url.split('?')[0]] ?? null
    if (req.url === '/landing') return res.end('landed')
    res.statusCode = req.url === '/cross' ? 303 : 302
    res.setHeader('location', to)
    res.end()
  })
  const allow = { allowPrivate: ['127.0.0.1', 'svc.example.test'], lookup: toLocal, backoff: 1 }
  assert.equal((await httpCall(`${url}/same`, allow)).status, 302)
  assert.equal((await httpCall(`${url}/same`, { ...allow, redirects: 2 })).text, 'landed')
  const cross = await httpCall(`${url}/cross`, { ...allow, redirects: 2, method: 'POST', json: { a: 1 }, headers: { authorization: 'Bearer SECRET' } })
  assert.equal(cross.text, 'final GET')
  assert.deepEqual(seen, [null])
  assert.equal((await httpCall(`${url}/private`, { ...allow, redirects: 2 }).catch(e => e)).code, 'UNSAFE_URL')
  assert.equal((await httpCall(`${url}/loop`, { ...allow, redirects: 3 }).catch(e => e)).code, 'REDIRECT')
  assert.equal(seen.length, 1)
  server.close()
  target.close()
})
