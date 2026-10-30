import { test, after } from 'node:test'
import assert from 'node:assert/strict'
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { STATUS_CODES, get as httpGet } from 'node:http'
import { createServer, redirect, runChain } from '../server/server.mjs'
const dist = mkdtempSync(join(tmpdir(), 'brain-http-status-'))
mkdirSync(join(dist, '_server'))
writeFileSync(join(dist, '_server/manifest.json'), JSON.stringify({ routes: [{ path: '/status/:code', rx: '^/status/(?<code>\\d+)/?$', server: '_server/status.mjs' }], rpc: {}, redirects: { exact: { '/old': ['/new', 308], '/partner': ['https://partner.example/new', 302], '/bad': ['//evil.example', 302] } } }))
writeFileSync(join(dist, '_server/status.mjs'), `import { HttpError } from ${JSON.stringify(new URL('../server/server.mjs', import.meta.url).href)}; export function load(ctx) { throw new HttpError(Number(ctx.params.code), 'private status details') }`)
writeFileSync(join(dist, 'server.mjs'), 'private server implementation')
writeFileSync(join(dist, '_headers'), 'private host configuration')
writeFileSync(join(dist, '.brain_env'), 'private environment marker')
writeFileSync(join(dist, '410.html'), '<!doctype html><title>Custom gone</title><main><h1>Custom gone</h1></main>')
writeFileSync(join(dist, '404.html'), '<!doctype html><title>Custom missing</title><main><h1>Custom missing</h1></main>')
writeFileSync(join(dist, 'google.html'), "<html><head><meta name='brain-google-signin' content='1'></head><body>Sign in</body></html>")
writeFileSync(join(dist, 'ordinary.html'), "<html><head><!-- <meta name='brain-google-signin' content='1'> --></head><body>Ordinary</body></html>")
const server = createServer({ dist, log() {} })
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve))
const base = `http://127.0.0.1:${server.address().port}`
after(async () => { await new Promise(resolve => { server.close(resolve); server.closeAllConnections() }); rmSync(dist, { recursive: true, force: true }) })
const request = (path, options = {}) => fetch(base + path, { redirect: 'manual', ...options })
test('custom pages survive HTML negotiation while JSON API defaults are traceable and private', async () => {
  const html = await request('/missing?private=do-not-reflect', { headers: { accept: 'text/html', 'x-request-id': 'known-reference' } })
  assert.equal(html.status, 404)
  assert.match(html.headers.get('cache-control'), /no-store/)
  assert.equal(html.headers.get('x-request-id'), 'known-reference')
  assert.match(await html.text(), /Custom missing/)
  for (const [path, accept] of [['/api/missing', '*/*'], ['/missing', 'application/json'], ['/missing', 'text/html;q=0,*/*;q=1']]) {
    const response = await request(path, { headers: { accept } })
    assert.equal(response.status, 404)
    assert.match(response.headers.get('content-type'), /application\/json/)
    const body = await response.json()
    assert.equal(body.rid, response.headers.get('x-request-id'))
    assert.equal(body.status, 404)
    assert.ok(!JSON.stringify(body).includes('private='))
  }
})
test('HEAD has no body for errors, static custom pages and health', async () => {
  for (const path of ['/missing', '/404.html', '/_health']) {
    const response = await request(path, { method: 'HEAD', headers: { accept: 'text/html' } })
    assert.equal(await response.text(), '')
  }
})
test('method errors have accessible mobile defaults without reflected request data', async () => {
  const response = await request('/private-token', { method: 'DELETE', headers: { accept: 'text/html' } })
  assert.equal(response.status, 405)
  assert.equal(response.headers.get('allow'), 'GET, HEAD, POST')
  const body = await response.text()
  assert.match(body, /<html lang="en">/)
  assert.match(body, /name="viewport"/)
  assert.match(body, /<main><h1>405 — Method Not Allowed/)
  assert.match(body, new RegExp(response.headers.get('x-request-id')))
  assert.ok(!body.includes('private-token'))
})
test('redirect helpers and guards reject invalid codes, cross-origin paths and response splitting', async () => {
  for (const code of [301, 302, 303, 307, 308]) assert.deepEqual(redirect('/home?tab=1', code), { redirect: '/home?tab=1', status: code })
  for (const code of [200, 304, 305, 399, 700, '303']) assert.throws(() => redirect('/home', code))
  for (const path of ['https://evil.example', '//evil.example', '/\\evil.example', '/\r\nX-Test: bad', '/%2fevil.example', '/%09/evil.example']) {
    assert.throws(() => redirect(path))
    await assert.rejects(runChain({ guards: [() => ({ redirect: path })], ctx: {} }))
  }
  const local = await request('/old?q=1'); assert.equal(local.status, 308); assert.equal(local.headers.get('location'), '/new?q=1')
  const partner = await request('/partner'); assert.equal(partner.status, 302); assert.equal(partner.headers.get('location'), 'https://partner.example/new')
  const rejected = await request('/bad', { headers: { accept: 'text/html' } }); assert.equal(rejected.status, 500); assert.ok(!(await rejected.text()).includes('evil.example'))
})

