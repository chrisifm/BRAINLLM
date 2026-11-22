// server.mjs SSR: an fx/drv error inside a render is logged once, structured, with the request id and path. The core's
// console fallback (`BRAIN_ERROR: Operation failed. Try again.`) is for the browser and never reaches the server log.
import { test, after } from 'node:test'
import assert from 'node:assert/strict'
import { copyFileSync, mkdirSync, mkdtempSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = fileURLToPath(new URL('.', import.meta.url))
const dist = mkdtempSync(join(tmpdir(), 'brain-ssr-errors-'))
const w = (rel, s) => (mkdirSync(join(dist, rel, '..'), { recursive: true }), writeFileSync(join(dist, rel), s))
mkdirSync(join(dist, '_b'), { recursive: true })
mkdirSync(join(dist, '_server/pages'), { recursive: true })
for (const [from, to] of [['../brain-guard.js', '_b/guard.js'], ['../brain.js', '_b/brain.js'], ['../brain-rpc.js', '_b/brain-rpc.js'], ['../shim/dom.js', '_server/dom.js']]) copyFileSync(join(here, from), join(dist, to))
w('_b/guarded.js', `import { tpl, insert } from '/_b/brain.js'
import { guard } from '/_b/guard.js'
const Bad = guard(() => { const r = tpl('<p class="b"><!></p>')(); insert(null, () => { throw new Error('binding exploded') }, r.firstChild); return r })
Bad.f = 'comp/bad.brain'
export default function Guarded() { return [tpl('<p class="ok">kept</p>')(), Bad({})] }
`)
w('_b/bare.js', `import { tpl, insert } from '/_b/brain.js'
export default function Bare() { const r = tpl('<p class="b"><!></p>')(); insert(null, () => { throw new TypeError('bare binding exploded') }, r.firstChild); return r }
`)
w('_b/late.js', `import { tpl, fx } from '/_b/brain.js'
export default function Late() { fx(() => { Promise.reject(new TypeError('late rejection')) }); return tpl('<p class="late">kept</p>')() }
`)
w('_b/rpcfx.js', `import { tpl, fx, rpc } from '/_b/brain.js'
export default function Rpc() { fx(() => { rpc('k3x9/list', []).then(() => { globalThis.__rpcResolved = true }) }); return tpl('<p class="rpc">kept</p>')() }
`)
w('_b/ld.js', `import { tpl } from '/_b/brain.js'
export default function Ld() { return tpl('<h1>Room</h1>')() }
Ld.seo = () => ({ title: 'Room', jsonld: [{ '@type': 'BreadcrumbList', itemListElement: [] }, { '@context': 'https://schema.org', '@type': 'Product', name: 'Room' }] })
`)
w('_b/faq.js', `import { tpl } from '/_b/brain.js'
const Landing = () => tpl('<section faq><h2>FAQ</h2><details><summary>Is it free?</summary><p>Yes, for <b>ten</b> people.</p></details></section>')()
export default function Faq() { return [tpl('<h1>Home</h1>')(), Landing()] }
`)
w('_server/busy.mjs', `export async function load(ctx) {
  if (ctx.query.kind === 'limited') throw Object.assign(new Error('rate limited for now'), { status: 429, retryAfter: 2 })
  if (ctx.query.kind === 'plain') throw { reason: 'private detail' }
  if (ctx.query.kind === 'broken') throw new Error('a real defect')
  throw Object.assign(new Error('backend restarting'), { status: 503, retryAfter: 3 })
}`)
w('_server/pages/page.html', '<!doctype html><html><head><title>Page</title></head><body><div id="brain-view"><!--brain:ssr--></div><script type="application/json" id="brain-props"><!--brain:props--></script></body></html>')
const route = (path, view) => ({ path, view, layouts: [], server: null, page: '_server/pages/page.html' })
w('_server/manifest.json', JSON.stringify({ v: 1, build: 'e1', core: '/_b/brain.js', routes: [route('/guarded', '/_b/guarded.js'), route('/bare', '/_b/bare.js'), route('/late', '/_b/late.js'), route('/faq', '/_b/faq.js'), route('/rpcfx', '/_b/rpcfx.js'), route('/ld', '/_b/ld.js'), { ...route('/busy', '/_b/bare.js'), server: '_server/busy.mjs' }] }))

const { createServer } = await import('../server/server.mjs')
const logs = []
const server = createServer({ dist, log: (level, msg, x) => logs.push({ level, msg, ...x }) })
await new Promise(r => server.listen(0, '127.0.0.1', r))
const base = `http://127.0.0.1:${server.address().port}`
after(() => server.close())

// what the process printed on its own console while a request rendered
const consoleDuring = async f => {
  const seen = [], original = console.error
  console.error = (...a) => seen.push(a.map(String).join(' '))
  try { await f() } finally { console.error = original }
  return seen
}

test('a guarded component that fails in an fx: one structured log with the rid, no anonymous console line', async () => {
  logs.length = 0
  let r
  const printed = await consoleDuring(async () => { r = await fetch(base + '/guarded', { headers: { 'x-request-id': 'req-g1' } }) })
  assert.equal(r.status, 200)
  assert.match(await r.text(), /<p class="ok">kept<\/p>/)
  assert.deepEqual(printed.filter(l => l.includes('BRAIN_ERROR')), [])
  const failed = logs.filter(l => l.msg === 'component failed during SSR')
  assert.equal(failed.length, 1)
  assert.equal(failed[0].rid, 'req-g1')
  assert.equal(failed[0].path, '/guarded')
  assert.equal(failed[0].where, 'fx')
  assert.equal(failed[0].comp, 'comp/bad.brain')
  assert.ok(!JSON.stringify(failed[0]).includes('binding exploded'))
})

test('the fallback of a component that failed on the server names the request, so a report finds the log line', async () => {
  const html = await (await fetch(base + '/guarded', { headers: { 'x-request-id': 'req-g2' } })).text()
  assert.match(html, /class="brain-fallback" data-rid="req-g2">[^<]*<small class="brain-ref">req-g2<\/small>/)
})

test('an unguarded fx failure while rendering: logged with rid, path, where and its error type instead of BRAIN_ERROR', async () => {
  logs.length = 0
  let r
  const printed = await consoleDuring(async () => { r = await fetch(base + '/bare', { headers: { 'x-request-id': 'req-b1' } }) })
  assert.equal(r.status, 200)
  await r.text()
  assert.deepEqual(printed.filter(l => l.includes('BRAIN_ERROR')), [])
  const failed = logs.filter(l => l.msg === 'component failed during SSR')
  assert.equal(failed.length, 1)
  assert.equal(failed[0].rid, 'req-b1')
  assert.equal(failed[0].path, '/bare')
  assert.equal(failed[0].where, 'fx')
  assert.equal(failed[0].comp, null)
  assert.equal(failed[0].errorType, 'TypeError')
  assert.ok(!JSON.stringify(failed[0]).includes('bare binding exploded'))
})

test('a temporary load failure (503/429) logs NODE_TEMPORARY_ERROR; a defect keeps NODE_INTERNAL_ERROR', async () => {
  logs.length = 0
  const busy = await fetch(base + '/busy', { headers: { 'x-request-id': 'req-t1' } })
  assert.equal(busy.status, 503)
  await busy.text()
  const limited = await fetch(base + '/busy?kind=limited', { headers: { 'x-request-id': 'req-t2' } })
  assert.equal(limited.status, 503)
  await limited.text()
  const broken = await fetch(base + '/busy?kind=broken', { headers: { 'x-request-id': 'req-t3' } })
  assert.equal(broken.status, 500)
  await broken.text()
  const by = rid => logs.find(l => l.rid === rid && /^load /.test(l.msg))
  assert.equal(by('req-t1').msg, 'load temporarily unavailable')
  assert.equal(by('req-t1').code, 'NODE_TEMPORARY_ERROR')
  assert.equal(by('req-t1').temporary, true)
  assert.equal(by('req-t2').code, 'NODE_TEMPORARY_ERROR')
  assert.equal(by('req-t3').msg, 'load failed')
  assert.equal(by('req-t3').code, 'NODE_INTERNAL_ERROR')
  assert.equal(by('req-t3').temporary, undefined)
})

test('load that throws a plain object: the log names the kind of value, never the value', async () => {
  logs.length = 0
  const r = await fetch(base + '/busy?kind=plain', { headers: { 'x-request-id': 'req-p1' } })
  assert.equal(r.status, 500)
  await r.text()
  const l = logs.find(l => l.rid === 'req-p1' && l.msg === 'load failed')
  assert.equal(l.thrown, 'object')
  assert.match(l.fix, /non-Error value \(object\)/)
  assert.ok(!JSON.stringify(l).includes('private detail'))
})

// node:test treats an unhandled rejection in its own process as a test failure, so the server runs in a child process
test('a promise an fx started and nobody caught: the page still answers and the failure is logged, the process lives', async () => {
  const { execFileSync } = await import('node:child_process')
  const serverUrl = new URL('../server/server.mjs', import.meta.url).href
  const script = `
    const { createServer } = await import(${JSON.stringify(serverUrl)})
    const logs = []
    const server = createServer({ dist: ${JSON.stringify(dist)}, log: (level, msg, x) => logs.push({ level, msg, ...x }) })
    await new Promise(r => server.listen(0, '127.0.0.1', r))
    const base = 'http://127.0.0.1:' + server.address().port
    const r = await fetch(base + '/late')
    const html = await r.text()
    await new Promise(done => setTimeout(done, 50))
    const again = await fetch(base + '/bare')
    await again.text()
    server.close()
    process.stdout.write(JSON.stringify({ status: r.status, kept: html.includes('class="late"'), again: again.status, logs }))
  `
  const out = JSON.parse(execFileSync(process.execPath, ['--input-type=module', '-e', script], { encoding: 'utf8', timeout: 20000 }))
  assert.equal(out.status, 200)
  assert.equal(out.kept, true)
  assert.equal(out.again, 200)
  const entry = out.logs.find(l => l.where === 'promise')
  assert.ok(entry, JSON.stringify(out.logs))
  assert.equal(entry.errorType, 'TypeError')
  assert.ok(!JSON.stringify(entry).includes('late rejection'))
})

test('a faq group inside a child component of a page rendered per request gets its FAQPage', async () => {
  const r = await fetch(base + '/faq')
  assert.equal(r.status, 200)
  const html = await r.text()
  const block = /<script\b[^>]*type="application\/ld\+json"[^>]*>([^<]*FAQPage[^<]*)<\/script>/.exec(html)
  assert.ok(block, html)
  const data = JSON.parse(block[1])
  assert.equal(data.mainEntity[0].name, 'Is it free?')
  assert.equal(data.mainEntity[0].acceptedAnswer.text, 'Yes, for ten people.')
})

test('a server function an fx calls while the page renders on the server is a no-op: no fetch, no failure', async () => {
  logs.length = 0
  const real = globalThis.fetch, seen = []
  globalThis.fetch = (u, init) => (seen.push(String(u)), real(u, init))
  let r, html
  try {
    r = await fetch(base + '/rpcfx', { headers: { 'x-request-id': 'req-rpc' } })
    html = await r.text()
    await new Promise(done => setTimeout(done, 20))
  } finally {
    globalThis.fetch = real
  }
  assert.equal(r.status, 200)
  assert.match(html, /<p class="rpc">kept<\/p>/)
  assert.deepEqual(seen.filter(u => u.includes('_r/')), [])
  assert.equal(globalThis.__rpcResolved, undefined)
  assert.deepEqual(logs.filter(l => l.rid === 'req-rpc' && l.level === 'error'), [])
})

test('jsonld nodes rendered per request get "@context" when the view left it out', async () => {
  const html = await (await fetch(base + '/ld')).text()
  const block = /<script\b[^>]*type="application\/ld\+json"[^>]*>([^<]*BreadcrumbList[^<]*)<\/script>/.exec(html)
  assert.ok(block, html)
  const data = JSON.parse(block[1])
  assert.equal(data[0]['@context'], 'https://schema.org')
  assert.equal(data[1]['@context'], 'https://schema.org')
  assert.equal(Object.keys(data[0])[0], '@context')
})
