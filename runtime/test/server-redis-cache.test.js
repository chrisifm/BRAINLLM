// End-to-end public SSR reuse across independent Node processes, using only a disposable authenticated Redis.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { request } from 'node:http'
import { copyFileSync, mkdirSync, mkdtempSync, writeFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { createRedisCache } from '../server/redis-cache.mjs'
import { ownedRedis, redisAvailable } from './helpers/owned-redis.mjs'
const wait = ms => new Promise(ok => setTimeout(ok, ms))

function fixture(t, build = 'synthetic-shared-build') {
  const dist = mkdtempSync(join(tmpdir(), 'brain-redis-ssr-'))
  t.after(() => rmSync(dist, { recursive: true, force: true }))
  const write = (path, text) => { mkdirSync(join(dist, path, '..'), { recursive: true }); writeFileSync(join(dist, path), text) }
  mkdirSync(join(dist, '_b'), { recursive: true }); mkdirSync(join(dist, '_server'), { recursive: true })
  copyFileSync(new URL('../brain.js', import.meta.url), join(dist, '_b/brain.js'))
  copyFileSync(new URL('../shim/dom.js', import.meta.url), join(dist, '_server/dom.js'))
  write('_b/view.js', `import {tpl} from '/_b/brain.js'; export default function View(p) {return tpl('<p>'+p.message+'</p>')()}`)
  write('_server/load.mjs', `let count=0; export function load(ctx) {if(ctx.query.mode==='session')ctx.session.set({name:'Synthetic'});return {message:ctx.env.EXAMPLE_WORKER_NAME+'_'+(++count)+'_'+(ctx.req.headers['x-example-account']||'public')}}`)
  write('_server/middleware.mjs', `export default async (ctx,next)=>{if(ctx.query.mode==='vary')ctx.headers.Vary='Accept-Language';if(ctx.query.mode==='private')ctx.headers['Cache-Control']='private, no-store';await next()}`)
  write('_server/pages/page.html', '<!doctype html><html><head><title>Example</title></head><body><div id="brain-view"><!--brain:ssr--></div><script type="application/json" id="brain-props"><!--brain:props--></script></body></html>')
  const route = (path, cache) => ({ path, rx: `^${path}/?$`, view: '/_b/view.js', layouts: [], server: '_server/load.mjs', page: '_server/pages/page.html', cache, middleware: ['_server/middleware.mjs'] })
  write('_server/manifest.json', JSON.stringify({ v: 1, build, core: '/_b/brain.js', rpc: {}, routes: [route('/n', 60), route('/stale', 1)] }))
  return dist
}
async function worker(t, dist, redisURL, name, namespace = 'example-app') {
  const script = `import {createServer} from ${JSON.stringify(new URL('../server/server.mjs', import.meta.url).href)}; const server=createServer({dist:process.argv[1],env:{REDIS_URL:process.env.REDIS_URL,BRAIN_REDIS_NAMESPACE:process.env.BRAIN_REDIS_NAMESPACE,EXAMPLE_WORKER_NAME:process.env.EXAMPLE_WORKER_NAME},secret:'synthetic-session-secret',log(){}});server.listen(0,'127.0.0.1',()=>process.send({port:server.address().port}));process.on('message',message=>{if(message==='stop'){server.closeAllConnections();server.close(()=>process.disconnect())}})`
  const child = spawn(process.execPath, ['--input-type=module', '-e', script, dist], { env: { ...process.env, REDIS_URL: redisURL, BRAIN_REDIS_NAMESPACE: namespace, EXAMPLE_WORKER_NAME: name }, stdio: ['ignore', 'pipe', 'pipe', 'ipc'] })
  let output = ''
  child.stdout.on('data', data => { output = (output + data).slice(-8192) }); child.stderr.on('data', data => { output = (output + data).slice(-8192) })
  const exited = new Promise(ok => child.once('exit', (code, signal) => ok({ code, signal })))
  t.after(async () => {
    if (child.exitCode !== null || child.signalCode !== null) return
    const force = setTimeout(() => child.kill('SIGKILL'), 3000)
    child.send('stop'); const status = await exited; clearTimeout(force)
    assert.equal(status.code, 0, output); assert.equal(status.signal, null)
  })
  const port = await new Promise((ok, no) => {
    const timer = setTimeout(() => no(Error('Owned SSR worker did not become ready')), 5000)
    child.once('message', message => { clearTimeout(timer); ok(message.port) }); child.once('error', no)
    exited.then(() => { clearTimeout(timer); no(Error('Owned SSR worker exited before readiness: ' + output)) })
  })
  const get = (path, headers = {}, method = 'GET', host = 'example.com') => new Promise((ok, no) => {
    const outgoing = request({ host: '127.0.0.1', port, path, method, headers: { Host: host, Accept: 'text/html', ...headers }, agent: false, timeout: 5000 }, incoming => {
      let body = ''; incoming.setEncoding('utf8'); incoming.on('data', chunk => { body += chunk; if (body.length > 2 * 1024 * 1024) outgoing.destroy() }); incoming.on('end', () => ok({ body, status: incoming.statusCode, headers: incoming.headers }))
    })
    outgoing.on('error', no); outgoing.on('timeout', () => outgoing.destroy(Error('Owned SSR request timed out'))); outgoing.end()
  })
  return { get, child, output: () => output }
}
const message = response => response.body.match(/<p>([^<]+)<\/p>/)?.[1]
const nonce = response => response.body.match(/nonce="([^"]+)"/)?.[1]

