// Rate-limit keys (server.mjs overLimit, rpc, formPost): a key exists only for a real `<hash>/<name>` function, the
// form id is exactly `<hash>/<name>` (a suffix cannot buy a fresh budget), and the map has a hard size cap.
import { test, after } from 'node:test'
import assert from 'node:assert/strict'
import { copyFileSync, mkdirSync, mkdtempSync, realpathSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = fileURLToPath(new URL('.', import.meta.url))
const dist = realpathSync(mkdtempSync(join(tmpdir(), 'brain-dist-limit-'))) // the server imports through realpath
const w = (rel, s) => {
  mkdirSync(join(dist, rel, '..'), { recursive: true })
  writeFileSync(join(dist, rel), s)
}
mkdirSync(join(dist, '_server'), { recursive: true })
for (const f of ['brain.js', 'brain-form.js', 'brain-schema.js']) copyFileSync(join(here, '..', f), join(dist, '_server', f))
w('_server/acct.mjs', `export let runs = 0
export async function login(ctx, v) { runs++; return { redirect: '/done' } }
export async function ping() { return 'pong' }
export const $forms = { login: { email: 'str!' } }
`)
w('index.html', '<!doctype html><title>Home</title><main><h1>Home</h1></main>')
w('404.html', '<!doctype html><title>404</title><main><h1>Lost</h1></main>')
w('_server/manifest.json', JSON.stringify({
  v: 1, build: 'b1', core: '/_b/core.js',
  routes: [{ path: '/login', rx: '^/login/?$', view: '/_b/none.js', layouts: [], server: '_server/acct.mjs', forms: true }],
  rpc: { acct: '_server/acct.mjs' },
  notFound: '404.html',
}))

const { createServer, overLimit, rateLimitKeys, HITS_MAX, HITS_KEY_MAX, callForm, callRpc } = await import('../server/server.mjs')
const server = createServer({ dist, rateLimit: 3, secret: 'test-secret-0123456789', env: { BRAIN_SECRET: 's' }, log: () => {} }) // gitleaks:allow -- local test-only session secret
await new Promise(r => server.listen(0, '127.0.0.1', r))
const base = `http://127.0.0.1:${server.address().port}`
after(() => server.close())
const form = id => fetch(base + '/login', { method: 'POST', redirect: 'manual', headers: { origin: base, 'content-type': 'application/x-www-form-urlencoded' }, body: '_brain=' + encodeURIComponent(id) + '&email=a' })
const rpc = path => fetch(base + '/_r/' + path, { method: 'POST', headers: { origin: base, 'x-brain-rpc': '1', 'content-type': 'application/json' }, body: '[]' })
const acct = await import(join(dist, '_server/acct.mjs'))

test('a form id with a suffix is refused and never runs the function under a fresh budget', async () => {
  const codes = []
  for (let i = 0; i < 4; i++) codes.push((await form('acct/login')).status)
  assert.deepEqual(codes, [303, 303, 303, 429])
  const runs = acct.runs
  const bypass = []
  for (let i = 0; i < 5; i++) bypass.push((await form('acct/login/' + i)).status)
  assert.deepEqual(bypass, [400, 400, 400, 400, 400])
  assert.equal(acct.runs, runs, 'the suffixed id never reaches login()')
  assert.equal((await form('acct/login')).status, 429, 'the canonical key keeps counting')
})

test('bogus function names and oversized ids do not create rate-limit keys', async () => {
  const before = rateLimitKeys()
  for (let i = 0; i < 50; i++) {
    assert.equal((await rpc('acct/f' + 'a'.repeat(4000) + i)).status, 404)
    assert.equal((await rpc('acct/nope' + i)).status, 404)
    assert.equal((await rpc('__proto__/toString')).status, 404)
    assert.equal((await form('acct/' + 'x'.repeat(8000) + i)).status, 400)
    assert.equal((await form('acct/nope' + i)).status, 400)
  }
  assert.equal(rateLimitKeys(), before)
  assert.equal((await rpc('acct/ping')).status, 200)
  assert.equal(rateLimitKeys(), before + 1)
})

test('callRpc and callForm validate the function before they count', async () => {
  const before = rateLimitKeys()
  for (let i = 0; i < 20; i++) {
    assert.equal((await callRpc({ fn: undefined, args: [], ctx: {}, key: 'k rpc ' + i })).status, 404)
    assert.equal((await callForm({ module: { $forms: {} }, raw: '_brain=h%2Fmissing', ctx: {}, key: 'k form ' + i })).status, 400)
  }
  assert.equal(rateLimitKeys(), before)
})

test('the map never passes its cap and a long key is stored as a bounded digest', () => {
  for (let i = 0; i < HITS_MAX + 500; i++) overLimit('203.0.113.9 cap/' + i)
  assert.equal(rateLimitKeys(), HITS_MAX)
  const long = 'y'.repeat(HITS_KEY_MAX * 40)
  assert.equal(overLimit(long, 1), 0)
  assert.ok(overLimit(long, 1) > 0, 'the same long key still counts')
  assert.equal(rateLimitKeys(), HITS_MAX)
})
