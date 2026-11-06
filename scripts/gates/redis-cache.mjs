// Actual selected native compiler -> private production helpers -> two independent SSR workers + owned Redis.
// This gate never uses an existing REDIS_URL. Redis is a required local test tool, never a silent skip.
import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { request } from 'node:http'
import { createServer as tcpServer } from 'node:net'
import dns from 'node:dns'
import { mkdirSync, mkdtempSync, readFileSync, readdirSync, writeFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { pathToFileURL } from 'node:url'
import { ownedRedis, redisAvailable } from '../../runtime/test/helpers/owned-redis.mjs'

const binary = process.env.BRAIN_BIN
if (!binary) throw Error('Set BRAIN_BIN to the exact native binary under test')
if (!redisAvailable) throw Error('This gate requires local redis-server and redis-cli; install them before running the release gate')
const directory = mkdtempSync(join(tmpdir(), 'brain-redis-gate-')), cleanup = [], children = new Set()
const systemEnv = Object.fromEntries(['PATH', 'HOME', 'TMPDIR', 'LANG', 'SystemRoot'].filter(key => process.env[key]).map(key => [key, process.env[key]]))
const env = { ...systemEnv, NO_COLOR: '1', RAYON_NUM_THREADS: '2', PUBLIC_SITE_URL: 'https://example.com' }
function owned(command, args, cwd, extra = {}, ipc = false) {
  const child = spawn(command, args, { cwd, env: { ...env, ...extra }, detached: true, stdio: ipc ? ['ignore', 'pipe', 'pipe', 'ipc'] : ['ignore', 'pipe', 'pipe'] })
  const output = { text: '' }; children.add(child)
  const keep = data => { output.text = (output.text + data).slice(-128 * 1024) }
  child.stdout.on('data', keep); child.stderr.on('data', keep); child.once('error', () => {})
  const exited = new Promise(ok => { child.once('exit', (code, signal) => { children.delete(child); ok({ code, signal }) }); child.once('error', () => { children.delete(child); ok({ code: null, signal: null }) }) })
  const stop = async () => {
    if (!child.pid || child.exitCode !== null || child.signalCode !== null) return
    const force = setTimeout(() => { try { process.kill(-child.pid, 'SIGKILL') } catch {} }, 3000)
    try { process.kill(-child.pid, 'SIGTERM') } catch {}
    await exited; clearTimeout(force)
  }
  cleanup.push(stop)
  return { child, output, exited, stop }
}
async function cli(args, cwd, extra = {}) {
  const running = owned(binary, args, cwd, extra)
  const timer = setTimeout(running.stop, 60000)
  try { const status = await running.exited; assert.equal(status.code, 0, `native command failed: ${running.output.text.slice(-2000)}`); assert.equal(status.signal, null); return running.output.text } finally { clearTimeout(timer) }
}
const write = (root, path, source) => { mkdirSync(join(root, path, '..'), { recursive: true }); writeFileSync(join(root, path), source) }
function inventory(root) {
  const result = []
  for (const entry of readdirSync(root, { withFileTypes: true })) { const path = join(root, entry.name); if (entry.isDirectory()) result.push(...inventory(path)); else if (entry.isFile()) result.push(path) }
  return result
}
async function worker(dist, redisURL, name) {
  const script = `import {createServer} from ${JSON.stringify(pathToFileURL(join(dist, 'server.mjs')).href)};const s=createServer({dist:process.argv[1],env:{REDIS_URL:process.env.REDIS_URL,BRAIN_REDIS_NAMESPACE:'example-release-gate',EXAMPLE_WORKER_NAME:process.env.EXAMPLE_WORKER_NAME},secret:'synthetic-session-secret',log(){}});s.listen(0,'127.0.0.1',()=>process.send({port:s.address().port}));process.on('SIGTERM',()=>{s.closeAllConnections();s.close(()=>process.disconnect())})`
  const running = owned(process.execPath, ['--input-type=module', '-e', script, dist], dist, { REDIS_URL: redisURL, EXAMPLE_WORKER_NAME: name }, true)
  const port = await new Promise((ok, no) => {
    const timer = setTimeout(() => no(Error('Owned compiled SSR worker was not ready')), 10000)
    running.child.once('message', message => { clearTimeout(timer); ok(message.port) }); running.child.once('error', no)
    running.exited.then(() => { clearTimeout(timer); no(Error('Owned compiled SSR worker exited: ' + running.output.text.slice(-2000))) })
  })
  const get = (path, headers = {}) => new Promise((ok, no) => {
    const outgoing = request({ host: '127.0.0.1', port, path, headers: { Host: 'example.com', Accept: 'text/html', ...headers }, agent: false, timeout: 5000 }, incoming => {
      let body = ''; incoming.setEncoding('utf8'); incoming.on('data', chunk => { body += chunk; if (body.length > 2 * 1024 * 1024) outgoing.destroy() }); incoming.on('end', () => ok({ status: incoming.statusCode, headers: incoming.headers, body }))
    })
    outgoing.on('error', no); outgoing.on('timeout', () => outgoing.destroy(Error('Owned compiled SSR request timed out'))); outgoing.end()
  })
  return { get, running }
}
const props = response => {
  const found = response.body.match(/<script\b[^>]*\bid="brain-props"[^>]*>([\s\S]*?)<\/script>/)
  assert.ok(found, 'Compiled SSR props must be present')
  return JSON.parse(found[1])
}
const total = setTimeout(async () => { for (const fn of [...cleanup].reverse()) await fn(); process.exitCode = 1 }, 180000)
try {
  const redis = await ownedRedis({ after: fn => cleanup.push(fn) })
  await cli(['new', 'example-app'], directory)
  const app = join(directory, 'example-app')
  const config = JSON.parse(readFileSync(join(app, 'brain.json'), 'utf8'))
  config.compilerBranding = false
  write(app, 'brain.json', JSON.stringify(config))
  write(app, 'views/shared.brain', '<meta cache="60">\n<meta seo title="Shared page" description="A synthetic public page used to verify bounded shared server caching.">\n<script server>\nexport async function load(ctx) { return { message: ctx.env.EXAMPLE_WORKER_NAME || "public" } }\n</script>\n<main><h1>Shared page</h1><p>{props.message}</p></main>\n')
  const buildEnv = { REDIS_URL: redis.url, BRAIN_REDIS_NAMESPACE: 'example-release-gate' }
  await cli(['build'], app, buildEnv)
  const dist = join(app, 'dist'), helper = join(dist, '_server/redis-cache.mjs'), signature = '/* by BRAINLLM https://chrisifm.github.io/BRAINLLM/ */'
  assert.ok(readFileSync(join(dist, 'server.mjs'), 'utf8').includes("from './_server/redis-cache.mjs'"))
  assert.ok(readFileSync(helper, 'utf8').includes('createRedisCache')); assert.ok(!readFileSync(helper, 'utf8').includes(signature))
  for (const path of inventory(dist)) { const bytes = readFileSync(path); assert.ok(!bytes.includes(Buffer.from(redis.url)), 'Redis URL must not enter an artifact'); assert.ok(!bytes.includes(Buffer.from('synthetic-cache-password')), 'Redis credentials must not enter an artifact') }
  const syntax = owned(process.execPath, ['--check', join(dist, 'server.mjs')], app); assert.equal((await syntax.exited).code, 0, syntax.output.text)
  const helperSyntax = owned(process.execPath, ['--check', helper], app); assert.equal((await helperSyntax.exited).code, 0, helperSyntax.output.text)
  const { createRedisCache } = await import(pathToFileURL(helper).href)
  const cacheConfig = url => ({ url, namespace: 'example-wire-gate', build: 'synthetic-build', base: '/' })
  for (const corrupt of ['framing', 'http-header']) {
    const envelope = JSON.stringify({ v: 1, out: 'public page', h: corrupt === 'http-header' ? { 'content-type': 'text/html\x01' } : {}, t: Date.now(), seconds: 60 })
    const length = Buffer.from(String(Buffer.byteLength(envelope)))
    if (corrupt === 'framing') length[0] |= 0x80
    const wire = Buffer.concat([Buffer.from('$'), length, Buffer.from('\r\n' + envelope + '\r\n')])
    const sockets = new Set()
    const server = tcpServer(socket => {
      sockets.add(socket); socket.once('close', () => sockets.delete(socket)); socket.on('error', () => {})
      socket.once('data', () => socket.end(wire))
    })
    await new Promise((ok, no) => server.once('error', no).listen(0, '127.0.0.1', ok))
    const cache = createRedisCache(cacheConfig(`redis://127.0.0.1:${server.address().port}/0`))
    try { assert.equal(await cache.get('public-route', 60), null, 'compiled helper must treat malformed ' + corrupt + ' as a miss') }
    finally { cache.close(); for (const socket of sockets) socket.destroy(); await new Promise(ok => server.close(ok)) }
  }
  const originalLookup = dns.lookup, held = []
  const pendingDNS = createRedisCache(cacheConfig('redis://redis.example.test:6379/0'))
  try {
    dns.lookup = (_host, _options, callback) => held.push(callback)
    assert.deepEqual(await Promise.all(Array.from({ length: 4 }, (_, index) => pendingDNS.get('first-' + index, 60))), Array(4).fill(null))
    assert.deepEqual(await Promise.all(Array.from({ length: 8 }, (_, index) => pendingDNS.get('second-' + index, 60))), Array(8).fill(null))
    assert.equal(held.length, 4, 'the compiled helper must retain unresolved DNS admission after socket timeout')
  } finally {
    pendingDNS.close(); dns.lookup = originalLookup
    for (const callback of held) callback(Object.assign(new Error('Synthetic DNS cleanup'), { code: 'ENOTFOUND' }))
  }
  const a = await worker(dist, redis.url, 'worker-one'), b = await worker(dist, redis.url, 'worker-two')
  const first = await a.get('/shared'), shared = await b.get('/shared')
  assert.equal(first.status, 200); assert.equal(shared.status, 200); assert.equal(props(first).message, 'worker-one'); assert.equal(props(shared).message, 'worker-one')
  assert.notEqual(first.body.match(/nonce="([^"]+)"/)?.[1], shared.body.match(/nonce="([^"]+)"/)?.[1])
  const privatePage = await b.get('/shared', { Cookie: 'synthetic=one' }); assert.equal(props(privatePage).message, 'worker-two')
  for (const path of ['/_server/redis-cache.mjs', '/_server/manifest.json']) { const hidden = await a.get(path); assert.equal(hidden.status, 404); assert.ok(!hidden.body.includes('createRedisCache')) }
  await a.running.stop(); await b.running.stop()
  for (const current of [a, b]) { const status = await current.running.exited; assert.equal(status.code, 0); assert.equal(status.signal, null) }
  config.compilerBranding = true; write(app, 'brain.json', JSON.stringify(config)); await cli(['build'], app, buildEnv)
  assert.ok(readFileSync(helper, 'utf8').includes(signature), 'default branding must cover the private helper')
  config.compilerBranding = false; write(app, 'brain.json', JSON.stringify(config)); await cli(['build'], app, buildEnv)
  assert.ok(!readFileSync(helper, 'utf8').includes(signature), 'explicit branding false must remove helper attribution after a rebuild')
  console.log('PASS Redis release gate: exact native build; own Redis; compiled private helper; malformed framing/header and pending DNS bounds; two SSR workers; fresh nonce; cookie bypass; no credentials; branding transitions')
} finally {
  clearTimeout(total)
  for (const fn of [...cleanup].reverse()) await fn()
  for (const child of children) { try { process.kill(-child.pid, 'SIGKILL') } catch {} }
  rmSync(directory, { recursive: true, force: true })
}
