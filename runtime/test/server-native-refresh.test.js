// A password JWT session outlives its 5-minute access cookie: SSR renews it instead of reading as anonymous.
import { test, after } from 'node:test'
import assert from 'node:assert/strict'
import { createServer as http } from 'node:http'
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { createServer } from '../server/server.mjs'
const user = { id: '00000000-0000-4000-8000-000000000001', email: 'native@example.test', role: 'editor', verified: true }
const dist = mkdtempSync(join(tmpdir(), 'brain-native-refresh-')); mkdirSync(join(dist, '_server'))
writeFileSync(join(dist, 'package.json'), '{"type":"module"}')
writeFileSync(join(dist, '_server/read.mjs'), `export async function load(ctx) { return { redirect: ctx.user ? '/signed-in' : '/anonymous' } }`)
writeFileSync(join(dist, '_server/manifest.json'), JSON.stringify({ routes: [{ path: '/read', server: '_server/read.mjs' }] }))
let mode = 'ok'
const seen = []
const backend = http((req, res) => {
  seen.push({ path: req.url, method: req.method, cookie: req.headers.cookie, origin: req.headers.origin })
  res.setHeader('content-type', 'application/json')
  if (req.url === '/_brain/auth/refresh') {
    if (mode === 'denied') { res.statusCode = 401; return res.end('{"error":"private"}') }
    if (mode === 'down') { res.statusCode = 503; return res.end('{}') }
    res.setHeader('set-cookie', ['__Host-brain-access=fresh; Path=/; Secure; HttpOnly; SameSite=Lax; Max-Age=300', '__Host-brain-refresh=next; Path=/; Secure; HttpOnly; SameSite=Lax; Max-Age=604800', 'unrelated=never; Path=/'])
    return res.end(JSON.stringify({ user: { ...user, password_hash: 'private' }, access_token: 'private-marker' }))
  }
  const ok = /__Host-brain-access=(fresh|valid)/.test(req.headers.cookie || '')
  res.statusCode = ok ? 200 : 401
  res.end(ok ? JSON.stringify({ user }) : '{}')
})
await new Promise(resolve => backend.listen(0, '127.0.0.1', resolve))
const frontend = createServer({ dist, backendURL: `http://127.0.0.1:${backend.address().port}`, log() {} })
await new Promise(resolve => frontend.listen(0, '127.0.0.1', resolve))
const base = `http://127.0.0.1:${frontend.address().port}`
after(async () => { for (const server of [frontend, backend]) await new Promise(resolve => { server.close(resolve); server.closeAllConnections() }); rmSync(dist, { recursive: true, force: true }) })
const get = cookie => { seen.length = 0; return fetch(base + '/read', { redirect: 'manual', headers: { cookie } }) }

test('an expired access cookie with a refresh cookie renews during SSR and relays only auth cookies', async () => {
  mode = 'ok'
  const response = await get('__Host-brain-refresh=old')
  assert.equal(response.headers.get('location'), '/signed-in')
  assert.deepEqual(response.headers.getSetCookie().map(c => c.split('=')[0]), ['__Host-brain-access', '__Host-brain-refresh'])
  assert.match(response.headers.get('cache-control'), /no-store/)
  const refresh = seen.find(s => s.path === '/_brain/auth/refresh')
  assert.equal(refresh.method, 'POST'); assert.match(refresh.cookie, /__Host-brain-refresh=old/); assert.ok(refresh.origin)
})
test('a rejected access cookie also renews; a valid one never touches the refresh token', async () => {
  mode = 'ok'
  assert.equal((await get('__Host-brain-access=expired; __Host-brain-refresh=old')).headers.get('location'), '/signed-in')
  const response = await get('__Host-brain-access=valid; __Host-brain-refresh=old')
  assert.equal(response.headers.get('location'), '/signed-in')
  assert.equal(seen.filter(s => s.path === '/_brain/auth/refresh').length, 0)
  assert.ok(!seen[0].cookie.includes('refresh'), 'the refresh cookie is not sent to read the session')
})
test('a denied refresh reads as anonymous; no cookies and no refresh cookie stay anonymous without a call', async () => {
  mode = 'denied'
  const denied = await get('__Host-brain-refresh=old')
  assert.equal(denied.headers.get('location'), '/anonymous')
  assert.deepEqual(denied.headers.getSetCookie(), [])
  assert.equal((await get('unrelated=1')).headers.get('location'), '/anonymous')
  assert.equal(seen.length, 0)
})
test('an unavailable backend during refresh is unavailable, not a sign-out', async () => {
  mode = 'down'
  assert.equal((await get('__Host-brain-refresh=old')).status, 503)
})
