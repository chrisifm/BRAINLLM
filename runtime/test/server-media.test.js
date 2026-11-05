import { test, after } from 'node:test'
import assert from 'node:assert/strict'
import { mkdtempSync, mkdirSync, writeFileSync, appendFileSync, renameSync, rmSync } from 'node:fs'
import { connect } from 'node:net'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { createHash } from 'node:crypto'
import { brotliCompressSync } from 'node:zlib'
import { createServer } from '../server/server.mjs'

const dist = mkdtempSync(join(tmpdir(), 'brain-media-'))
mkdirSync(join(dist, '_server'))
writeFileSync(join(dist, '_server/manifest.json'), JSON.stringify({ routes: [], rpc: {} }))
const body = Buffer.from('0123456789'.repeat(1000))
writeFileSync(join(dist, 'clip.mp4'), body)
writeFileSync(join(dist, 'clip.mp4.br'), brotliCompressSync(body))
writeFileSync(join(dist, 'atomic.mp4'), 'before')
writeFileSync(join(dist, 'replacement.mp4'), 'after!')
writeFileSync(join(dist, 'growing.mp4'), 'before')
writeFileSync(join(dist, 'empty.mp4'), '')
const types = { m3u8: 'application/vnd.apple.mpegurl', m3u: 'application/vnd.apple.mpegurl', m4s: 'video/iso.segment', mpd: 'application/dash+xml', ts: 'video/mp2t', vtt: 'text/vtt; charset=utf-8', mp3: 'audio/mpeg' }
for (const ext of Object.keys(types)) writeFileSync(join(dist, `stream.${ext}`), 'stream')
const server = createServer({ dist, log() {} })
server.prependListener('request', (req, res) => {
  if (!['/atomic.mp4', '/growing.mp4'].includes(req.url)) return
  const writeHead = res.writeHead
  res.writeHead = function (...args) {
    if (req.url === '/atomic.mp4') renameSync(join(dist, 'replacement.mp4'), join(dist, 'atomic.mp4'))
    else appendFileSync(join(dist, 'growing.mp4'), 'extra bytes')
    return writeHead.apply(this, args)
  }
})
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve))
const base = `http://127.0.0.1:${server.address().port}`
const get = (options = {}) => fetch(base + '/clip.mp4', { ...options, headers: { 'accept-encoding': 'identity', ...options.headers } })
after(async () => { await new Promise(resolve => { server.close(resolve); server.closeAllConnections() }); rmSync(dist, { recursive: true, force: true }) })

test('full media GET and HEAD share content length and a strong content validator', async () => {
  const etag = '"' + createHash('sha256').update(body).digest('base64url') + '"'
  for (const method of ['GET', 'HEAD']) {
    const r = await get({ method })
    assert.equal(r.status, 200)
    assert.equal(r.headers.get('content-length'), String(body.length))
    assert.equal(r.headers.get('transfer-encoding'), null)
    assert.equal(r.headers.get('etag'), etag)
    assert.equal(r.headers.get('accept-ranges'), 'bytes')
    assert.equal((await r.arrayBuffer()).byteLength, method === 'HEAD' ? 0 : body.length)
  }
  const cached = await get({ headers: { 'if-none-match': `W/${etag}` } })
  assert.equal(cached.status, 304)
  assert.equal(cached.headers.get('etag'), etag)
  assert.equal(await cached.text(), '')
})

test('If-Range accepts only the exact strong validator and never a compressed representation validator', async () => {
  const first = await get(), etag = first.headers.get('etag')
  await first.arrayBuffer()
  const compressed = await get({ headers: { 'accept-encoding': 'br' } }), compressedTag = compressed.headers.get('etag')
  assert.notEqual(compressedTag, etag)
  await compressed.arrayBuffer()
  for (const [validator, status] of [[etag, 206], [`W/${etag}`, 200], ['"stale"', 200], [compressedTag, 200], ['Wed, 01 Jan 2020 00:00:00 GMT', 200]]) {
    const r = await get({ headers: { range: 'bytes=1-3', 'if-range': validator } })
    assert.equal(r.status, status, validator)
    assert.equal(r.headers.get('etag'), etag)
    assert.equal(r.headers.get('content-length'), status === 206 ? '3' : String(body.length))
    assert.equal(await r.text(), status === 206 ? '123' : body.toString())
  }
  const partial = await get({ headers: { range: 'bytes=1-3', 'if-range': etag, 'accept-encoding': 'br' } })
  assert.equal(partial.status, 206)
  assert.equal(partial.headers.get('content-encoding'), null)
  assert.equal(partial.headers.get('content-range'), `bytes 1-3/${body.length}`)
  await partial.text()
})

test('replacing a same-size public file invalidates its content validator', async () => {
  writeFileSync(join(dist, 'changing.mp4'), 'before')
  const first = await fetch(base + '/changing.mp4'), etag = first.headers.get('etag')
  await first.text()
  writeFileSync(join(dist, 'changing.mp4'), 'after!')
  const next = await fetch(base + '/changing.mp4', { headers: { 'if-none-match': etag } })
  assert.equal(next.status, 200)
  assert.notEqual(next.headers.get('etag'), etag)
  assert.equal(await next.text(), 'after!')
})

test('an atomic deployment between validation and transmission keeps bytes and headers on one descriptor', async () => {
  const r = await fetch(base + '/atomic.mp4')
  assert.equal(r.status, 200)
  assert.equal(r.headers.get('content-length'), '6')
  assert.equal(r.headers.get('etag'), '"' + createHash('sha256').update('before').digest('base64url') + '"')
  assert.equal(await r.text(), 'before')
})

test('a growing file never emits bytes beyond Content-Length and empty media stays valid', async () => {
  const response = await new Promise((resolve, reject) => {
    const socket = connect(server.address().port, '127.0.0.1', () => socket.write('GET /growing.mp4 HTTP/1.1\r\nHost: localhost\r\nAccept-Encoding: identity\r\nConnection: close\r\n\r\n'))
    socket.setTimeout(5000, () => socket.destroy(new Error('Media response deadline exceeded')))
    const chunks = []
    socket.on('data', chunk => chunks.push(chunk))
    socket.on('end', () => resolve(Buffer.concat(chunks).toString()))
    socket.on('error', reject)
  })
  assert.match(response, /content-length: 6\r\n/i)
  assert.equal(response.split('\r\n\r\n')[1], 'before')
  const empty = await fetch(base + '/empty.mp4')
  assert.equal(empty.status, 200)
  assert.equal(empty.headers.get('content-length'), '0')
  assert.equal(await empty.text(), '')
})

test('streaming formats have registered content types under nosniff', async () => {
  for (const [ext, type] of Object.entries(types)) {
    const r = await fetch(base + `/stream.${ext}`)
    assert.equal(r.headers.get('content-type'), type, ext)
    assert.equal(r.headers.get('x-content-type-options'), 'nosniff')
    if (['m3u8', 'm3u', 'mpd'].includes(ext)) assert.equal(r.headers.get('cache-control'), 'no-cache')
    await r.text()
  }
})
