// #235 `<meta cache="N">` in server.mjs: the SSR HTML of anonymous GETs is reused for N s per URL, then served stale
// while it regenerates in the background; requests with a session or Authorization always render.
import { test, after } from 'node:test'
import assert from 'node:assert/strict'
import { copyFileSync, mkdirSync, mkdtempSync, writeFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = fileURLToPath(new URL('.', import.meta.url))
const dist = mkdtempSync(join(tmpdir(), 'brain-cache-'))
const w = (rel, s) => (mkdirSync(join(dist, rel, '..'), { recursive: true }), writeFileSync(join(dist, rel), s))
mkdirSync(join(dist, '_b'), { recursive: true })
copyFileSync(join(here, '../brain.js'), join(dist, '_b/brain.js'))
mkdirSync(join(dist, '_server'), { recursive: true })
copyFileSync(join(here, '../shim/dom.js'), join(dist, '_server/dom.js'))
w('_b/n.js', `import { tpl } from '/_b/brain.js'\nexport default function N(p) { return tpl('<p>call ' + p.n + '</p>')() }\n`)
w('_server/n.server.mjs', `let n = 0\nexport function load(ctx) { if (ctx.url.searchParams.has('session')) ctx.session.set({ name: 'Test' }); return { n: ++n } }\n`)
w('_server/pages/p.html', '<!doctype html><html><head><title>t</title></head><body><div id="brain-view"><!--brain:ssr--></div><script type="application/json" id="brain-props"><!--brain:props--></script></body></html>')
w('_server/manifest.json', JSON.stringify({
  v: 1, build: 'b1', core: '/_b/brain.js', rpc: {},
  routes: [{ path: '/n', rx: '^/n/?$', view: '/_b/n.js', layouts: [], server: '_server/n.server.mjs', page: '_server/pages/p.html', cache: 1 }],
}))

const { createServer } = await import('../server/server.mjs')
const server = createServer({ dist, secret: 'test-only-session-key', log() {} })
await new Promise(r => server.listen(0, '127.0.0.1', r))
const base = `http://127.0.0.1:${server.address().port}`
after(async () => { await new Promise(r => server.close(r)); rmSync(dist, { recursive: true, force: true }) })
const get = (p, h = {}) => fetch(base + p, { headers: h })
const call = async (p, h) => (await (await get(p, h)).text()).match(/call (\d+)/)[1]
const wait = ms => new Promise(r => setTimeout(r, ms))

test('<meta cache>: same HTML within N s (fresh nonce each time), stale-while-revalidate after, never for a session', async () => {
  const r1 = await get('/n')
  assert.equal(r1.headers.get('cache-control'), 'public, s-maxage=1, stale-while-revalidate=1')
  const t1 = await r1.text()
  assert.match(t1, /call 1/)
  const r2 = await get('/n')
  const t2 = await r2.text()
  assert.match(t2, /call 1/) // load() did not run again
  const nonce = t => (t.match(/nonce="([^"]+)"/) || [])[1]
  assert.notEqual(nonce(t1), nonce(t2)) // the CSP nonce is still per response
  assert.equal(await call('/n?x=1'), '2') // another URL, its own entry
  await wait(1100)
  assert.equal(await call('/n'), '1') // stale: served at once…
  await wait(50)
  assert.equal(await call('/n'), '3') // …while it regenerated in the background
  assert.equal(await call('/n', { cookie: '__Host-session=abc.def' }), '4') // a session always renders
  assert.equal(await call('/n', { authorization: 'Bearer x' }), '5')
  await wait(2100)
  assert.equal(await call('/n'), '6') // older than 2N: rendered before answering
})

test('SSR cache bypasses custom cookies and never stores a response that sets a session', async () => {
  const a = await call('/n?custom=1', { cookie: 'custom_session=a' })
  const b = await call('/n?custom=1', { cookie: 'custom_session=b' })
  assert.notEqual(a, b)
  const r1 = await get('/n?session=1'), r2 = await get('/n?session=1')
  assert.equal(r1.headers.get('cache-control'), 'no-store')
  assert.ok(r1.headers.get('set-cookie'))
  assert.notEqual((await r1.text()).match(/call (\d+)/)[1], (await r2.text()).match(/call (\d+)/)[1])
})
