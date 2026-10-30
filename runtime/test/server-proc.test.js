// server.mjs as a process under a supervisor (INTERNALS §34): listens on an inherited socket (BRAIN_LISTEN_FD),
// prints `brain:ready`, drains on SIGTERM (in-flight requests finish, new ones are refused, exit 0; forced after
// BRAIN_DRAIN_MS). The parent here plays `brain start`: it binds the port and hands the fd to the child.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { copyFileSync, mkdirSync, mkdtempSync, writeFileSync } from 'node:fs'
import { createServer as tcp } from 'node:net'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = fileURLToPath(new URL('.', import.meta.url))
const { env: ENV } = process
const dist = mkdtempSync(join(tmpdir(), 'brain-proc-'))
mkdirSync(join(dist, '_server'), { recursive: true })
copyFileSync(join(here, '../server/server.mjs'), join(dist, 'server.mjs'))
copyFileSync(join(here, '../server/native-sockets.mjs'), join(dist, 'native-sockets.mjs'))
writeFileSync(join(dist, 'index.html'), '<!doctype html><title>x</title><main><h1>Home</h1></main>')
writeFileSync(join(dist, '_server/slow.mjs'), 'export async function wait(ctx, ms) { await new Promise(r => setTimeout(r, ms)); return { waited: ms, pid: process.pid } }\n')
writeFileSync(join(dist, '_server/manifest.json'), JSON.stringify({ v: 1, routes: [], rpc: { s: '_server/slow.mjs' } }))

// bind like the supervisor, start the child with the socket as fd 3, then close the parent's copy (only the child accepts)
async function supervise(env = {}) {
  const sock = tcp()
  await new Promise(r => sock.listen(0, '127.0.0.1', r))
  const port = sock.address().port
  const child = spawn(process.execPath, [join(dist, 'server.mjs')], {
    stdio: ['ignore', 'pipe', 'pipe', sock._handle.fd],
    env: { ...ENV, BRAIN_LISTEN_FD: '3', PORT: '1', ...env }, // PORT must be ignored when the fd is set
  })
  let out = ''
  const ready = new Promise((ok, fail) => {
    child.stdout.on('data', d => {
      out += d
      const m = /^brain:ready (\d+)$/m.exec(out)
      if (m) ok(+m[1])
    })
    child.on('exit', c => fail(new Error('exited before ready: ' + c)))
  })
  const pid = await ready
  sock.close()
  const exited = new Promise(r => child.on('exit', (code, sig) => r({ code, sig })))
  return { child, port, pid, exited, out: () => out }
}
const call = (port, ms) =>
  fetch(`http://127.0.0.1:${port}/_r/s/wait`, { method: 'POST', headers: { 'x-brain-rpc': '1', 'content-type': 'application/json' }, body: JSON.stringify([ms]) })

test('listens on the inherited fd (PORT ignored), prints brain:ready <pid>, serves', async () => {
  const p = await supervise()
  assert.equal(p.pid, p.child.pid)
  const r = await fetch(`http://127.0.0.1:${p.port}/`)
  assert.equal(r.status, 200)
  assert.match(await r.text(), /Home/)
  assert.deepEqual((await (await call(p.port, 1)).json()).pid, p.child.pid)
  p.child.kill('SIGTERM')
  assert.deepEqual(await p.exited, { code: 0, sig: null })
})

test('SIGTERM drains: the request in flight completes, the next one is refused, exit 0', async () => {
  const p = await supervise()
  const slow = call(p.port, 400)
  await new Promise(r => setTimeout(r, 100)) // the request is in flight
  const t0 = Date.now()
  p.child.kill('SIGTERM')
  const r = await slow
  assert.equal(r.status, 200)
  assert.equal(r.headers.get('connection'), 'close')
  assert.equal((await r.json()).waited, 400)
  const { code } = await p.exited
  assert.equal(code, 0)
  assert.ok(Date.now() - t0 < 2000)
  await assert.rejects(fetch(`http://127.0.0.1:${p.port}/`), /fetch failed/) // nobody accepts anymore
})

test('drain limit: a request longer than BRAIN_DRAIN_MS is cut and the process still exits 0 on time', async () => {
  const p = await supervise({ BRAIN_DRAIN_MS: '200' })
  const slow = call(p.port, 5000).then(r => r.status, e => 'cut')
  await new Promise(r => setTimeout(r, 100))
  const t0 = Date.now()
  p.child.kill('SIGTERM')
  const { code } = await p.exited
  assert.equal(code, 0)
  assert.ok(Date.now() - t0 < 1500, 'did not wait for the 5 s request')
  assert.equal(await slow, 'cut')
})

test('two children on the same socket: the old one drains, the new one keeps serving (reload)', async () => {
  const sock = tcp()
  await new Promise(r => sock.listen(0, '127.0.0.1', r))
  const port = sock.address().port
  const start = () => {
    const c = spawn(process.execPath, [join(dist, 'server.mjs')], { stdio: ['ignore', 'pipe', 'pipe', sock._handle.fd], env: { ...ENV, BRAIN_LISTEN_FD: '3' } })
    const ready = new Promise(ok => c.stdout.on('data', d => /brain:ready/.test(d) && ok()))
    const exited = new Promise(r => c.on('exit', code => r(code)))
    return { c, ready, exited }
  }
  const a = start()
  await a.ready
  const b = start()
  await b.ready
  sock.close()
  a.c.kill('SIGTERM')
  assert.equal(await a.exited, 0)
  for (let i = 0; i < 5; i++) assert.equal((await (await call(port, 1)).json()).pid, b.c.pid)
  b.c.kill('SIGTERM')
  assert.equal(await b.exited, 0)
})
