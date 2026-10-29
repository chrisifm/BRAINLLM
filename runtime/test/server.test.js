// runtime/server/server.mjs (INTERNALS §11.1, §22, §31) against a fake dist/: SSR with load and params, SEO head,
// static files, RPC defenses, signed session, security headers.
import { test, after } from 'node:test'
import assert from 'node:assert/strict'
import { copyFileSync, mkdirSync, mkdtempSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { gzipSync } from 'node:zlib'
import { fileURLToPath } from 'node:url'

const here = fileURLToPath(new URL('.', import.meta.url))
const dist = mkdtempSync(join(tmpdir(), 'brain-dist-'))
const w = (rel, s) => {
  mkdirSync(join(dist, rel, '..'), { recursive: true })
  writeFileSync(join(dist, rel), s)
}
mkdirSync(join(dist, '_b'), { recursive: true })
mkdirSync(join(dist, '_server', 'pages'), { recursive: true })
copyFileSync(join(here, '../brain.js'), join(dist, '_b/core.js'))
copyFileSync(join(here, '../shim/dom.js'), join(dist, '_server/dom.js'))
w('_b/view.js', `import { tpl, insert, attr } from '/_b/core.js'
const t = tpl('<main><h1><!></h1><p>id <!></p><select name="c"><option>All</option><option>Alpha</option></select><select name="d"><option>One</option><option>Two</option></select><input></main>')
export default function User($p) {
  const r = t(), h = r.firstChild, p = h.nextSibling, s = p.nextSibling, s2 = s.nextSibling
  insert(null, () => $p.name, h.firstChild)
  insert(null, () => $p.id, p.lastChild)
  // F9: <option value={o.value} selected={…}> and <select value={x}> as codegen emits them
  attr(s.firstChild, 'value', () => '')
  attr(s.lastChild, 'value', () => 'a')
  attr(s.lastChild, 'selected', () => true)
  attr(s2.firstChild, 'value', () => '1')
  attr(s2.lastChild, 'value', () => '2')
  attr(s2, 'value', () => '2')
  r.lastChild.value = $p.name
  return r
}
User.seo = p => ({ title: 'User ' + p.name, description: 'Profile of ' + p.name, canonical: '/users/' + p.id, 'og:title': 'User ' + p.name, jsonld: { '@type': 'Person', name: p.name + '</script>' } })
`)
w('_server/user.mjs', `export async function load(ctx) {
  const id = ctx.params.id
  if (id === 'slow') return new Promise(() => {})
  if (id === 'boom') throw new Error('db password is hunter2')
  if (id === 'private') return { redirect: '/login' }
  if (id === 'gone') throw { notFound: true, status: 404 } // what \`throw notFound()\` compiles to (F11)
  if (id === 'missing') return { notFound: true, status: 404 }
  return { id, name: 'Ada<b>', q: ctx.query.q || null, who: ctx.user && ctx.user.name }
}
`)
w('_server/docs.mjs', `export async function load(ctx) { return { id: 'doc', name: ctx.params.rest } }\nexport const seo = p => ({ title: 'Doc ' + p.name })`)
w('_server/rpc1.mjs', `export async function save(ctx, data) { return { saved: data.name, n: data.__proto__ === Object.prototype } }
export async function login(ctx, name) { ctx.session.set({ name }); return 'ok' }
export async function who(ctx) { return ctx.user }
export async function logout(ctx) { ctx.session.clear(); return 'bye' }
export async function envinfo(ctx) { let threw = false; try { ctx.env.BRAIN_SECRET = 'hijack' } catch { threw = true } return { frozen: Object.isFrozen(ctx.env), threw, app: ctx.env.APP_NAME, secret: ctx.env.BRAIN_SECRET } }
export async function slow() { return new Promise(() => {}) }
export async function boom() { throw new Error('secret stack detail') }
export async function invalid() { throw Object.assign(new Error('Invalid input'), { errors: { email: 'Already taken' } }) }
`)
w('_server/pages/user.html', '<!doctype html><html><head><title>Default</title><meta name="description" content="default"></head><body><div id="brain-view"><!--brain:ssr--><script type="module">mountView()</script></div><script type="application/json" id="brain-props"><!--brain:props--></script></body></html>')
w('_server/job.mjs', `export async function load(ctx) { return { id: ctx.params.slug, name: ctx.params.slug } }
export const seo = p => ({ title: 'Job ' + p.name, canonical: p.name === 'canon' ? 'https://x.test/c/1' : undefined, $head: '<meta property="og:type" content="article">' })`)
w('_server/pages/job.html', '<!doctype html><html><head><title>Job</title><link rel="canonical" href="https://x.test/jobs/:slug"><meta property="og:url" content="https://x.test/jobs/:slug"><meta property="og:type" content="website"></head><body><div id="brain-view"><!--brain:ssr--></div><script type="application/json" id="brain-props"><!--brain:props--></script></body></html>')
w('index.html', '<!doctype html><html><head><title>Home</title></head><body><main><h1>Home</h1></main><script type="module" src="/_b/app.js"></script></body></html>')
w('about.html', '<!doctype html><title>About</title><main><h1>About</h1></main>')
w('docs/index.html', '<!doctype html><title>Docs</title><main><h1>Docs</h1></main>')
w('404.html', '<!doctype html><title>404</title><main><h1>Lost</h1></main>')
w('_b/app.js', 'console.log("app")')
w('sw.js', 'self.BRAIN_BUILD={}')
writeFileSync(join(dist, '_b/app.js.gz'), gzipSync('console.log("app")'))
w('_server/manifest.json', JSON.stringify({
  v: 1, build: 'b7c1', core: '/_b/core.js',
  routes: [
    { path: '/users/:id', rx: '^/users/(?<id>(?![_.])[^/\\\\]+)/?$', view: '/_b/view.js', layouts: [], server: '_server/user.mjs', page: '_server/pages/user.html' },
    { path: '/jobs/:slug', rx: '^/jobs/(?<slug>(?![_.])[^/\\\\]+)/?$', view: '/_b/view.js', layouts: [], server: '_server/job.mjs', page: '_server/pages/job.html' },
    { path: '/docs/*rest', rx: '^/docs/(?<rest>(?![_.])[^/\\\\]+(?:/(?![_.])[^/\\\\]+)*)/?$', view: '/_b/view.js', layouts: [], server: '_server/docs.mjs', page: '_server/pages/user.html' },
  ],
  rpc: { abc: '_server/rpc1.mjs' },
  notFound: '404.html',
}))

const { createServer } = await import('../server/server.mjs')
const logs = []
const server = createServer({ dist, timeout: 150, rateLimit: 5, secret: 'test-secret-0123456789', env: { APP_NAME: 'shop', BRAIN_SECRET: 's' }, log: (level, msg, x) => logs.push({ level, msg, ...x }) }) // gitleaks:allow -- local test-only session secret
await new Promise(r => server.listen(0, '127.0.0.1', r))
const base = `http://127.0.0.1:${server.address().port}`
after(() => server.close())
const get = (p, h = {}) => fetch(base + p, { headers: h, redirect: 'manual' })
const call = (fn, args, h = {}) =>
  fetch(`${base}/_r/abc/${fn}`, { method: 'POST', headers: { 'content-type': 'application/json', 'x-brain-rpc': '1', ...h }, body: typeof args === 'string' ? args : JSON.stringify(args) })

test('SSR: params + query reach load, the view renders, props are embedded safely, SEO head replaced, nonce on scripts', async () => {
  const r = await get('/users/7?q=x')
  assert.equal(r.status, 200)
  const h = await r.text()
  assert.match(h, /<h1>Ada&lt;b&gt;(<!---->)?<\/h1>/) // escaped interpolation from the real runtime
  assert.match(h, /<p>id 7(<!---->)?<\/p>/)
  // F9: option values (including an empty one, serialized as a bare `value`) and the selected option are in the SSR HTML, so a GET form works before JS
  assert.match(h, /<select name="c"><option value(?:="")?>All<\/option><option value="a" selected>Alpha<\/option><\/select>/)
  assert.match(h, /<select name="d"><option value="1">One<\/option><option value="2" selected>Two<\/option><\/select>/)
  assert.match(h, /<input value="Ada&lt;b&gt;">|<input value="Ada<b>">/) // bind:value-like property copied to the attribute
  const props = JSON.parse(h.match(/id="brain-props">([^<]*)</)[1])
  assert.deepEqual(props, { id: '7', name: 'Ada<b>', q: 'x', who: null })
  assert.ok(!h.includes('"name":"Ada<b>"'), 'props JSON escapes <')
  assert.match(h, /<title>User Ada&lt;b&gt;<\/title>/)
  assert.match(h, /<meta name="description" content="Profile of Ada&lt;b&gt;">/)
  assert.ok(!h.includes('content="default"'))
  assert.match(h, /<link rel="canonical" href="\/users\/7">/)
  assert.match(h, /<meta property="og:title" content="User Ada&lt;b&gt;">/)
  assert.match(h, /application\/ld\+json">\{"@type":"Person","name":"Ada\\u003cb>\\u003c\/script>"\}/)
  const nonce = r.headers.get('content-security-policy').match(/'nonce-([^']+)'/)[1]
  assert.equal((h.match(new RegExp(`<script nonce="${nonce.replace(/[+/=]/g, '\\$&')}"`, 'g')) || []).length, 3)
  assert.equal(r.headers.get('x-content-type-options'), 'nosniff')
  assert.equal(r.headers.get('x-brain-build'), 'b7c1')
  assert.match(r.headers.get('content-security-policy'), /frame-ancestors 'none'/)
  assert.match(r.headers.get('content-security-policy'), /connect-src 'self' https: wss:/) // api() may call HTTPS APIs
  assert.ok(r.headers.get('strict-transport-security'))
  assert.ok(r.headers.get('x-request-id'))
})

test('SSR: load timeout → 504 page, load error → generic 500 (stack only in the log), redirect → 303', async () => {
  const slow = await get('/users/slow')
  assert.equal(slow.status, 504)
  const boom = await get('/users/boom')
  assert.equal(boom.status, 500)
  const t = await boom.text()
  assert.ok(!t.includes('hunter2'))
  assert.ok(logs.some(l => l.msg === 'load failed' && l.stack.includes('hunter2')))
  const red = await get('/users/private')
  assert.equal(red.status, 303)
  assert.equal(red.headers.get('location'), '/login')
  // F11: notFound() from load → a real 404 with the app's 404 page, thrown or returned (no soft 404)
  for (const id of ['gone', 'missing']) {
    const nf = await get('/users/' + id)
    assert.equal(nf.status, 404, id)
    assert.match(await nf.text(), /Lost/)
  }
})

test('static: clean URLs, index, gzip when accepted, immutable /_b/, 404 page, server code never served', async () => {
  assert.match(await (await get('/about')).text(), /About/)
  assert.match(await (await get('/docs/')).text(), /Docs/)
  const home = await get('/')
  assert.match(home.headers.get('content-security-policy'), /nonce-/)
  assert.match(await home.text(), /<script nonce="[^"]+" type="module" src="\/_b\/app.js">/)
  const gz = await fetch(base + '/_b/app.js', { headers: { 'accept-encoding': 'gzip' } })
  assert.equal(gz.headers.get('content-encoding'), 'gzip')
  assert.equal(await gz.text(), 'console.log("app")')
  assert.match(gz.headers.get('cache-control'), /immutable/)
  assert.equal(gz.headers.get('content-type'), 'text/javascript; charset=utf-8')
  const nf = await get('/nope')
  assert.equal(nf.status, 404)
  assert.match(await nf.text(), /Lost/)
  assert.equal((await get('/_server/rpc1.mjs')).status, 404)
  assert.equal((await get('/_server/manifest.json')).status, 404)
  assert.equal((await get('/../../etc/passwd')).status, 404)
  assert.equal((await get('/sw.js')).headers.get('cache-control'), 'no-cache')
  assert.deepEqual(await (await get('/_health')).json(), { ok: true, build: 'b7c1' })
  assert.equal((await get('/about')).headers.get('x-brain-build'), 'b7c1') // every HTML response carries it
})

test('RPC: returns JSON; strips __proto__; CSRF header and same origin required; 64 KB; bad JSON', async () => {
  const ok = await call('save', [{ name: 'Ada', __proto__: { polluted: true } }])
  assert.equal(ok.status, 200)
  assert.deepEqual(await ok.json(), { saved: 'Ada', n: true })
  const noHeader = await fetch(`${base}/_r/abc/save`, { method: 'POST', body: '[]' })
  assert.equal(noHeader.status, 403)
  assert.equal((await call('save', [{}], { origin: 'https://evil.test' })).status, 403)
  assert.equal((await call('save', [{}], { 'sec-fetch-site': 'cross-site' })).status, 403)
  assert.equal((await call('save', 'x'.repeat(70 * 1024))).status, 413)
  assert.equal((await call('save', '{nope')).status, 400)
  assert.equal((await call('save', '{"a":1}')).status, 400) // must be an array
  assert.equal((await call('missing', [])).status, 404)
  assert.equal((await fetch(`${base}/_r/zzz/save`, { method: 'POST', headers: { 'x-brain-rpc': '1' }, body: '[]' })).status, 404)
  assert.equal((await fetch(`${base}/_r/abc/save`, { headers: { 'x-brain-rpc': '1' } })).status, 405)
})

test('RPC: 422 with field errors, timeout → 504, unexpected error → generic 500 with request id, rate limit → 429', async () => {
  const inv = await call('invalid', [])
  assert.equal(inv.status, 422)
  assert.deepEqual(await inv.json(), { error: 'Invalid input', errors: { email: 'Already taken' } })
  assert.equal((await call('slow', [])).status, 504)
  const boom = await call('boom', [])
  assert.equal(boom.status, 500)
  const j = await boom.json()
  assert.equal(j.error, 'Internal error')
  assert.ok(j.rid && !JSON.stringify(j).includes('secret stack'))
  assert.ok(logs.some(l => l.rid === j.rid && l.stack.includes('secret stack detail')))
  const codes = []
  for (let i = 0; i < 7; i++) codes.push((await call('who', [], { 'x-forwarded-for': '9.9.9.9' })).status)
  assert.deepEqual(codes, [200, 200, 200, 200, 200, 429, 429])
  const other = await call('who', [], { 'x-forwarded-for': '8.8.8.8' })
  assert.equal(other.status, 200) // per IP
})

test('session: signed HttpOnly/Secure/SameSite cookie → ctx.user in RPC and load; tampering ignored; clear', async () => {
  const r = await call('login', ['Ada'], { 'x-forwarded-for': '1.1.1.1' })
  const c = r.headers.get('set-cookie')
  assert.match(c, /^__Host-session=[^;]+; Path=\/; HttpOnly; Secure; SameSite=Lax; Max-Age=604800$/)
  const cookie = c.split(';')[0]
  assert.deepEqual(await (await call('who', [], { cookie, 'x-forwarded-for': '1.1.1.2' })).json(), { name: 'Ada' })
  const page = await (await get('/users/1', { cookie })).text()
  assert.match(page, /"who":"Ada"/)
  const [k, v] = cookie.split('=')
  const [data, mac] = v.split('.')
  const forged = Buffer.from(JSON.stringify({ u: { name: 'Admin' }, exp: Date.now() + 1e6 })).toString('base64url')
  assert.equal(await (await call('who', [], { cookie: `${k}=${forged}.${mac}`, 'x-forwarded-for': '1.1.1.3' })).json(), null)
  assert.equal(await (await call('who', [], { cookie: `${k}=${data}.${mac}x`, 'x-forwarded-for': '1.1.1.4' })).json(), null)
  const out = await call('logout', [], { cookie, 'x-forwarded-for': '1.1.1.5' })
  assert.match(out.headers.get('set-cookie'), /^__Host-session=; .*Max-Age=0$/)
})

test('ETag: SSR HTML weak, static HTML strong (stable across nonces) → 304 with no body; logout sends Clear-Site-Data', async () => {
  const a = await get('/users/7')
  const et = a.headers.get('etag')
  assert.match(et, /^W\/"[\w-]{22}"$/)
  await a.text()
  const b = await get('/users/7', { 'if-none-match': et })
  assert.equal(b.status, 304)
  assert.equal(await b.text(), '')
  assert.equal((await get('/users/8', { 'if-none-match': et })).status, 200) // other content, other tag
  const h = await get('/about')
  const st = h.headers.get('etag')
  assert.equal(st, '"b7c1"') // the build signature
  assert.equal(h.headers.get('cache-control'), 'no-cache')
  assert.equal((await get('/about', { 'if-none-match': `"x", ${st}` })).status, 304)
  const r = await call('login', ['Ada'], { 'x-forwarded-for': '2.2.2.1' })
  assert.equal(r.headers.get('clear-site-data'), null)
  const out = await call('logout', [], { 'x-forwarded-for': '2.2.2.2' })
  assert.equal(out.headers.get('clear-site-data'), '"storage"') // F16: "cache" made Chrome hold the logout response ~18 s
})

test('ctx.env: only what the supervisor passed, frozen (a write throws and changes nothing)', async () => {
  const r = await (await call('envinfo', [], { 'x-forwarded-for': '3.3.3.3' })).json()
  assert.deepEqual(r, { frozen: true, threw: true, app: 'shop', secret: 's' })
})

test("BUILD #177: the manifest's CSP is used as is, with this response's nonce added to script-src", async () => {
  const { createServer: cs } = await import('../server/server.mjs?csp')
  const fs = await import('node:fs')
  const d2 = fs.mkdtempSync(join(tmpdir(), 'brain-csp-'))
  fs.mkdirSync(join(d2, '_server'))
  fs.writeFileSync(join(d2, 'index.html'), '<!doctype html><title>x</title><script>1</script>')
  fs.writeFileSync(join(d2, '_server/manifest.json'), JSON.stringify({ v: 1, routes: [], rpc: {}, csp: "default-src 'self'; script-src 'self' https://www.googletagmanager.com; connect-src 'self' https: wss:" }))
  const s2 = cs({ dist: d2, log() {} })
  await new Promise(r => s2.listen(0, '127.0.0.1', r))
  const res = await fetch(`http://127.0.0.1:${s2.address().port}/`)
  const c = res.headers.get('content-security-policy')
  assert.match(c, /^default-src 'self'; script-src 'self' https:\/\/www.googletagmanager.com 'nonce-[^']+'; connect-src 'self' https: wss:$/)
  s2.close()
})

test('#193 routes come from the manifest (rx, order): :param one segment, *rest catch-all, _segments never match', async () => {
  assert.match(await (await get('/users/7/')).text(), /<p>id 7/)
  assert.equal((await get('/users/_x')).status, 404)
  assert.equal((await get('/users/a/b')).status, 404)
  const doc = await (await get('/docs/a/b/c')).text()
  assert.match(doc, /<h1>a\/b\/c/)
  assert.match(doc, /<title>Doc a\/b\/c<\/title>/) // seo from the server module wins over the client chunk's V.seo
  assert.match(await (await get('/docs')).text(), /<h1>Docs<\/h1>/) // *rest needs ≥1 segment: /docs is the static docs/index.html
})

test('F5: an SSR route answers with its real URL in canonical and og:url, and a view <head> tag replaces the generated one', async () => {
  const h = await (await get('/jobs/backend-dev')).text()
  assert.match(h, /<link rel="canonical" href="https:\/\/x\.test\/jobs\/backend-dev">/)
  assert.match(h, /<meta property="og:url" content="https:\/\/x\.test\/jobs\/backend-dev">/)
  assert.ok(!h.includes(':slug'), 'no route pattern left in the head')
  assert.equal((h.match(/property="og:type"/g) || []).length, 1, 'one og:type: the view head wins')
  assert.match(h, /<meta property="og:type" content="article">/)
  // a canonical from seo(props) also becomes og:url
  const c = await (await get('/jobs/canon')).text()
  assert.match(c, /<link rel="canonical" href="https:\/\/x\.test\/c\/1">/)
  assert.match(c, /<meta property="og:url" content="https:\/\/x\.test\/c\/1">/)
})
