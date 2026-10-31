// #232 server.mjs: 103 Early Hints with the route's preloads, and streaming SSR (head + layout shell before load()
// ends; a late redirect or failure handled inline).
import { test, after } from 'node:test'
import assert from 'node:assert/strict'
import http from 'node:http'
import { copyFileSync, mkdirSync, mkdtempSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = fileURLToPath(new URL('.', import.meta.url))
const dist = mkdtempSync(join(tmpdir(), 'brain-stream-'))
const w = (rel, s) => (mkdirSync(join(dist, rel, '..'), { recursive: true }), writeFileSync(join(dist, rel), s))
mkdirSync(join(dist, '_b'), { recursive: true })
copyFileSync(join(here, '../brain.js'), join(dist, '_b/brain.js'))
mkdirSync(join(dist, '_server'), { recursive: true })
copyFileSync(join(here, '../shim/dom.js'), join(dist, '_server/dom.js'))
w('_b/v.js', `import { tpl } from '/_b/brain.js'\nexport default function V(p) { return tpl('<h1>' + p.msg + '</h1>')() }\n`)
w('_server/v.server.mjs', `export const seo = () => ({ title: 'Streamed' })\nexport async function load(ctx) {\n  await new Promise(r => (globalThis.__release = r))\n  if (ctx.query.go) return { redirect: '/elsewhere' }\n  if (ctx.query.fail) throw new Error('db down')\n  return { msg: 'loaded' }\n}\n`)
// load() that settles at once: the held head lets it answer its real status (F7b)
w('_server/q.server.mjs', `export const seo = () => ({ title: 'Quick' })\nexport async function load(ctx) {\n  if (ctx.query.nf) throw { notFound: true, status: 404 }\n  if (ctx.query.nfr) return { notFound: true, status: 404 }\n  if (ctx.query.go) throw { redirect: '/elsewhere', status: 303 }\n  if (ctx.query.deny) throw { status: 403 }\n  if (ctx.query.fail) throw new Error('db down')\n  return { msg: 'quick' }\n}\n`)
w('_b/ld.js', `import { tpl } from '/_b/brain.js'\nexport default function L() { return tpl('<p aria-label="Loading" class="ld">Loading…</p>')() }\n`)
w('_b/err.js', `import { tpl } from '/_b/brain.js'\nexport default function E(p) { return tpl('<h2>Error view ' + p.rid + '</h2>')() }\n`)
w('_server/pages/p.html', '<!doctype html><html><head><link rel="stylesheet" href="/_b/app.css"><title>t</title></head><body><nav>shell</nav><div id="brain-view"><!--brain:ssr--></div><script type="application/json" id="brain-props"><!--brain:props--></script></body></html>')
// pages with the client mount script (brain-pack): the view's page and the _error.brain template
w('_server/pages/m.html', '<!doctype html><html><head><title>t</title></head><body><nav>shell</nav><div id="brain-view"><!--brain:ssr--></div><script type="application/json" id="brain-props"><!--brain:props--></script><script type="module">const e=document.getElementById(\'brain-view\');mount(View)</script></body></html>')
w('_server/pages/err.html', '<!doctype html><html><head><title>t</title></head><body><nav>shell</nav><div id="brain-view"><!--brain:ssr--></div><script type="application/json" id="brain-props"><!--brain:props--></script><script type="module">const e=document.getElementById(\'brain-view\');mount(ErrorView)</script></body></html>')
w('_server/manifest.json', JSON.stringify({
  v: 1, build: 'b1', core: '/_b/brain.js', rpc: {},
  routes: [{ path: '/s', rx: '^/s/?$', view: '/_b/v.js', layouts: [], server: '_server/v.server.mjs', page: '_server/pages/p.html', stream: true, links: ['</_b/app.css>; rel=preload; as=style'] },
    { path: '/e', rx: '^/e/?$', view: '/_b/v.js', layouts: [], server: '_server/v.server.mjs', page: '_server/pages/p.html', stream: true, error: { view: '/_b/err.js', layouts: [], page: '_server/pages/p.html' } },
    { path: '/me', rx: '^/me/?$', view: '/_b/v.js', layouts: [], server: '_server/v.server.mjs', page: '_server/pages/m.html', stream: true, error: { view: '/_b/err.js', layouts: [], page: '_server/pages/err.html' } },
    { path: '/l', rx: '^/l/?$', view: '/_b/v.js', layouts: [], server: '_server/v.server.mjs', page: '_server/pages/p.html', stream: true, loading: '/_b/ld.js' },
    { path: '/mn', rx: '^/mn/?$', view: '/_b/v.js', layouts: [], server: '_server/v.server.mjs', page: '_server/pages/m.html', stream: true },
    { path: '/q', rx: '^/q/?$', view: '/_b/v.js', layouts: [], server: '_server/q.server.mjs', page: '_server/pages/p.html', stream: true }],
}))

const { createServer } = await import('../server/server.mjs')
const logs = []
const server = createServer({ dist, log: (lvl, msg, f) => logs.push(msg) })
await new Promise(r => server.listen(0, '127.0.0.1', r))
const port = server.address().port
after(() => server.close())

// raw request: collects 103 hints and the body chunks as they arrive
const req = path =>
  new Promise((ok, no) => {
    const hints = [], chunks = []
    const r = http.get({ host: '127.0.0.1', port, path }, res => {
      res.setEncoding('utf8')
      res.on('data', c => chunks.push(c))
      res.on('end', () => ok({ status: res.statusCode, hints, body: chunks.join('') }))
    })
    r.on('information', i => i.statusCode === 103 && hints.push(i.headers.link))
    r.on('error', no)
    ok.chunks = chunks
    req.last = chunks
  })
const until = async f => {
  for (let i = 0; i < 200 && !f(); i++) await new Promise(r => setTimeout(r, 5))
  return f()
}
// the head went out (the hold for load() expired): what load() does next is handled inline
const late = () => until(() => globalThis.__release && req.last && req.last.join('').includes('<nav>shell</nav>'))

test('F7b: a load() that settles while the head is held answers its real status, like a non-streamed route', async () => {
  const go = await req('/q?go=1')
  assert.equal(go.status, 303)
  assert.equal((await req('/q?nf=1')).status, 404)
  assert.equal((await req('/q?nfr=1')).status, 404)
  assert.equal((await req('/q?deny=1')).status, 403)
  const fail = await req('/q?fail=1')
  assert.equal(fail.status, 500)
  assert.doesNotMatch(fail.body, /db down/)
  const ok = await req('/q')
  assert.equal(ok.status, 200)
  assert.match(ok.body, /<title>Quick<\/title>[\s\S]*<h1>quick<\/h1>/)
})

test('103 Early Hints carry the preloads; the head and layout shell arrive before load() ends', async () => {
  const p = req('/s')
  assert.ok(await until(() => req.last && req.last.join('').includes('<nav>shell</nav>')), 'shell sent before load()')
  const early = req.last.join('')
  assert.match(early, /<title>Streamed<\/title>/)
  assert.doesNotMatch(early, /loaded/)
  globalThis.__release()
  const r = await p
  assert.equal(r.status, 200)
  assert.deepEqual(r.hints, ['</_b/app.css>; rel=preload; as=style'])
  assert.match(r.body, /<h1>loaded<\/h1>/)
  assert.match(r.body, /"msg":"loaded"/) // props for the client
  assert.match(r.body, /<\/html>$/)
})

test('a redirect or a failure after the head went out: location.replace / inline notice with the request id', async () => {
  globalThis.__release = null
  let p = req('/s?go=1')
  await late()
  globalThis.__release()
  let r = await p
  assert.match(r.body, /<script nonce="[^"]+">location\.replace\("\/elsewhere"\)<\/script>/)
  globalThis.__release = null
  p = req('/s?fail=1')
  await late()
  globalThis.__release()
  r = await p
  assert.match(r.body, /<p role="alert" class="brain-fallback">Something went wrong\. Reference: [\w-]+<\/p>/)
  assert.ok(logs.includes('load failed (streamed)'))
})

test("a streamed route with an _error.brain renders it inside the region when load() fails", async () => {
  globalThis.__release = null
  const p = req('/e?fail=1')
  await late()
  globalThis.__release()
  const r = await p
  assert.match(r.body, /<h2>Error view [\w-]+<\/h2>/)
  assert.doesNotMatch(r.body, /class="brain-fallback"/)
})

test('QA #289: when load() fails on a streamed route the client mounts _error.brain, never the view over the error', async () => {
  globalThis.__release = null
  let p = req('/me?fail=1')
  await late()
  globalThis.__release()
  let r = await p
  const rid = r.body.match(/<h2>Error view ([\w-]+)<\/h2>/)[1]
  assert.match(r.body, /mount\(ErrorView\)/)
  assert.doesNotMatch(r.body, /mount\(View\)/)
  assert.equal(JSON.parse(r.body.match(/id="brain-props">([^<]*)</)[1]).rid, rid) // the error view hydrates with { message, rid }
  assert.match(r.body, /<nav>shell<\/nav>/) // the layout stays
  globalThis.__release = null
  p = req('/mn?fail=1') // no _error.brain: the notice stays and the view is not mounted over it
  await late()
  globalThis.__release()
  r = await p
  assert.match(r.body, /class="brain-fallback">Something went wrong\. Reference: [\w-]+</)
  assert.doesNotMatch(r.body, /mount\(View\)/)
  assert.match(r.body, /<\/html>$/)
})

test('QA #253: the first chunk shows _loading.brain in the region while load() runs; the last chunk removes it', async () => {
  globalThis.__release = null
  const p = req('/l')
  assert.ok(await until(() => req.last && /aria-label="Loading"/.test(req.last.join(''))), 'loading view sent before load() ends')
  const early = req.last.join('')
  assert.match(early, /<div id="brain-view"><!--brain:loading--><p aria-label="Loading" class="ld">Loading…<\/p><!--\/brain:loading-->$/)
  await late()
  globalThis.__release()
  const r = await p
  const region = r.body.match(/<div id="brain-view">([\s\S]*)<\/div><script[^>]* type="application\/json"/)[1]
  assert.match(region, /<!--\/brain:loading--><script nonce="[^"]+">[^<]+<\/script><h1>loaded<\/h1>$/)
  // what the browser does when that script runs: only the view is left in the region
  const { install } = await import('../shim/dom.js')
  const d = install()
  const box = d.createElement('div')
  box.innerHTML = region
  const sc = box.querySelector('script')
  new Function('document', sc.textContent)({ currentScript: sc })
  assert.equal(box.innerHTML, '<h1>loaded</h1>')
})
