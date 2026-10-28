// server.mjs under a base path (§23 #148, manifest "base": "/app/"): routes, files and functions on the app path,
// /app → 301 /app/, outside the base → 404, /_health at the root, redirects get the base, chunks importing /app/_b/.
import { test, after } from 'node:test'
import assert from 'node:assert/strict'
import { copyFileSync, mkdirSync, mkdtempSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = fileURLToPath(new URL('.', import.meta.url))
const dist = mkdtempSync(join(tmpdir(), 'brain-base-'))
const w = (rel, s) => (mkdirSync(join(dist, rel, '..'), { recursive: true }), writeFileSync(join(dist, rel), s))
mkdirSync(join(dist, '_b'), { recursive: true })
copyFileSync(join(here, '../brain.js'), join(dist, '_b/brain.js'))
copyFileSync(join(here, '../brain-env.js'), join(dist, '_b/brain-env.js'))
mkdirSync(join(dist, '_server'), { recursive: true })
copyFileSync(join(here, '../shim/dom.js'), join(dist, '_server/dom.js'))
w('_b/hello.js', `import { tpl } from '/app/_b/brain.js'\nexport default function Hello() { return tpl('<h1>Hello from the app</h1>')() }\n`)
w('_b/post.js', `import { tpl, attr } from '/app/_b/brain.js'\nimport { route } from '/app/_b/brain-env.js'\nexport default function Post() { const r = route(), e = tpl('<p>' + [r.path, r.params.id, r.query.q].join(' ') + '<a>edit</a></p>')(); attr(e.lastChild, 'href', () => '/posts/' + r.params.id + '/edit'); return e }\n`)
w('_server/guard.mjs', `export default () => ({ redirect: '/login' })`)
w('_server/fns.mjs', `export async function ping(ctx, x) { return { pong: x, path: ctx.url.pathname } }`)
w('_server/pages/p.html', '<!doctype html><html><head><title>t</title></head><body><div id="brain-view"><!--brain:ssr--></div><script type="application/json" id="brain-props"><!--brain:props--></script></body></html>')
w('index.html', '<!doctype html><title>Home</title><main>home</main>')
w('index.md', '# Home')
w('rss.xml', '<rss/>')
w('atom.xml', '<feed/>')
w('site.webmanifest', '{}')
w('_b/app.js', 'console.log(1)')
w('_server/manifest.json', JSON.stringify({
  v: 1, build: 'b1', base: '/app/', redirects: { exact: { '/old': ['/hello', 301], '/away': ['https://elsewhere.test/x', 302] }, patterns: [{ re: '^/blog/([^/]+)/?$', to: '/posts/$1', status: 308 }] }, core: '/app/_b/brain.js', rpc: { f: '_server/fns.mjs' },
  routes: [
    { path: '/hello', rx: '^/hello/?$', view: '/app/_b/hello.js', layouts: [], server: null, page: '_server/pages/p.html' },
    { path: '/posts/:id', rx: '^/posts/(?<id>[^/]+)/?$', view: '/app/_b/post.js', layouts: [], server: null, page: '_server/pages/p.html' },
    { path: '/private', rx: '^/private/?$', view: '/app/_b/hello.js', layouts: [], server: null, page: '_server/pages/p.html', guards: ['_server/guard.mjs'] },
  ],
}))

const { createServer } = await import('../server/server.mjs')
const server = createServer({ dist, log() {} })
await new Promise(r => server.listen(0, '127.0.0.1', r))
const base = `http://127.0.0.1:${server.address().port}`
after(() => server.close())
const get = p => fetch(base + p, { redirect: 'manual' })

test('pages, files and SSR under /app/; /app → 301; outside the base → 404; /_health at the root', async () => {
  const r = await get('/app')
  assert.equal(r.status, 301)
  assert.equal(r.headers.get('location'), '/app/')
  assert.match(await (await get('/app/')).text(), /home/)
  assert.match(await (await get('/app/hello')).text(), /<h1>Hello from the app<\/h1>/) // the chunk imports /app/_b/brain.js
  const js = await get('/app/_b/app.js')
  assert.equal(js.status, 200)
  assert.match(js.headers.get('cache-control'), /immutable/)
  assert.equal((await get('/')).status, 404)
  assert.equal((await get('/hello')).status, 404)
  assert.deepEqual(await (await get('/_health')).json(), { ok: true, build: 'b1' })
})

test('RPC at /app/_r/ and app-path redirects get the base', async () => {
  const r = await fetch(base + '/app/_r/f/ping', { method: 'POST', headers: { 'x-brain-rpc': '1', origin: base, 'content-type': 'application/json' }, body: '[7]' })
  assert.equal(r.status, 200)
  assert.equal((await r.json()).pong, 7)
  const g = await get('/app/private')
  assert.equal(g.status, 303)
  assert.equal(g.headers.get('location'), '/app/login')
})

test('#169 route() during SSR: app path, params and query; the page carries <meta name="brain-params"> for the browser', async () => {
  const t = await (await get('/app/posts/7?q=x')).text()
  assert.match(t, /<p>\/posts\/7 7 x<a href="\/app\/posts\/7\/edit">edit<\/a><\/p>/) // a computed href gets the base in SSR
  assert.match(t, /<meta name="brain-params" content="\{&quot;id&quot;:&quot;7&quot;\}">/)
  assert.doesNotMatch(await (await get('/app/hello')).text(), /brain-params/) // no params, no meta
})

test('SEO: llms.txt page copies (.md), feeds and the web manifest get their own content types', async () => {
  assert.equal((await get('/app/index.md')).headers.get('content-type'), 'text/markdown; charset=utf-8')
  assert.equal((await get('/app/rss.xml')).headers.get('content-type'), 'application/rss+xml; charset=utf-8')
  assert.equal((await get('/app/atom.xml')).headers.get('content-type'), 'application/atom+xml; charset=utf-8')
  assert.equal((await get('/app/site.webmanifest')).headers.get('content-type'), 'application/manifest+json')
})

test('BUILD #219: manifest redirects (exact, then patterns) on the app path; same-site targets get the base, query kept', async () => {
  let r = await get('/app/old/?ref=x')
  assert.deepEqual([r.status, r.headers.get('location')], [301, '/app/hello?ref=x'])
  r = await get('/app/blog/first-post')
  assert.deepEqual([r.status, r.headers.get('location')], [308, '/app/posts/first-post'])
  r = await get('/app/away')
  assert.deepEqual([r.status, r.headers.get('location')], [302, 'https://elsewhere.test/x'])
  assert.equal((await get('/old')).status, 404) // outside the base: not the app's
})
