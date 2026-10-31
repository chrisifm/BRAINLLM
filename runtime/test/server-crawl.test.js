// Crawler-safe answers from runtime/server/server.mjs (INTERNALS §33 "Redirects and status codes"): gone() → 410,
// one-hop trailing-slash and canonical-host redirects with the query kept, guarded pages kept out of the index, and
// streamed routes that still answer real status codes when load() is quick.
import { test, after } from 'node:test'
import assert from 'node:assert/strict'
import { copyFileSync, mkdirSync, mkdtempSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { request } from 'node:http'

const here = fileURLToPath(new URL('.', import.meta.url))
const { createServer } = await import('../server/server.mjs')

function makeDist(redirects) {
  const dist = mkdtempSync(join(tmpdir(), 'brain-crawl-'))
  const w = (rel, s) => {
    mkdirSync(join(dist, rel, '..'), { recursive: true })
    writeFileSync(join(dist, rel), s)
  }
  mkdirSync(join(dist, '_b'), { recursive: true })
  copyFileSync(join(here, '../brain.js'), join(dist, '_b/core.js'))
  mkdirSync(join(dist, '_server'), { recursive: true })
  copyFileSync(join(here, '../shim/dom.js'), join(dist, '_server/dom.js'))
  w('_b/item.js', `import { tpl, insert } from './core.js'
const t = tpl('<main><h1><!></h1></main>')
export default function Item($p) { const r = t(); insert(null, () => $p.name, r.firstChild.firstChild); return r }
`)
  w('_server/item.mjs', `export async function load(ctx) {
  const id = ctx.params.id
  if (id === 'retired') throw { notFound: true, gone: true, status: 410 } // what \`throw gone()\` compiles to
  if (id === 'deleted') return { notFound: true, gone: true, status: 410 }
  if (id === 'missing') return { notFound: true, status: 404 }
  if (id === 'renamed') return { redirect: '/items/new-name', status: 301 }
  return { id, name: 'Item ' + id }
}
`)
  w('_server/guard.mjs', `export default ctx => ctx.user ? true : { redirect: '/login' }`)
  w('_server/deny.mjs', `export default () => ({ deny: 'Members only' })`)
  w('_server/pages/item.html', '<!doctype html><html><head><title>Item</title></head><body><div id="brain-view"><!--brain:ssr--></div><script type="application/json" id="brain-props"><!--brain:props--></script></body></html>')
  w('index.html', '<!doctype html><title>Home</title><main><h1>Home</h1></main>')
  w('about.html', '<!doctype html><title>About</title><main><h1>About</h1></main>')
  w('docs/index.html', '<!doctype html><title>Docs</title><main><h1>Docs</h1></main>')
  w('404.html', '<!doctype html><title>404</title><main><h1>Lost</h1></main>')
  w('_server/manifest.json', JSON.stringify({
    v: 1, build: 'c1', core: '/_b/core.js',
    routes: [
      { path: '/items/:id', rx: '^/items/(?<id>(?![_.])[^/\\\\]+)/?$', view: '/_b/item.js', layouts: [], server: '_server/item.mjs', page: '_server/pages/item.html' },
      { path: '/streamed/:id', rx: '^/streamed/(?<id>(?![_.])[^/\\\\]+)/?$', view: '/_b/item.js', layouts: [], server: '_server/item.mjs', page: '_server/pages/item.html', stream: true },
      { path: '/account', rx: '^/account/?$', view: '/_b/item.js', layouts: [], server: null, page: '_server/pages/item.html', guards: ['_server/guard.mjs'] },
      { path: '/members', rx: '^/members/?$', view: '/_b/item.js', layouts: [], server: null, page: '_server/pages/item.html', guards: ['_server/deny.mjs'] },
    ],
    rpc: {},
    notFound: '404.html',
    redirects,
  }))
  return dist
}

async function start(redirects) {
  const server = createServer({ dist: makeDist(redirects), timeout: 1000, secret: 'test-secret-0123456789', env: {}, log: () => {} }) // gitleaks:allow -- local test-only session secret
  await new Promise(r => server.listen(0, '127.0.0.1', r))
  after(() => server.close())
  const port = server.address().port, base = `http://127.0.0.1:${port}`
  // fetch() never sends a custom Host header, so the canonical-host case goes through node:http
  const withHost = (p, host) => new Promise((ok, no) => request({ port, path: p, headers: { host, accept: 'text/html' } }, r => { r.resume(); ok({ status: r.statusCode, headers: { get: k => r.headers[k] ?? null } }) }).on('error', no).end())
  return (p, h = {}) => (h.host ? withHost(p, h.host) : fetch(base + p, { headers: { accept: 'text/html', ...h }, redirect: 'manual' }))
}

const get = await start({ exact: { '/old-items': ['/items/', 301] }, patterns: [] })

test('gone() from load answers 410 (thrown or returned), notFound() stays 404, a moved record is a 301', async () => {
  for (const id of ['retired', 'deleted']) {
    const r = await get('/items/' + id)
    assert.equal(r.status, 410, id)
    assert.match(await r.text(), /Lost/, 'without a 410 page the 404 page is shown')
  }
  assert.equal((await get('/items/missing')).status, 404)
  const moved = await get('/items/renamed')
  assert.equal(moved.status, 301)
  assert.equal(moved.headers.get('location'), '/items/new-name')
})

test('trailing slash: one 301 to the slash-less URL with the query kept; the root and assets are untouched', async () => {
  const r = await get('/about/?ref=mail&x=1')
  assert.equal(r.status, 301)
  assert.equal(r.headers.get('location'), '/about?ref=mail&x=1')
  assert.equal((await get('/about')).status, 200)
  assert.equal((await get('/docs')).status, 200, 'a folder index is served without the slash')
  assert.equal((await get('/docs/')).headers.get('location'), '/docs')
  assert.equal((await get('/items/7/')).headers.get('location'), '/items/7')
  assert.equal((await get('/')).status, 200)
})

test('a redirect rule and the slash policy are one hop, never a chain', async () => {
  const r = await get('/old-items/?page=2')
  assert.equal(r.status, 301)
  assert.equal(r.headers.get('location'), '/items?page=2', 'the rule target is normalized by the same policy')
})

test('guarded pages stay out of the index: the login redirect and the 403 page carry X-Robots-Tag noindex', async () => {
  const r = await get('/account')
  assert.equal(r.status, 303)
  assert.equal(r.headers.get('location'), '/login')
  assert.equal(r.headers.get('x-robots-tag'), 'noindex, nofollow')
  const d = await get('/members')
  assert.equal(d.status, 403)
  assert.equal(d.headers.get('x-robots-tag'), 'noindex, nofollow')
})

test('streamed route: a quick load() still answers 410 / 404 / 301 instead of a soft 200', async () => {
  assert.equal((await get('/streamed/retired')).status, 410)
  assert.equal((await get('/streamed/missing')).status, 404)
  assert.equal((await get('/streamed/renamed')).status, 301)
  const ok = await get('/streamed/9')
  assert.equal(ok.status, 200)
  assert.match(await ok.text(), /Item 9/)
})

test('trailingSlash "always" and a canonical host: one hop to https host + slash; loopback hosts are never redirected', async () => {
  const g = await start({ exact: {}, patterns: [], slash: 'always', host: 'https://shop.example' })
  const r = await g('/about?x=1', { host: 'www.shop.example' })
  assert.equal(r.status, 301)
  assert.equal(r.headers.get('location'), 'https://shop.example/about/?x=1')
  assert.equal((await g('/about/')).status, 200, 'loopback Host: served, no host redirect')
  assert.equal((await g('/about')).headers.get('location'), '/about/')
  const ig = await start({ exact: {}, patterns: [], slash: 'ignore' })
  assert.equal((await ig('/about/')).status, 200)
  assert.equal((await ig('/about')).status, 200)
})
