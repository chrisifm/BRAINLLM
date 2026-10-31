// Origin refusals of the production server (B0741, B0742, B0743): the same answers as before, a stable code for the
// client, and the seen origin, host and exact fix for the operator's log. The client never receives configuration.
import { test, after } from 'node:test'
import assert from 'node:assert/strict'
import { createServer as http } from 'node:http'
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { connect } from 'node:net'
import { createServer } from '../server/server.mjs'
import { installNativeSockets } from '../server/native-sockets.mjs'

const dist = mkdtempSync(join(tmpdir(), 'brain-origin-'))
mkdirSync(join(dist, '_server'))
writeFileSync(join(dist, '_server/account.mjs'), `export async function ping() { return 'pong' }
export async function signIn(ctx, values) { return ctx.backend.login(values) }
export async function signOut(ctx) { await ctx.session.clear(); return 'out' }
export async function save(ctx) { return ctx.backend.resource('posts').create({ title: 'One' }) }`)
writeFileSync(join(dist, '_server/manifest.json'), JSON.stringify({ routes: [], rpc: { account: '_server/account.mjs' } }))
const backend = http((req, res) => { res.writeHead(401, { 'content-type': 'application/json' }); res.end('{}') })
await new Promise(resolve => backend.listen(0, '127.0.0.1', resolve))
const logs = []
const frontend = createServer({ dist, backendURL: `http://127.0.0.1:${backend.address().port}`, secret: 'synthetic-origin-secret', log: (level, msg, extra) => logs.push({ level, msg, ...extra }) })
await new Promise(resolve => frontend.listen(0, '127.0.0.1', resolve))
const base = `http://127.0.0.1:${frontend.address().port}`
after(async () => {
  await new Promise(resolve => { frontend.close(resolve); frontend.closeAllConnections() })
  await new Promise(resolve => { backend.close(resolve); backend.closeAllConnections() })
  rmSync(dist, { recursive: true, force: true })
})
const rpc = (fn, headers = {}, body = '[]') => fetch(`${base}/_r/account/${fn}`, { method: 'POST', headers: { 'content-type': 'application/json', 'x-brain-rpc': '1', ...headers }, body })
const last = code => logs.filter(entry => entry.code === code).at(-1)

test('a cross-origin RPC keeps its 403 and message, adds only the code, and logs seen origin, host and fix', async () => {
  const response = await rpc('ping', { origin: 'https://shop.example' })
  assert.equal(response.status, 403)
  const body = await response.json()
  assert.equal(body.error, 'Cross-origin request')
  assert.equal(body.brainCode, 'B0741')
  assert.deepEqual(Object.keys(body).sort(), ['brainCode', 'error'], 'nothing about the configuration reaches the client')
  const entry = last('B0741')
  assert.equal(entry.seenOrigin, 'https://shop.example')
  assert.equal(entry.host, new URL(base).host)
  assert.match(entry.fix, /proxy_set_header Host \$host/)
  assert.match(entry.fix, /brain explain B0741/)
})

test('a cross-origin form post keeps its 403, names the code in a header and logs the reason', async () => {
  logs.length = 0
  const local = createServer({ dist: mkForm(), secret: 'synthetic-origin-secret', log: (level, msg, extra) => logs.push({ level, msg, ...extra }) })
  await new Promise(resolve => local.listen(0, '127.0.0.1', resolve))
  try {
    const target = `http://127.0.0.1:${local.address().port}/signup`
    const response = await fetch(target, { method: 'POST', headers: { origin: 'https://shop.example', 'content-type': 'application/x-www-form-urlencoded' }, body: 'a=1' })
    assert.equal(response.status, 403)
    assert.equal(response.headers.get('x-brain-code'), 'B0741')
    assert.equal(last('B0741').seenOrigin, 'https://shop.example')
    const silent = await fetch(target, { method: 'POST', headers: { 'content-type': 'application/x-www-form-urlencoded' }, body: 'a=1' })
    assert.equal(silent.status, 403)
    assert.match(last('B0741').fix, /no Origin/)
  } finally {
    await new Promise(resolve => { local.close(resolve); local.closeAllConnections() })
  }
})

function mkForm() {
  const folder = mkdtempSync(join(tmpdir(), 'brain-origin-form-'))
  mkdirSync(join(folder, '_server'))
  writeFileSync(join(folder, '_server/manifest.json'), JSON.stringify({ routes: [{ path: '/signup', rx: '^/signup/?$' }], rpc: {} }))
  return folder
}