test('SSR Redis cache shares public HTML across workers, keeps fresh nonces and isolates query/build/namespace/host', { skip: !redisAvailable, timeout: 20000 }, async t => {
  const redis = await ownedRedis(t), dist = fixture(t)
  const a = await worker(t, dist, redis.url, 'worker-one'), b = await worker(t, dist, redis.url, 'worker-two')
  const first = await a.get('/n?a=1&b=2'); assert.equal(first.status, 200); assert.equal(message(first), 'worker-one_1_public')
  const shared = await b.get('/n?b=2&utm_source=sample&a=1'); assert.equal(message(shared), message(first)); assert.notEqual(nonce(first), nonce(shared))
  assert.equal((await b.get('/n?a=1&b=2', {}, 'HEAD')).body, '')
  assert.match(message(await b.get('/n?a=1&b=3')), /^worker-two_/)
  assert.match(message(await b.get('/n?a=1&b=2', {}, 'GET', 'other.example.com')), /^worker-two_/)
  const other = await worker(t, dist, redis.url, 'other-namespace', 'other-app'); assert.match(message(await other.get('/n?a=1&b=2')), /^other-namespace_/)
  const next = await worker(t, fixture(t, 'next-build'), redis.url, 'next-build'); assert.match(message(await next.get('/n?a=1&b=2')), /^next-build_/)
  for (const path of ['/_server/redis-cache.mjs', '/_server/manifest.json']) { const hidden = await a.get(path); assert.equal(hidden.status, 404); assert.ok(!hidden.body.includes('createRedisCache')) }
})
test('Cookie, Authorization, Range, no-store and app headers bypass existing shared and local entries', { skip: !redisAvailable, timeout: 20000 }, async t => {
  const redis = await ownedRedis(t), dist = fixture(t), a = await worker(t, dist, redis.url, 'worker-one'), b = await worker(t, dist, redis.url, 'worker-two')
  const path = '/n?identity=1', publicPage = await a.get(path); assert.equal(message(await b.get(path)), message(publicPage))
  for (const headers of [{ Cookie: 'synthetic=one' }, { Cookie: '' }, { Authorization: 'Bearer synthetic' }, { Authorization: '' }, { Range: 'bytes=0-1' }, { 'Cache-Control': 'no-store' }, { 'X-Example-Account': 'private-one' }]) {
    const privatePage = await b.get(path, headers); assert.notEqual(message(privatePage), message(publicPage)); assert.match(message(privatePage), /^worker-two_/)
  }
  assert.equal(message(await b.get(path)), message(publicPage), 'bypassed responses must not replace a public entry')
  for (const mode of ['session', 'private', 'vary']) {
    const first = await a.get('/n?mode=' + mode), second = await b.get('/n?mode=' + mode)
    assert.notEqual(message(first), message(second)); assert.equal(first.headers['cache-control'], mode === 'private' ? 'private, no-store' : 'no-store')
    if (mode === 'session') assert.ok(first.headers['set-cookie'])
  }
  assert.ok(!a.output().includes('synthetic-cache-password')); assert.ok(!b.output().includes('synthetic-cache-password'))
})
test('cross-worker stale HTML revalidates and unavailable Redis preserves local public cache', { skip: !redisAvailable, timeout: 20000 }, async t => {
  const redis = await ownedRedis(t), dist = fixture(t), a = await worker(t, dist, redis.url, 'worker-one'), b = await worker(t, dist, redis.url, 'worker-two')
  const first = await a.get('/stale'); await wait(1100)
  assert.equal(message(await b.get('/stale')), message(first), 'a stale shared copy is served while this worker renders')
  let refreshed
  for (let i = 0; i < 20; i++) { refreshed = await b.get('/stale'); if (message(refreshed)?.startsWith('worker-two_')) break; await wait(25) }
  assert.match(message(refreshed), /^worker-two_/)
  const offline = await worker(t, dist, 'redis://127.0.0.1:1/0', 'offline-worker')
  const before = Date.now(), local = await offline.get('/n?offline=1'); assert.equal(local.status, 200); assert.equal(message(await offline.get('/n?offline=1')), message(local)); assert.ok(Date.now() - before < 1500)
  const filler = createRedisCache({ url: redis.url, namespace: 'owned-capacity-fixture', build: 'synthetic-build' }); t.after(() => filler.close())
  let full = false
  for (let i = 0; i < 40; i++) if (!await filler.set('filler-' + i, { out: 'x'.repeat(512 * 1024), h: {}, cacheable: true, t: Date.now() }, 60)) { full = true; break }
  assert.equal(full, true)
  // A large SET can fail while small HTML still fits. This exclusively owned instance now has zero write headroom.
  assert.equal(redis.cli(['CONFIG', 'SET', 'maxmemory', '1']), 'OK')
  assert.match(redis.cli(['SET', 'owned-capacity-probe', 'x', 'EX', '60']), /OOM/)
  const unretained = await a.get('/n?full=1'); assert.equal(unretained.status, 200)
  assert.equal(message(await a.get('/n?full=1')), message(unretained), 'a rejected Redis write must preserve local caching')
  const other = await b.get('/n?full=1'); assert.equal(other.status, 200); assert.match(message(other), /^worker-two_/, 'full Redis must degrade to a worker-local miss')

})