test('every registered HTTP error has a safe fallback, with explicit custom status pages preserved', async () => {
  for (const code of Object.keys(STATUS_CODES).map(Number).filter(code => code >= 400 && code <= 599)) {
    const response = await new Promise((resolve, reject) => {
      const req = httpGet(base + `/status/${code}`, { headers: { accept: 'text/html' } }, res => {
        let body = ''; res.on('data', chunk => { body += chunk; if (body.length > 32768) req.destroy(new Error('Oversized error response')) }); res.on('end', () => resolve({ status: res.statusCode, headers: res.headers, body })); res.on('error', reject)
      }); req.setTimeout(3000, () => req.destroy(new Error('HTTP fixture timeout'))); req.on('error', reject)
    })
    assert.equal(response.status, code)
    assert.match(response.headers['cache-control'], /no-store/)
    const body = response.body
    assert.ok(!body.includes('private status details'))
    assert.match(body, /<main>/)
    if (code === 410) assert.match(body, /Custom gone/)
    else if (code === 404) assert.match(body, /Custom missing/)
    else assert.match(body, new RegExp(`<h1>${code} —`))
  }
  const unknown = await request('/status/599', { headers: { accept: 'application/json' } })
  assert.equal(unknown.status, 500)
  assert.ok(!JSON.stringify(await unknown.json()).includes('private status details'))
})

test('Google opener policy is limited to actual document-head opt-in', async () => {
  const google = await request('/google'); assert.equal(google.headers.get('cross-origin-opener-policy'), 'same-origin-allow-popups')
  const ordinary = await request('/ordinary'); assert.equal(ordinary.headers.get('cross-origin-opener-policy'), 'same-origin')
})

test('static dispatch cannot expose encoded server modules, deployment files or dotfiles',async()=>{
  for(const path of ['/_server/manifest.json','/%5fserver/manifest.json','/_server%2fmanifest.json','/public%2f..%2f_server%2fmanifest.json','/server.mjs','/server%2emjs','/_headers','/%5fheaders','/.brain_env','/%2ebrain_env']){
    const response=await request(path)
    assert.equal(response.status,404,path)
    const body=await response.text()
    assert.ok(!body.includes('private server implementation')&&!body.includes('private environment marker')&&!body.includes('private host configuration'),path)
  }
})

test('same-site redirects preserve encoded public query spaces while path/control defenses stay enforced',()=>{
  assert.equal(redirect('/results?filter=last%2030%20days').redirect,'/results?filter=last%2030%20days')
  for(const path of ['/bad%20path','/results?filter=%0aheader','/results?filter=%5cevil','/'+String.fromCharCode(10)+'/evil.example'])assert.throws(()=>redirect(path))
})