test('the native bridge names B0742 when a server function runs without a same-origin request', async () => {
  // Sec-Fetch-Site passes the RPC check without an Origin the bridge could relay.
  for (const [fn, body] of [['signIn', '[{"email":"a@b.test","password":"synthetic-password"}]'], ['signOut', '[]'], ['save', '[]']]) {
    logs.length = 0
    const response = await rpc(fn, { 'sec-fetch-site': 'same-origin', cookie: '__Host-brain-access=native' }, body)
    assert.equal(response.status, 403, fn)
    const answer = await response.json()
    assert.match(answer.error, /B0742/, fn)
    const entry = last('B0742')
    assert.ok(entry, `${fn} logs its refusal`)
    assert.equal(entry.seenOrigin, null)
    assert.match(entry.fix, /brain explain B0742/)
    assert.equal(JSON.stringify(entry).includes('synthetic-password'), false)
  }
})

test('refusal logs are bounded and an oversized Origin is truncated before it is logged', async () => {
  logs.length = 0
  await rpc('ping', { origin: `https://${'a'.repeat(4000)}.example` })
  assert.ok(last('B0741').seenOrigin.length <= 256)
  for (let i = 0; i < 64; i++) await rpc('ping', { origin: 'https://flood.example' })
  assert.ok(logs.filter(entry => entry.code === 'B0741').length <= 16, 'a refusal is not a free log line')
})

test('a socket refused for its origin logs B0743 with the seen origin and the key to change', async () => {
  const lines = []
  const server = http()
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve))
  installNativeSockets(server, { target: `http://127.0.0.1:${backend.address().port}`, base: '/', cookie: () => '', log: (level, msg, extra) => lines.push({ level, msg, ...extra }) })
  const handshake = origin => new Promise(resolve => {
    const socket = connect(server.address().port, '127.0.0.1', () => socket.write(`GET /_brain/socket HTTP/1.1\r\nHost: shop.example\r\nOrigin: ${origin}\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Protocol: brain.v1\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n`))
    let text = ''
    socket.on('data', chunk => { text += chunk })
    socket.on('close', () => resolve(text))
    socket.on('error', () => resolve(text))
  })
  try {
    const answer = await handshake('https://shop.example/admin')
    assert.match(answer, /^HTTP\/1\.1 403/)
    assert.equal(answer.includes('allowedOrigins'), false, 'the handshake answer stays opaque')
    const entry = lines.find(line => line.code === 'NATIVE_SOCKET_ORIGIN')
    assert.equal(entry.brainCode, 'B0743')
    assert.equal(entry.seenOrigin, 'https://shop.example/admin')
    assert.match(entry.fix, /backend\.sockets\.allowedOrigins/)
  } finally {
    await new Promise(resolve => { server.close(resolve); server.closeAllConnections?.() })
  }
})

test('a sign-in the backend refused for its origin is not reported as wrong credentials', async () => {
  const answers = { status: 403, body: '{"error":{"code":"ORIGIN_DENIED","brainCode":"B0740"}}' }
  const refusing = http((req, res) => { res.writeHead(answers.status, { 'content-type': 'application/json' }); res.end(answers.body) })
  await new Promise(resolve => refusing.listen(0, '127.0.0.1', resolve))
  const gateway = createServer({ dist, backendURL: `http://127.0.0.1:${refusing.address().port}`, secret: 'synthetic-origin-secret', log: () => {} })
  await new Promise(resolve => gateway.listen(0, '127.0.0.1', resolve))
  const at = `127.0.0.1:${gateway.address().port}`
  const signIn = async () => {
    const response = await fetch(`http://${at}/_r/account/signIn`, { method: 'POST', headers: { 'content-type': 'application/json', 'x-brain-rpc': '1', origin: `http://${at}` }, body: JSON.stringify([{ email: 'shopper@example.test', password: 'synthetic-password' }]) })
    return { status: response.status, error: (await response.json()).error }
  }
  try {
    const refused = await signIn()
    assert.equal(refused.status, 403)
    assert.match(refused.error, /B0740/)
    assert.match(refused.error, /backend\.origins/)
    assert.doesNotMatch(refused.error, /Wrong email/)
    assert.doesNotMatch(refused.error, /127\.0\.0\.1/, 'the client never receives the configuration')
    Object.assign(answers, { status: 404, body: '{}' })
    assert.deepEqual(await signIn(), { status: 503, error: 'Service Unavailable' }, 'not reported as wrong credentials; 5xx text stays generic')
    Object.assign(answers, { status: 401, body: '{}' })
    assert.deepEqual(await signIn(), { status: 401, error: 'Wrong email or password' })
  } finally {
    await new Promise(resolve => { gateway.close(resolve); gateway.closeAllConnections() })
    await new Promise(resolve => { refusing.close(resolve); refusing.closeAllConnections() })
  }
})
