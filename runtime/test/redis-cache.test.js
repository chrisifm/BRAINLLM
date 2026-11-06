import { test } from 'node:test'
import assert from 'node:assert/strict'
import net from 'node:net'
import dns from 'node:dns'
import tls from 'node:tls'
import { spawn, spawnSync } from 'node:child_process'
import { mkdtempSync, readFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { createRedisCache, publicCacheRequest } from '../server/redis-cache.mjs'
import { ownedRedis, redisAvailable } from './helpers/owned-redis.mjs'

const wait = ms => new Promise(ok => setTimeout(ok, ms))
const value = (out = 'public page') => ({ out, h: {}, t: Date.now(), cacheable: true })
const config = url => ({ url, namespace: 'example-app', build: 'synthetic-build', base: '/' })
// Independent wire fixture: accepts arrays of bulk strings, the commands actually sent by the adapter.
function commands(buffer) {
  let offset = 0
  const result = []
  while (offset < buffer.length) {
    const end = buffer.indexOf('\r\n', offset); if (end < 0) break
    assert.equal(buffer[offset], 42)
    const count = Number(buffer.toString('ascii', offset + 1, end)); let at = end + 2; const parts = []
    for (let i = 0; i < count; i++) {
      const tail = buffer.indexOf('\r\n', at); if (tail < 0) return { result, remaining: buffer.subarray(offset) }
      assert.equal(buffer[at], 36)
      const length = Number(buffer.toString('ascii', at + 1, tail)); if (buffer.length < tail + 2 + length + 2) return { result, remaining: buffer.subarray(offset) }
      parts.push(buffer.toString('utf8', tail + 2, tail + 2 + length)); at = tail + 2 + length + 2
    }
    result.push(parts); offset = at
  }
  return { result, remaining: buffer.subarray(offset) }
}
async function mock(t, responder, tlsOptions) {
  const live = new Set(), seen = []
  const connected = socket => {
    live.add(socket); socket.on('close', () => live.delete(socket)); socket.on('error', () => {})
    let buffer = Buffer.alloc(0)
    socket.on('data', chunk => {
      buffer = Buffer.concat([buffer, chunk]); assert.ok(buffer.length <= 2 * 1024 * 1024)
      const parsed = commands(buffer); buffer = parsed.remaining
      for (const command of parsed.result) { seen.push(command); responder(socket, command) }
    })
  }
  const server = tlsOptions ? tls.createServer(tlsOptions, connected) : net.createServer(connected)
  server.on('tlsClientError', () => {})
  await new Promise(ok => server.listen(0, '127.0.0.1', ok))
  t.after(async () => { for (const socket of live) socket.destroy(); await new Promise(ok => server.close(ok)) })
  return { url: `${tlsOptions ? 'rediss' : 'redis'}://127.0.0.1:${server.address().port}/0`, seen, live }
}
const bulk = body => Buffer.concat([Buffer.from(`$${Buffer.byteLength(body)}\r\n`), Buffer.from(body), Buffer.from('\r\n')])

test('requires both opt-ins, valid namespace/build and a bounded standalone URL', () => {
  assert.equal(createRedisCache({ url: 'redis://127.0.0.1:1' }), null)
  assert.equal(createRedisCache({ namespace: 'example' }), null)
  for (const extra of [{ namespace: '' }, { namespace: '../other' }, { build: '' }, { url: 'http://example.com' }, { url: 'redis://example.com/0?rejectUnauthorized=false' }, { url: 'redis://example.com/5000' }, { url: 'redis://example.com/#x' }]) assert.equal(createRedisCache({ ...config('redis://127.0.0.1:1'), ...extra }), null)
})
test('identity, Range, cache directives and arbitrary app headers bypass without private key material', () => {
  const request = headers => ({ method: 'GET', headers })
  assert.equal(publicCacheRequest(request({ host: 'example.com', accept: 'text/html' })), true)
  for (const headers of [{ cookie: '' }, { authorization: '' }, { range: 'bytes=0-1' }, { 'cache-control': 'no-store' }, { 'cache-control': 'no-cache' }, { pragma: 'no-cache' }, { 'x-example-account': 'synthetic-user' }, { 'proxy-authorization': 'opaque' }]) assert.equal(publicCacheRequest(request(headers)), false)
})
test('wire AUTH/SELECT and TTL SET are bounded; keys reveal neither namespace nor URL query', async t => {
  let stored = null
  const service = await mock(t, (socket, command) => {
    if (command[0] === 'SET') { assert.equal(command[3], 'PX'); assert.ok(+command[4] > 0 && +command[4] <= 2000); stored = command[2] }
    socket.write(command[0] === 'GET' ? stored ? bulk(stored) : '$-1\r\n' : '+OK\r\n')
  })
  const cache = createRedisCache(config(service.url.replace('://', '://example%2Duser:synthetic%3Apassword@').replace('/0', '/2'))); t.after(() => cache.close())
  const identity = 'https://example.com/page?private-query=opaque'
  assert.equal(await cache.set(identity, value('public α\npage'), 1), true)
  assert.equal((await cache.get(identity, 1)).out, 'public α\npage')
  assert.deepEqual(service.seen.filter(c => c[0] === 'AUTH'), [['AUTH', 'example-user', 'synthetic:password'], ['AUTH', 'example-user', 'synthetic:password']])
  assert.deepEqual(service.seen.filter(c => c[0] === 'SELECT'), [['SELECT', '2'], ['SELECT', '2']])
  const keys = service.seen.filter(c => c[0] === 'GET' || c[0] === 'SET').map(c => c[1]); assert.equal(keys[0], keys[1]); assert.match(keys[0], /^brain:ssr:v1:[a-f0-9]{64}:[a-f0-9]{64}:[a-f0-9]{64}$/)
  assert.ok(!keys[0].includes('opaque')); assert.ok(!keys[0].includes('example-app'))
})
test('malformed, oversized, private, expired and TLS/protocol errors degrade to misses', async t => {
  let response = '$-1\r\n'
  const service = await mock(t, socket => socket.write(response)); const cache = createRedisCache(config(service.url)); t.after(() => cache.close())
  for (const raw of ['-OOM command not allowed when used memory > maxmemory\r\n', '-NOAUTH authentication required\r\n', '-MOVED 1 example.com:6379\r\n', '$99999999\r\n', '*1\r\n', '$2\r\n{}XX', bulk('not json'), bulk(JSON.stringify({ v: 1, out: 'private', h: { 'set-cookie': 'synthetic=opaque' }, t: Date.now(), seconds: 1 })), bulk(JSON.stringify({ v: 1, out: 'expired', h: {}, t: Date.now() - 3000, seconds: 1 }))]) { response = raw; assert.equal(await cache.get('identity', 1), null) }
  assert.equal(await cache.set('identity', value('a'.repeat(1024 * 1024 + 1)), 1), false)
  for (const h of [{ vary: 'Accept-Language' }, { 'set-cookie': 'synthetic=x' }, { 'cache-control': 'private' }, { 'cache-control': 'no-store' }, { 'x-private': 'opaque' }]) assert.equal(await cache.set('identity', { ...value(), h }, 1), false)
})
test('four connections, no waiting queue; timeout and teardown settle pending operations', async t => {
  const service = await mock(t, () => {})
  const cache = createRedisCache(config(service.url)); t.after(() => cache.close())
  const start = Date.now(), pending = Array.from({ length: 4 }, (_, i) => cache.get('key-' + i, 1))
  assert.equal(await cache.get('saturated', 1), null)
  assert.ok(Date.now() - start < 100)
  assert.deepEqual(await Promise.all(pending), [null, null, null, null]); assert.ok(Date.now() - start < 1000)
  await wait(20); assert.equal(service.live.size, 0); assert.equal(service.seen.length, 4)
  const again = [cache.get('a', 1), cache.get('b', 1)]; await wait(20); cache.close()
  assert.deepEqual(await Promise.all(again), [null, null]); assert.equal(await cache.get('closed', 1), null)
})
test('high-bit RESP header bytes never normalize into valid lengths or successful replies', async t => {
  const body = JSON.stringify({ v: 1, out: 'public page', h: {}, t: Date.now(), seconds: 60 })
  const length = Buffer.from(String(Buffer.byteLength(body)))
  length[0] |= 0x80
  const service = await mock(t, socket => socket.write(Buffer.concat([Buffer.from('$'), length, Buffer.from('\r\n' + body + '\r\n')])))
  const cache = createRedisCache(config(service.url)); t.after(() => cache.close())
  assert.equal(await cache.get('malformed-length', 60), null)
})
test('invalid cached HTTP header controls and non-Latin1 values are misses, never response errors', async t => {
  let header = 'text/html\x01'
  const service = await mock(t, socket => socket.write(bulk(JSON.stringify({ v: 1, out: 'public page', h: { 'content-type': header }, t: Date.now(), seconds: 60 }))))
  const cache = createRedisCache(config(service.url)); t.after(() => cache.close())
  for (header of ['text/html\x01', 'text/html\x7f', 'text/html\u0100']) {
    assert.equal(await cache.get('malformed-header', 60), null)
    assert.equal(await cache.set('malformed-header', { ...value(), h: { 'content-type': header } }, 60), false)
  }
})
test('a DNS completion after the absolute deadline never opens a late connection', async t => {
  const service = await mock(t, () => {})
  const lookup = dns.lookup; let held, options
  dns.lookup = (_host, o, callback) => { options = o; held = callback }
  const cache = createRedisCache(config(service.url.replace('127.0.0.1', 'redis.example.test'))); t.after(() => cache.close())
  try {
    const waiting = cache.get('identity', 1); assert.equal(typeof held, 'function')
    assert.equal(await waiting, null)
  } finally { dns.lookup = lookup }
  if (options.all) held(null, [{ address: '127.0.0.1', family: 4 }]); else held(null, '127.0.0.1', 4)
  await wait(30); assert.equal(service.seen.length, 0); assert.equal(service.live.size, 0)
})
test('expired DNS retains its admission lease until callback completion, bounding successive waves', { timeout: 5000 }, async t => {
  const original = dns.lookup, held = []
  dns.lookup = (_host, _options, callback) => held.push(callback)
  const cache = createRedisCache(config('redis://redis.example.test:6379/0'))
  t.after(() => cache.close())
  try {
    assert.deepEqual(await Promise.all(Array.from({ length: 4 }, (_, index) => cache.get('first-' + index, 1))), [null, null, null, null])
    assert.equal(held.length, 4)
    const start = Date.now()
    assert.deepEqual(await Promise.all(Array.from({ length: 16 }, (_, index) => cache.get('second-' + index, 1))), Array(16).fill(null))
    assert.equal(held.length, 4, 'socket timeout must not release unresolved native DNS work into another wave')
    assert.ok(Date.now() - start < 150, 'saturation falls through immediately without a waiting queue')
    held.shift()(Object.assign(new Error('Synthetic DNS refusal'), { code: 'ENOTFOUND' }))
    const next = cache.get('after-one-completion', 1)
    assert.equal(held.length, 4, 'one completed lookup releases exactly one slot')
    cache.close()
    assert.equal(await next, null)
    assert.equal(await cache.get('closed', 1), null)
  } finally {
    cache.close()
    dns.lookup = original
    for (const callback of held) callback(Object.assign(new Error('Synthetic DNS cleanup'), { code: 'ENOTFOUND' }))
  }
})
test('real owned Redis: AUTH, namespaces/builds, TTL and noeviction/full degradation', { skip: !redisAvailable, timeout: 20000 }, async t => {
  const redis = await ownedRedis(t), a = createRedisCache(config(redis.url)), b = createRedisCache({ ...config(redis.url), namespace: 'other-app' }), next = createRedisCache({ ...config(redis.url), build: 'next-build' })
  t.after(() => { a.close(); b.close(); next.close() })
  assert.equal(await a.set('same-route', value(), 1), true)
  assert.equal((await a.get('same-route', 1)).out, 'public page'); assert.equal(await b.get('same-route', 1), null); assert.equal(await next.get('same-route', 1), null)
  const keys = redis.cli(['KEYS', 'brain:ssr:v1:*']).split('\n'); assert.equal(keys.length, 1)
  const ttl = +redis.cli(['PTTL', keys[0]]); assert.ok(ttl > 0 && ttl <= 2000)
  await wait(1100); assert.ok(await a.get('same-route', 1)); await wait(1050); assert.equal(redis.cli(['EXISTS', keys[0]]), '0'); assert.equal(await a.get('same-route', 1), null)
  assert.equal(await a.set('retained', value('retained page'), 60), true)
  let full = false, writes = 0
  for (let i = 0; i < 40; i++) { if (!await a.set('filler-' + i, value('x'.repeat(512 * 1024)), 60)) { full = true; break } writes++ }
  assert.ok(full && writes > 0, 'owned 8 MiB noeviction instance must reach OOM')
  assert.equal((await a.get('retained', 60)).out, 'retained page', 'noeviction must not evict an existing cache entry')
  assert.match(redis.cli(['INFO', 'memory']), /maxmemory_policy:noeviction/)
})
test('rediss validates certificate and hostname; a trusted local CA succeeds', { timeout: 15000 }, async t => {
  if (spawnSync('openssl', ['version'], { timeout: 5000 }).status !== 0) return t.skip('openssl is required for the owned TLS fixture')
  const directory = mkdtempSync(join(tmpdir(), 'brain-redis-tls-')); t.after(() => rmSync(directory, { recursive: true, force: true }))
  const cert = join(directory, 'cert.pem'), key = join(directory, 'key.pem')
  assert.equal(spawnSync('openssl', ['req', '-x509', '-newkey', 'rsa:2048', '-nodes', '-subj', '/CN=localhost', '-addext', 'subjectAltName=IP:127.0.0.1', '-days', '1', '-keyout', key, '-out', cert], { timeout: 10000, stdio: 'ignore' }).status, 0)
  const body = JSON.stringify({ v: 1, out: 'trusted public page', h: {}, t: Date.now(), seconds: 60 })
  const service = await mock(t, socket => socket.write(bulk(body)), { key: readFileSync(key), cert: readFileSync(cert) })
  const untrusted = createRedisCache(config(service.url)); t.after(() => untrusted.close()); assert.equal(await untrusted.get('identity', 60), null)
  const script = `import {createRedisCache} from ${JSON.stringify(new URL('../server/redis-cache.mjs', import.meta.url).href)}; const c=createRedisCache(${JSON.stringify(config(service.url))}); const v=await c.get('identity',60); c.close(); process.exit(v?.out==='trusted public page'?0:1)`
  const child = spawn(process.execPath, ['--input-type=module', '-e', script], { env: { ...process.env, NODE_EXTRA_CA_CERTS: cert }, stdio: ['ignore', 'pipe', 'pipe'] })
  const timer = setTimeout(() => child.kill('SIGKILL'), 5000)
  try { assert.equal(await new Promise(ok => child.once('exit', code => ok(code))), 0) } finally { clearTimeout(timer); if (child.exitCode === null && child.signalCode === null) child.kill('SIGKILL') }
  const wrongCert = join(directory, 'wrong-cert.pem'), wrongKey = join(directory, 'wrong-key.pem')
  assert.equal(spawnSync('openssl', ['req', '-x509', '-newkey', 'rsa:2048', '-nodes', '-subj', '/CN=wrong.example.com', '-addext', 'subjectAltName=IP:127.0.0.2', '-days', '1', '-keyout', wrongKey, '-out', wrongCert], { timeout: 10000, stdio: 'ignore' }).status, 0)
  const wrong = await mock(t, socket => socket.write(bulk(body)), { key: readFileSync(wrongKey), cert: readFileSync(wrongCert) })
  const mismatchScript = `import {createRedisCache} from ${JSON.stringify(new URL('../server/redis-cache.mjs', import.meta.url).href)}; const c=createRedisCache(${JSON.stringify(config(wrong.url))}); const v=await c.get('identity',60); c.close(); process.exit(v===null?0:1)`
  const mismatch = spawn(process.execPath, ['--input-type=module', '-e', mismatchScript], { env: { ...process.env, NODE_EXTRA_CA_CERTS: wrongCert }, stdio: ['ignore', 'pipe', 'pipe'] })
  const mismatchTimer = setTimeout(() => mismatch.kill('SIGKILL'), 5000)
  try { assert.equal(await new Promise(ok => mismatch.once('exit', code => ok(code))), 0); assert.equal(wrong.seen.length, 0, 'trusted but mismatched identity must fail before Redis commands') } finally { clearTimeout(mismatchTimer); if (mismatch.exitCode === null && mismatch.signalCode === null) mismatch.kill('SIGKILL') }

})
