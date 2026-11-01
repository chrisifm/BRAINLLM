// A request body that never completes settles its reader, and the unhandled-error log never carries the query string.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import http from 'node:http'
import net from 'node:net'
import { spawn } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import { readBody } from '../server/server.mjs'

const server = fileURLToPath(new URL('../server/server.mjs', import.meta.url))
const settles = (promise, ms = 2000) => Promise.race([
  promise.then(() => 'resolved', (e) => e.aborted ? 'aborted' : 'rejected'),
  new Promise((resolve) => setTimeout(() => resolve('pending'), ms)),
])
function reader(max) {
  return new Promise((resolve) => {
    const s = http.createServer((req) => resolve({ s, read: readBody(req, max) }))
    s.listen(0, '127.0.0.1', () => {
      const c = net.connect(s.address().port, '127.0.0.1', () => {
        c.write(`POST / HTTP/1.1\r\nHost: x\r\nContent-Length: ${1 << 24}\r\n\r\npartial`)
        s.$client = c
      })
      c.on('error', () => {})
    })
  })
}

test('a client that disconnects mid-body settles the reader as aborted', async () => {
  const { s, read } = await reader(1024)
  s.$client.destroy()
  assert.equal(await settles(read), 'aborted')
  s.close()
})

test('a body far past the limit is cut and the reader still settles', async () => {
  const { s, read } = await reader(8)
  // limit + 15x drain: 128 bytes past it the request is destroyed
  s.$client.write(Buffer.alloc(4096))
  assert.equal(await settles(read), 'aborted')
  s.$client.destroy()
  s.close()
})

test('a complete body resolves and a body over the limit rejects as too large', async () => {
  const ok = await new Promise((resolve) => {
    const s = http.createServer(async (req, res) => { const t = await readBody(req, 16); res.end(t); s.close(); resolve(t) })
    s.listen(0, '127.0.0.1', () => fetch(`http://127.0.0.1:${s.address().port}/`, { method: 'POST', body: 'hello' }))
  })
  assert.equal(ok, 'hello')
  const big = await new Promise((resolve) => {
    const s = http.createServer(async (req, res) => { const r = await settles(readBody(req, 2)); res.end(); s.close(); resolve(r) })
    s.listen(0, '127.0.0.1', () => fetch(`http://127.0.0.1:${s.address().port}/`, { method: 'POST', body: 'hello' }))
  })
  assert.equal(big, 'rejected')
})

test('the unhandled-error log keeps the path and drops the query', async () => {
  const port = await new Promise((resolve) => { const p = net.createServer().listen(0, '127.0.0.1', function () { const n = this.address().port; this.close(() => resolve(n)) }); void p })
  const child = spawn(process.execPath, [server], { env: { ...process.env, PORT: String(port), HOST: '127.0.0.1' }, stdio: ['ignore', 'pipe', 'pipe'] })
  let out = ''
  child.stdout.on('data', (d) => { out += d })
  child.stderr.on('data', (d) => { out += d })
  try {
    for (let i = 0; i < 100 && !/"msg":"listening"/.test(out); i++) await new Promise((r) => setTimeout(r, 50))
    // an invalid Host makes new URL() throw inside the request handler: the catch-all logs it
    await new Promise((resolve) => {
      const c = net.connect(port, '127.0.0.1', () => c.write('GET /private/path?reset_token=SECRETVALUE HTTP/1.1\r\nHost: [\r\nConnection: close\r\n\r\n'))
      c.on('data', () => {}); c.on('close', resolve); c.on('error', resolve)
    })
    for (let i = 0; i < 40 && !/"msg":"unhandled"/.test(out); i++) await new Promise((r) => setTimeout(r, 50))
    assert.match(out, /"msg":"unhandled"/)
    assert.match(out, /\/private\/path/)
    assert.doesNotMatch(out, /SECRETVALUE/)
  } finally {
    child.kill()
  }
})
