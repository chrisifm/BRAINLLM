// A 103 is opt-in: nginx below 1.29 takes it as the final response. The Link header stays on the real response.
import { test, after } from 'node:test'
import assert from 'node:assert/strict'
import http from 'node:http'
import net from 'node:net'
import { copyFileSync, mkdirSync, mkdtempSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = fileURLToPath(new URL('.', import.meta.url))
const dist = mkdtempSync(join(tmpdir(), 'brain-hints-'))
const w = (rel, s) => (mkdirSync(join(dist, rel, '..'), { recursive: true }), writeFileSync(join(dist, rel), s))
mkdirSync(join(dist, '_b'), { recursive: true })
copyFileSync(join(here, '../brain.js'), join(dist, '_b/brain.js'))
mkdirSync(join(dist, '_server'), { recursive: true })
copyFileSync(join(here, '../shim/dom.js'), join(dist, '_server/dom.js'))
w('_b/v.js', `import { tpl } from '/_b/brain.js'\nexport default function V() { return tpl('<h1>page</h1>')() }\n`)
w('_server/v.server.mjs', "export const seo = () => ({ title: 'Page' })\nexport async function load() { return { msg: 'loaded' } }\n")
w('_server/pages/p.html', '<!doctype html><html><head><!--brain-head--></head><body><div id="app"><!--brain-ssr--></div><!--brain-props--></body></html>')
const link = '</_b/app.css>; rel=preload; as=style'
const manifest = links => JSON.stringify({ v: 1, build: 'b1', core: '/_b/brain.js', rpc: {}, routes: [{ path: '/p', rx: '^/p/?$', view: '/_b/v.js', layouts: [], server: '_server/v.server.mjs', page: '_server/pages/p.html', stream: true, links }] })
w('_server/manifest.json', manifest([link]))

const { createServer } = await import('../server/server.mjs')
const servers = []
const start = async opts => {
  const server = createServer({ dist, log: () => {}, ...opts })
  await new Promise(ok => server.listen(0, '127.0.0.1', ok))
  servers.push(server)
  return server.address().port
}
after(() => servers.forEach(s => (s.closeAllConnections?.(), s.close())))

const raw = (port, version) => new Promise((ok, no) => {
  let text = ''
  const socket = net.connect(port, '127.0.0.1', () => socket.write(`GET /p HTTP/${version}\r\nHost: x\r\nConnection: close\r\n\r\n`))
  socket.on('data', d => (text += d)).on('end', () => ok(text)).on('error', no)
})

test('the default sends no 103 and keeps the preloads on the final response', async () => {
  const text = await raw(await start({}), '1.1')
  assert.match(text, /^HTTP\/1\.1 200 /)
  assert.doesNotMatch(text, / 103 /)
  assert.match(text, /\r\nlink: <\/_b\/app\.css>; rel=preload; as=style\r\n/i)
})

test('earlyHints sends the 103 to HTTP/1.1 and never to HTTP/1.0', async () => {
  const port = await start({ earlyHints: true })
  assert.match(await raw(port, '1.1'), /^HTTP\/1\.1 103 /)
  const old = await raw(port, '1.0')
  assert.doesNotMatch(old, / 103 /)
  assert.match(old, /^HTTP\/1\.1 200 /)
})

test('a long preload list stays out of the header', async () => {
  const many = Array.from({ length: 200 }, (_, i) => `</_b/f${i}.js>; rel=modulepreload`)
  w('_server/manifest.json', manifest(many))
  const text = await raw(await start({}), '1.1')
  assert.match(text, /^HTTP\/1\.1 200 /)
  assert.doesNotMatch(text, /\r\nlink:/i)
})
