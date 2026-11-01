// 0.8.3: what a production answer leaves behind and how many URLs one page has: every 5xx logs its cause and rid, the
// page cache ignores tracking parameters, `query="page"` and URL variants answer one canonical URL, Markdown twins name it.
import { test, after } from 'node:test'
import assert from 'node:assert/strict'
import { createServer as http } from 'node:http'
import { copyFileSync, mkdirSync, mkdtempSync, writeFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = fileURLToPath(new URL('.', import.meta.url))
const dist = mkdtempSync(join(tmpdir(), 'brain-serving-'))
const w = (rel, s) => (mkdirSync(join(dist, rel, '..'), { recursive: true }), writeFileSync(join(dist, rel), s))
mkdirSync(join(dist, '_b'), { recursive: true })
copyFileSync(join(here, '../brain.js'), join(dist, '_b/brain.js'))
mkdirSync(join(dist, '_server'), { recursive: true })
copyFileSync(join(here, '../shim/dom.js'), join(dist, '_server/dom.js'))
w('_b/v.js', `import { tpl } from '/_b/brain.js'\nexport default function V(p) { return tpl('<p>call ' + p.n + '</p>')() }\n`)
const page = (head = '') => `<!doctype html><html><head><title>t</title>${head}</head><body><div id="brain-view"><!--brain:ssr--></div><script type="application/json" id="brain-props"><!--brain:props--></script></body></html>`
w('_server/pages/p.html', page())
w('_server/pages/list.html', page('<meta name="brain-query" content="page">'))
w('_server/n.server.mjs', `let n = 0\nexport function load() { return { n: ++n } }\n`)
w('_server/flaky.server.mjs', `let n = 0\nexport function load(ctx) { if (globalThis.flakyFails) throw new Error('private-load-value'); return { n: ++n } }\n`)
w('_server/soft.server.mjs', `export async function load(ctx) { try { await ctx.backend.resource('things').list() } catch { return { n: 'fallback' } } return { n: 'real' } }\n`)
w('500.html', page())
w('about/index.html', page('<link rel="canonical" href="https://site.example/about">'))
w('about.md', '# About')
w('docs.md', '# Docs without a canonical')
w('docs/index.html', page())
const route = (path, server, extra = {}) => ({ path, rx: `^${path}/?$`, view: '/_b/v.js', layouts: [], server, page: '_server/pages/p.html', ...extra })
w('_server/manifest.json', JSON.stringify({
  v: 1, build: 'b1', core: '/_b/brain.js', rpc: {},
  routes: [
    route('/cached', '_server/n.server.mjs', { cache: 60 }),
    route('/flaky', '_server/flaky.server.mjs', { cache: 1 }),
    route('/soft', '_server/soft.server.mjs'),
    route('/softcached', '_server/soft.server.mjs', { cache: 60 }),
    route('/list', '_server/n.server.mjs', { page: '_server/pages/list.html' }),
    route('/plain', '_server/n.server.mjs'),
  ],
}))

let backendStatus = 500, backendVary = null, hits = 0
const backend = http((req, res) => {
  hits++
  const headers = { 'content-type': 'application/json' }
  if (backendVary) headers.vary = backendVary
  res.writeHead(backendStatus, headers)
  res.end('{"error":{"code":"DATABASE_BUSY"}}')
})
await new Promise(r => backend.listen(0, '127.0.0.1', r))
const logs = []
const { createServer } = await import('../server/server.mjs')
const make = o => createServer({ dist, secret: 'test-only-session-key', log: (level, msg, extra) => logs.push({ level, msg, ...extra }), ...o })
const server = make({ backendURL: `http://127.0.0.1:${backend.address().port}` })
await new Promise(r => server.listen(0, '127.0.0.1', r))
const base = `http://127.0.0.1:${server.address().port}`
// no native backend listening: the bridge has to fail on the transport
const dead = http(() => {})
await new Promise(r => dead.listen(0, '127.0.0.1', r))
const deadPort = dead.address().port
await new Promise(r => dead.close(r))
const orphan = make({ backendURL: `http://127.0.0.1:${deadPort}` })
await new Promise(r => orphan.listen(0, '127.0.0.1', r))
const orphanBase = `http://127.0.0.1:${orphan.address().port}`
after(async () => {
  await Promise.all([server, orphan, backend].map(s => new Promise(r => { s.closeAllConnections?.(); s.close(r) })))
  rmSync(dist, { recursive: true, force: true })
})
const get = (p, h = {}, b = base) => fetch(b + p, { headers: h, redirect: 'manual' })
const settled = () => new Promise(r => setTimeout(r, 20))
const lineFor = async res => {
  await res.arrayBuffer()
  await settled()
  const rid = res.headers.get('x-request-id')
  return logs.filter(l => l.rid === rid && l.level !== 'info')
}

test('a 5xx relayed from the native backend leaves one line with the rid and says where it came from', async () => {
  backendStatus = 500
  const res = await get('/api/things')
  assert.equal(res.status, 500)
  const lines = await lineFor(res)
  assert.equal(lines.length, 1, JSON.stringify(logs))
  assert.equal(lines[0].status, 500)
  assert.match(lines[0].cause, /backend/)
})

test('a backend that cannot be reached logs the transport cause, never a request value', async () => {
  const res = await get('/api/things?token=private-query-value', {}, orphanBase)
  assert.ok(res.status >= 500)
  const lines = await lineFor(res)
  assert.ok(lines.length >= 1)
  const all = JSON.stringify(lines)
  assert.match(all, /ECONNREFUSED/)
  assert.match(all, /Backend request unavailable/)
  assert.doesNotMatch(all, /private-query-value/)
})

test('a failed health probe through the backend is not silent', async () => {
  const res = await get('/health/ready', {}, orphanBase)
  assert.equal(res.status, 503)
  assert.ok((await lineFor(res)).length >= 1)
})

test('a status page opened directly says so instead of looking like a silent failure', async () => {
  const res = await get('/500')
  assert.equal(res.status, 500)
  await res.arrayBuffer()
  await settled()
  const line = logs.find(l => l.rid === res.headers.get('x-request-id'))
  assert.equal(line?.level, 'info')
  assert.match(line.cause, /status page/)
})

test('an application message in an error stays out of the log', async () => {
  globalThis.flakyFails = true
  const res = await get('/flaky?a=fail')
  assert.equal(res.status, 500)
  const lines = await lineFor(res)
  globalThis.flakyFails = false
  assert.ok(lines.length >= 1)
  assert.doesNotMatch(JSON.stringify(lines), /private-load-value/)
})

test('the page cache ignores tracking parameters and the order of the others', async () => {
  const call = async p => (await (await get(p)).text()).match(/call (\d+)/)[1]
  const first = await call('/cached?b=2&a=1')
  assert.equal(await call('/cached?a=1&b=2&utm_source=mail'), first)
  assert.equal(await call('/cached?gclid=x&b=2&fbclid=y&a=1&msclkid=z'), first)
  assert.notEqual(await call('/cached?a=1&b=3'), first)
})

test('a page cache answer allows stale-if-error and a failed revalidation keeps the copy', async () => {
  const fresh = await get('/flaky?keep=1')
  assert.equal(fresh.headers.get('cache-control'), 'public, s-maxage=1, stale-while-revalidate=1, stale-if-error=1')
  const copy = (await fresh.text()).match(/call (\d+)/)[1]
  await new Promise(r => setTimeout(r, 1100))
  globalThis.flakyFails = true
  await (await get('/flaky?keep=1')).text() // serves the copy, revalidates in the background, the load fails
  await settled()
  const still = await get('/flaky?keep=1')
  globalThis.flakyFails = false
  assert.equal(still.status, 200)
  assert.equal((await still.text()).match(/call (\d+)/)[1], copy)
})

test('`query="page"`: page 1 is the base URL, anything that is not a page number does not exist', async () => {
  const one = await get('/list?page=1&sort=new')
  assert.equal(one.status, 301)
  assert.equal(one.headers.get('location'), '/list?sort=new')
  assert.equal((await get('/list?page=1')).headers.get('location'), '/list')
  assert.equal((await get('/list?page=2')).status, 200)
  assert.equal((await get('/list')).status, 200)
  for (const bad of ['abc', '0', '-1', '1.5', '', '1e3', '02', '99999999999'])
    assert.equal((await get('/list?page=' + bad)).status, 404, bad)
  assert.equal((await get('/list?page=2&page=3')).status, 404)
  // a view that does not list `page` keeps its old behavior
  assert.equal((await get('/plain?page=abc')).status, 200)
})

test('URL variants collapse in one 301 on the same origin', async () => {
  const to = async p => { const r = await get(p); return [r.status, r.headers.get('location')] }
  assert.deepEqual(await to('//plain'), [301, '/plain'])
  assert.deepEqual(await to('/plain//'), [301, '/plain'])
  assert.deepEqual(await to('/about/index.html?x=1'), [301, '/about?x=1'])
  assert.deepEqual(await to('/index.html'), [301, '/'])
  assert.deepEqual(await to('//evil.example/x'), [301, '/evil.example/x'])
  assert.equal((await get('/%2f/x')).status, 404)
  const post = await fetch(base + '//plain', { method: 'POST', redirect: 'manual' })
  assert.equal(post.status, 404)
})

test('a Markdown twin names the canonical the page declares, and only that', async () => {
  const twin = await get('/about.md')
  assert.equal(twin.headers.get('link'), '<https://site.example/about>; rel="canonical"')
  assert.equal(twin.headers.get('x-robots-tag'), null)
  const negotiated = await get('/about', { accept: 'text/markdown' })
  assert.equal(negotiated.headers.get('link'), '<https://site.example/about>; rel="canonical"')
  assert.equal((await get('/docs.md')).headers.get('link'), null)
})

test('no response repeats a Vary token', async () => {
  backendStatus = 200
  backendVary = 'Accept, accept, Origin'
  const res = await get('/_brain/files/x/y')
  backendVary = null
  assert.equal(res.headers.get('vary'), 'Accept, Origin')
})

test('a page that survived a failed backend read answers 200 but says so in the log and is not cached', async () => {
  backendStatus = 500
  const res = await get('/soft')
  assert.equal(res.status, 200)
  assert.match(await res.text(), /call fallback/)
  await settled()
  const line = logs.find(l => l.rid === res.headers.get('x-request-id'))
  assert.equal(line?.level, 'warn')
  assert.match(line.msg, /failed backend read/)
  assert.equal(line.backendCode, 'DATABASE_BUSY')
  const kept = await get('/softcached')
  assert.equal(kept.headers.get('cache-control'), 'no-store')
  await kept.arrayBuffer()
  backendStatus = 200
  const healthy = await get('/plain')
  await healthy.arrayBuffer()
  await settled()
  assert.equal(logs.filter(l => l.rid === healthy.headers.get('x-request-id')).length, 0)
})

test('a cached page whose load reads the native backend is stored like any other', async () => {
  backendStatus = 200
  hits = 0
  const first = await get('/softcached?stored=1')
  assert.match(await first.text(), /call real/)
  assert.match(first.headers.get('cache-control'), /s-maxage=60/)
  const again = await get('/softcached?stored=1')
  await again.arrayBuffer()
  assert.equal(hits, 1)
})
