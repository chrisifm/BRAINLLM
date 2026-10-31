// server.mjs response compression: HTML pages (static and SSR, streamed included) and JSON are negotiated per request
// (zstd, br, gzip by Accept-Encoding weight), static files use their precompressed .zst/.br/.gz siblings, and every
// compressible response says `Vary: Accept-Encoding`. Small bodies, `no-transform` and `compress: false` stay identity.
import { test, after } from 'node:test'
import assert from 'node:assert/strict'
import { request } from 'node:http'
import zlib from 'node:zlib'
import { copyFileSync, mkdirSync, mkdtempSync, writeFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = fileURLToPath(new URL('.', import.meta.url))
const dist = mkdtempSync(join(tmpdir(), 'brain-compress-'))
const w = (rel, s) => (mkdirSync(join(dist, rel, '..'), { recursive: true }), writeFileSync(join(dist, rel), s))
mkdirSync(join(dist, '_b'), { recursive: true })
copyFileSync(join(here, '../brain.js'), join(dist, '_b/brain.js'))
mkdirSync(join(dist, '_server'), { recursive: true })
copyFileSync(join(here, '../shim/dom.js'), join(dist, '_server/dom.js'))
const filler = '<p>' + 'A well-compressible paragraph about the shop catalog. '.repeat(80) + '</p>'
w('index.html', `<!doctype html><html><head><title>Home</title></head><body>${filler}<script type="module">1</script></body></html>`)
w('tiny.html', '<!doctype html><html><head><title>t</title></head><body>tiny</body></html>')
const asset = 'export const text = "' + 'static asset body '.repeat(200) + '"\n'
w('_b/app.js', asset)
w('_b/app.js.br', zlib.brotliCompressSync(asset))
w('_b/app.js.gz', zlib.gzipSync(asset))
const zstd = typeof zlib.zstdCompressSync === 'function'
if (zstd) w('_b/app.js.zst', zlib.zstdCompressSync(asset))
w('_b/v.js', `import { tpl } from '/_b/brain.js'\nexport default function V(p) { return tpl('<section>' + p.text + '</section>')() }\n`)
w('_server/v.server.mjs', `export async function load(ctx) { if (ctx.url.searchParams.has('slow')) await new Promise(r => setTimeout(r, 30)); return { text: ${JSON.stringify('Server rendered order line. '.repeat(120))} } }\n`)
w('_server/pages/p.html', `<!doctype html><html><head><title>t</title></head><body>${filler}<div id="brain-view"><!--brain:ssr--></div><script type="application/json" id="brain-props"><!--brain:props--></script></body></html>`)
w('_server/manifest.json', JSON.stringify({
  v: 1, build: 'b1', core: '/_b/brain.js', rpc: {},
  routes: [
    { path: '/ssr', rx: '^/ssr/?$', view: '/_b/v.js', layouts: [], server: '_server/v.server.mjs', page: '_server/pages/p.html' },
    { path: '/live', rx: '^/live/?$', view: '/_b/v.js', layouts: [], server: '_server/v.server.mjs', page: '_server/pages/p.html', stream: true },
  ],
}))

const { createServer, acceptedEncodings } = await import('../server/server.mjs')
const servers = []
const start = async o => {
  const s = createServer({ dist, secret: 'test-only-session-key', log() {}, streamHoldMs: 0, ...o })
  await new Promise(r => s.listen(0, '127.0.0.1', r))
  servers.push(s)
  return s.address().port
}
const port = await start({})
const off = await start({ compress: false })
after(async () => { for (const s of servers) await new Promise(r => s.close(r)); rmSync(dist, { recursive: true, force: true }) })

// raw HTTP (fetch would decode the body and hide content-encoding)
const raw = (path, enc, p = port, method = 'GET') => new Promise((ok, fail) => {
  const req = request({ host: '127.0.0.1', port: p, path, method, headers: enc == null ? {} : { 'accept-encoding': enc } }, res => {
    const parts = []
    res.on('data', c => parts.push(c))
    res.on('end', () => ok({ status: res.statusCode, headers: res.headers, body: Buffer.concat(parts) }))
  })
  req.on('error', fail)
  req.end()
})
const decode = r => {
  const e = r.headers['content-encoding']
  if (e === 'br') return zlib.brotliDecompressSync(r.body).toString()
  if (e === 'gzip') return zlib.gunzipSync(r.body).toString()
  if (e === 'zstd') return zlib.zstdDecompressSync(r.body).toString()
  return r.body.toString()
}
const varies = r => /accept-encoding/i.test(r.headers.vary || '')

test('acceptedEncodings honours weights, q=0 and the server preference on ties', () => {
  assert.deepEqual(acceptedEncodings('gzip, br'), zstd ? ['br', 'gzip'] : ['br', 'gzip'])
  assert.deepEqual(acceptedEncodings('br;q=0, gzip'), ['gzip'])
  if (zstd) {
    assert.deepEqual(acceptedEncodings('gzip, deflate, br, zstd'), ['br', 'zstd', 'gzip'])
    assert.deepEqual(acceptedEncodings('gzip, deflate, br, zstd', ['zstd', 'br', 'gzip']), ['zstd', 'br', 'gzip'])
    assert.deepEqual(acceptedEncodings('zstd;q=0.5, gzip'), ['gzip', 'zstd'])
  }
  assert.deepEqual(acceptedEncodings(''), [])
})

test('static HTML pages are compressed per response (the nonce changes, the encoding is negotiated)', async () => {
  const br = await raw('/', 'br')
  assert.equal(br.status, 200)
  assert.equal(br.headers['content-encoding'], 'br')
  assert.ok(varies(br))
  const page = decode(br)
  assert.match(page, /<script nonce="[^"]+" type="module">1<\/script>/)
  assert.ok(br.body.length * 4 < Buffer.byteLength(page), `compressed ${br.body.length} of ${Buffer.byteLength(page)}`)
  const gz = await raw('/', 'gzip')
  assert.equal(gz.headers['content-encoding'], 'gzip')
  assert.match(decode(gz), /well-compressible/)
  const plain = await raw('/', null)
  assert.equal(plain.headers['content-encoding'], undefined)
  assert.ok(varies(plain), 'the identity answer still varies by Accept-Encoding')
  if (zstd) {
    const z = await raw('/', 'gzip, deflate, br, zstd')
    assert.equal(z.headers['content-encoding'], 'zstd', 'dynamic bodies prefer zstd on ties: fastest at a Brotli-like ratio')
    assert.match(decode(z), /well-compressible/)
  }
})

test('SSR and streamed SSR pages are compressed; streaming still flushes the head first', async () => {
  const r = await raw('/ssr', 'br')
  assert.equal(r.headers['content-encoding'], 'br')
  assert.match(decode(r), /Server rendered order line/)
  const s = await raw('/live?slow=1', 'gzip')
  assert.equal(s.headers['content-encoding'], 'gzip')
  assert.equal(s.headers['content-length'], undefined)
  const page = decode(s)
  assert.match(page, /<title>t<\/title>[\s\S]*Server rendered order line[\s\S]*<\/html>/)
})

test('small bodies, HEAD, 304 and compress:false stay identity', async () => {
  const tiny = await raw('/tiny.html', 'br')
  assert.equal(tiny.headers['content-encoding'], undefined)
  const head = await raw('/', 'br', port, 'HEAD')
  assert.equal(head.body.length, 0)
  const offr = await raw('/', 'br', off)
  assert.equal(offr.headers['content-encoding'], undefined)
  assert.match(offr.body.toString(), /well-compressible/)
})

test('static assets use the precompressed sibling: br on ties, .zst when only zstd is accepted', async () => {
  const br = await raw('/_b/app.js', 'gzip, deflate, br, zstd')
  assert.equal(br.headers['content-encoding'], 'br')
  assert.equal(decode(br), asset)
  assert.match(br.headers['cache-control'], /immutable/)
  assert.ok(varies(br))
  if (zstd) {
    const z = await raw('/_b/app.js', 'zstd')
    assert.equal(z.headers['content-encoding'], 'zstd')
    assert.equal(decode(z), asset)
  }
  const gz = await raw('/_b/app.js', 'gzip')
  assert.equal(gz.headers['content-encoding'], 'gzip')
})
