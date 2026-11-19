// Production cache behavior for public URLs that keep their name across deployments.
import { test, after } from 'node:test'
import assert from 'node:assert/strict'
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { brotliCompressSync } from 'node:zlib'
import { createServer } from '../server/server.mjs'

const dist = mkdtempSync(join(tmpdir(), 'brain-cache-policy-'))
mkdirSync(join(dist, '_server'))
mkdirSync(join(dist, '_b'))
writeFileSync(join(dist, '_server/manifest.json'), JSON.stringify({ routes: [], rpc: {} }))
const body = 'public content '.repeat(1000)
for (const path of ['settings.json', 'icon.svg', 'sw.js', 'stream.m3u8', '_b/content.txt', 'changing.txt']) {
  writeFileSync(join(dist, path), body)
  writeFileSync(join(dist, path + '.br'), brotliCompressSync(body))
}
writeFileSync(join(dist, 'about.html'), '<!doctype html><title>About</title><p>About</p>')
writeFileSync(join(dist, 'about.md'), '# About')
const server = createServer({ dist, log() {} })
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve))
const base = `http://127.0.0.1:${server.address().port}`
after(async () => {
  await new Promise(resolve => { server.close(resolve); server.closeAllConnections() })
  rmSync(dist, { recursive: true, force: true })
})

test('unhashed public files revalidate on GET, HEAD, 304 and Range; hashed assets stay immutable', async () => {
  for (const path of ['settings.json', 'icon.svg', 'sw.js', 'stream.m3u8', '_b/content.txt']) {
    const policy = path.startsWith('_b/') ? 'public, max-age=31536000, immutable' : 'no-cache'
    const get = await fetch(`${base}/${path}`, { headers: { 'accept-encoding': 'identity' } })
    assert.equal(get.status, 200)
    assert.equal(get.headers.get('cache-control'), policy, path)
    const etag = get.headers.get('etag')
    assert.ok(etag)
    assert.equal(await get.text(), body)
    for (const [method, headers, status] of [
      ['HEAD', {}, 200],
      ['GET', { 'if-none-match': etag }, 304],
      ['GET', { range: 'bytes=0-5', 'if-range': etag }, 206],
    ]) {
      const res = await fetch(`${base}/${path}`, { method, headers: { 'accept-encoding': 'identity', ...headers } })
      assert.equal(res.status, status, path)
      assert.equal(res.headers.get('cache-control'), policy, path)
      assert.equal(res.headers.get('etag'), etag)
      assert.equal(await res.text(), status === 206 ? body.slice(0, 6) : '')
    }
  }
})

test('compressed conditional responses retain representation encoding and validator', async () => {
  const first = await fetch(base + '/settings.json', { headers: { 'accept-encoding': 'br' } })
  const etag = first.headers.get('etag')
  assert.equal(first.headers.get('content-encoding'), 'br')
  assert.equal(await first.text(), body)
  const cached = await fetch(base + '/settings.json', { headers: { 'accept-encoding': 'br', 'if-none-match': etag } })
  assert.equal(cached.status, 304)
  assert.equal(cached.headers.get('content-encoding'), 'br')
  assert.equal(cached.headers.get('etag'), etag)
  assert.equal(cached.headers.get('cache-control'), 'no-cache')
  assert.equal(cached.headers.get('vary'), 'accept-encoding')
  assert.equal(await cached.text(), '')
})

test('a same-URL deployment replaces the public validator and response bytes', async () => {
  const first = await fetch(base + '/changing.txt', { headers: { 'accept-encoding': 'identity' } })
  const etag = first.headers.get('etag')
  await first.text()
  writeFileSync(join(dist, 'changing.txt'), 'updated content')
  const next = await fetch(base + '/changing.txt', { headers: { 'accept-encoding': 'identity', 'if-none-match': etag } })
  assert.equal(next.status, 200)
  assert.equal(next.headers.get('cache-control'), 'no-cache')
  assert.notEqual(next.headers.get('etag'), etag)
  assert.equal(await next.text(), 'updated content')
})

test('direct and negotiated Markdown twins use the same revalidation policy', async () => {
  for (const method of ['GET', 'HEAD']) {
    for (const path of ['/about.md', '/about']) {
      const res = await fetch(base + path, { method, headers: { accept: 'text/markdown', 'accept-encoding': 'identity' } })
      assert.equal(res.status, 200, path)
      assert.equal(res.headers.get('cache-control'), 'no-cache', path)
      assert.equal(await res.text(), method === 'HEAD' ? '' : '# About')
    }
  }
})
