// A `<meta cache>` route renders once per cold request even when the answer is not a cacheable 200: an error, a
// redirect or a not-found from load() is answered from that first render, never by running load() (and its backend
// reads) a second time. Counted, never timed.
import { test, after } from 'node:test'
import assert from 'node:assert/strict'
import { copyFileSync, mkdirSync, mkdtempSync, writeFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = fileURLToPath(new URL('.', import.meta.url))
const dist = mkdtempSync(join(tmpdir(), 'brain-cache-once-'))
const w = (rel, s) => (mkdirSync(join(dist, rel, '..'), { recursive: true }), writeFileSync(join(dist, rel), s))
mkdirSync(join(dist, '_b'), { recursive: true })
copyFileSync(join(here, '../brain.js'), join(dist, '_b/brain.js'))
mkdirSync(join(dist, '_server'), { recursive: true })
copyFileSync(join(here, '../shim/dom.js'), join(dist, '_server/dom.js'))
w('_b/v.js', `import { tpl } from '/_b/brain.js'\nexport default function V(p) { return tpl('<p>' + (p.message || 'ok') + '</p>')() }\n`)
// globalThis.loads counts load() runs per mode; the mode comes from the path
w('_server/v.server.mjs', `export function load(ctx) {
  const mode = ctx.url.pathname.slice(1)
  globalThis.loads[mode] = (globalThis.loads[mode] || 0) + 1
  if (mode === 'boom') throw new Error('load failed')
  if (mode === 'away') return { redirect: '/elsewhere' }
  if (mode === 'missing') return { notFound: true }
  return {}
}\n`)
w('_server/pages/p.html', '<!doctype html><html><head><title>t</title></head><body><div id="brain-view"><!--brain:ssr--></div><script type="application/json" id="brain-props"><!--brain:props--></script></body></html>')
const route = path => ({ path, rx: `^${path}/?$`, view: '/_b/v.js', layouts: [], server: '_server/v.server.mjs', page: '_server/pages/p.html', cache: 60 })
w('_server/manifest.json', JSON.stringify({ v: 1, build: 'b1', core: '/_b/brain.js', rpc: {}, routes: ['/boom', '/away', '/missing', '/fine'].map(route) }))

globalThis.loads = {}
const { createServer } = await import('../server/server.mjs')
const server = createServer({ dist, secret: 'test-only-session-key', log() {} })
await new Promise(r => server.listen(0, '127.0.0.1', r))
const base = `http://127.0.0.1:${server.address().port}`
after(async () => { await new Promise(r => server.close(r)); rmSync(dist, { recursive: true, force: true }) })
const get = (p, h = {}) => fetch(base + p, { headers: { accept: 'text/html', ...h }, redirect: 'manual' })

test('a failing load on a cached route runs once and still answers 500', async () => {
  const res = await get('/boom')
  assert.equal(res.status, 500)
  await res.text()
  assert.equal(globalThis.loads.boom, 1)
})

test('a redirect from load on a cached route runs once and keeps its location', async () => {
  const res = await get('/away')
  assert.equal(res.status, 303)
  assert.equal(res.headers.get('location'), '/elsewhere')
  assert.equal(globalThis.loads.away, 1)
})

test('a not-found from load on a cached route runs once and answers 404', async () => {
  const res = await get('/missing')
  assert.equal(res.status, 404)
  await res.text()
  assert.equal(globalThis.loads.missing, 1)
})

test('HEAD and compressed requests replay the same answer', async () => {
  const head = await fetch(base + '/boom?h=1', { method: 'HEAD', headers: { accept: 'text/html' } })
  assert.equal(head.status, 500)
  assert.equal(globalThis.loads.boom, 2)
  const gz = await get('/boom?z=1', { 'accept-encoding': 'gzip' })
  assert.equal(gz.status, 500)
  assert.match(await gz.text(), /<html|<!doctype/i)
  assert.equal(globalThis.loads.boom, 3)
})

test('a cacheable 200 is still rendered once and then served from the cache', async () => {
  for (let i = 0; i < 3; i++) assert.equal((await get('/fine')).status, 200)
  assert.equal(globalThis.loads.fine, 1)
})
