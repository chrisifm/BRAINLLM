// Error documents retain status and document security under both supported output extensions.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { createServer } from '../server/server.mjs'
test('HTML and Brain error aliases preserve GET/HEAD status, CSP and selected document opener policy', { timeout: 20000 }, async () => {
  for (const extension of ['html', 'brain']) for (const base of ['/', '/site/']) {
    const dist = mkdtempSync(join(tmpdir(), 'brain-error-aliases-'))
    let server
    try {
      mkdirSync(join(dist, '_server')); mkdirSync(join(dist, '_b'))
      writeFileSync(join(dist, '_server/manifest.json'), JSON.stringify({ base, routes: [{ path: '/from-load', rx: '^/from-load$', server: '_server/missing.mjs' }], rpc: {}, notFound: `404.${extension}`, errors: Object.fromEntries([403, 404, 410, 503].map(status => [status, `${status}.${extension}`])) }))
      writeFileSync(join(dist, '_server/missing.mjs'), 'export function load(){ return {notFound:true} }')
      for (const status of [403, 404, 410, 503]) writeFileSync(join(dist, `${status}.${extension}`), `<html><head>${status === 404 ? '<meta name="brain-google-signin" content="1">' : ''}<title>Example ${status}</title></head><body>Selected error ${status}</body></html>`)
      writeFileSync(join(dist, '_b/example.js'), 'export const example=1;')
      server = createServer({ dist, log() {} })
      await new Promise(resolve => server.listen(0, '127.0.0.1', resolve))
      const origin = `http://127.0.0.1:${server.address().port}`, prefix = base.slice(0, -1)
      for (const [path, status] of [['/missing', 404], ['/from-load', 404], ...[403, 404, 410, 503].flatMap(status => [[`/${status}`, status], [`/${status}.${extension}`, status]])]) for (const method of ['GET', 'HEAD']) {
        const response = await fetch(origin + prefix + path, { method, headers: { accept: 'text/html' }, redirect: 'manual' })
        assert.equal(response.status, status, base + path)
        if (method === 'GET') assert.ok(response.headers.get('content-security-policy'), base + path)
        else assert.equal(response.headers.get('content-security-policy'), null, 'HEAD retains cached GET policy')
        assert.equal(response.headers.get('cross-origin-opener-policy'), status === 404 ? 'same-origin-allow-popups' : 'same-origin', base + path)
        const body = await response.text()
        if (method === 'HEAD') assert.equal(body, '')
        else assert.ok(body.includes(`Selected error ${status}`), body)
      }
      const asset = await fetch(origin + prefix + '/_b/example.js')
      assert.equal(asset.status, 200)
      assert.equal(asset.headers.get('content-security-policy'), null)
      assert.notEqual(asset.headers.get('cross-origin-opener-policy'), 'same-origin-allow-popups')
      await asset.arrayBuffer()
    } finally {
      if (server) await new Promise(resolve => { server.close(resolve); server.closeAllConnections() })
      rmSync(dist, { recursive: true, force: true })
    }
  }
})

test('own-extension documents, assets and Markdown twins use the same serving policy', { timeout: 15000 }, async () => {
  for (const base of ['/', '/site/']) {
    const dist = mkdtempSync(join(tmpdir(), 'brain-own-extension-'))
    let server
    try {
      mkdirSync(join(dist, '_server')); mkdirSync(join(dist, '_b')); mkdirSync(join(dist, 'folder'))
      writeFileSync(join(dist, '_server/manifest.json'), JSON.stringify({ base, routes: [], rpc: {}, notFound: '404.brain' }))
      const page = '<html><head><link rel="canonical" href="https://example.test/plain"><meta name="brain-google-signin" content="1"></head><body>Own document<script>window.example=1</script></body></html>'
      for (const path of ['index.brain', 'plain.brain', 'folder/index.brain', '404.brain']) writeFileSync(join(dist, path), page)
      writeFileSync(join(dist, 'plain.md'), '# Own document')
      writeFileSync(join(dist, '_b/example.jsbrain'), 'export const example=1;')
      writeFileSync(join(dist, '_b/example.cssbrain'), 'body{color:black}')
      server = createServer({ dist, log() {} })
      await new Promise(resolve => server.listen(0, '127.0.0.1', resolve))
      const origin = `http://127.0.0.1:${server.address().port}${base.slice(0, -1)}`
      for (const path of ['/', '/plain', '/plain.brain', '/folder', '/folder/', '/folder/index.brain']) {
        const response = await fetch(origin + path, { headers: { accept: 'text/html' } })
        assert.equal(response.status, 200, path)
        assert.match(response.headers.get('content-type'), /^text\/html/)
        assert.equal(response.headers.get('cross-origin-opener-policy'), 'same-origin-allow-popups')
        const body = await response.text(), nonce = /<script nonce="([^"]+)"/.exec(body)?.[1]
        assert.ok(nonce)
        assert.ok(response.headers.get('content-security-policy').includes(`'nonce-${nonce}'`))
      }
      for (const [path, type] of [['/_b/example.jsbrain', 'text/javascript'], ['/_b/example.cssbrain', 'text/css']]) {
        const response = await fetch(origin + path)
        assert.equal(response.status, 200)
        assert.ok(response.headers.get('content-type').startsWith(type))
        assert.equal(response.headers.get('content-security-policy'), null)
        await response.arrayBuffer()
      }
      const markdown = await fetch(origin + '/plain.md')
      assert.equal(markdown.headers.get('link'), '<https://example.test/plain>; rel="canonical"')
      await markdown.text()
      const error = await fetch(origin + '/404%2Ebrain', { headers: { accept: 'text/html' } })
      assert.equal(error.status, 404)
      await error.text()
    } finally {
      if (server) await new Promise(resolve => { server.close(resolve); server.closeAllConnections() })
      rmSync(dist, { recursive: true, force: true })
    }
  }
})

test('HEAD metadata cannot replace the nonce policy of a cached GET before revalidation', async () => {
  const dist = mkdtempSync(join(tmpdir(), 'brain-head-policy-'))
  let server
  try {
    mkdirSync(join(dist, '_server'))
    writeFileSync(join(dist, '_server/manifest.json'), JSON.stringify({ routes: [], rpc: {} }))
    writeFileSync(join(dist, 'index.html'), '<html><head></head><body><script>window.example=1</script></body></html>')
    server = createServer({ dist, log() {} })
    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve))
    const origin = `http://127.0.0.1:${server.address().port}/`
    const get = await fetch(origin), body = await get.text(), stored = new Headers(get.headers)
    const nonce = /<script nonce="([^"]+)"/.exec(body)[1]
    const head = await fetch(origin, { method: 'HEAD' })
    assert.equal(head.status, 200)
    assert.equal(head.headers.get('etag'), get.headers.get('etag'))
    // RFC 9111 sections 4.3.5 and 3.2: matching HEAD updates received fields, retaining absent fields/body.
    for (const [key, value] of head.headers) stored.set(key, value)
    const conditional = await fetch(origin, { headers: { 'if-none-match': stored.get('etag') } })
    assert.equal(conditional.status, 304)
    for (const [key, value] of conditional.headers) stored.set(key, value)
    assert.ok(stored.get('content-security-policy').includes(`'nonce-${nonce}'`), 'updated cache must still allow its stored script body')
    assert.equal(head.headers.get('cross-origin-opener-policy'), 'same-origin')
  } finally {
    if (server) await new Promise(resolve => { server.close(resolve); server.closeAllConnections() })
    rmSync(dist, { recursive: true, force: true })
  }
})
