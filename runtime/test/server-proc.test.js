// server.mjs as a process under a supervisor (INTERNALS §34): listens on an inherited socket (BRAIN_LISTEN_FD),
// prints `brain:ready`, drains on SIGTERM (in-flight requests finish, new ones are refused, exit 0; forced after
// BRAIN_DRAIN_MS). The parent here plays `brain start`: it binds the port and hands the fd to the child.
import { after, test as nodeTest } from 'node:test'
import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { copyFileSync, mkdirSync, mkdtempSync, writeFileSync } from 'node:fs'
import { createServer as tcp } from 'node:net'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'

// A server that stops answering must fail this file, never hang it: every test has a deadline and every child dies with it.
// The bound is a hang detector, not a performance assertion.
const test = (name, run) => nodeTest(name, { timeout: 60_000 }, run)
const children = new Set()
const launch = (...args) => {
  const child = spawn(...args)
  children.add(child)
  child.on('exit', () => children.delete(child))
  return child
}
after(() => { for (const child of children) child.kill('SIGKILL') })

const here = fileURLToPath(new URL('.', import.meta.url))
const { env: ENV } = process
const dist = mkdtempSync(join(tmpdir(), 'brain-proc-'))
mkdirSync(join(dist, '_server'), { recursive: true })
copyFileSync(join(here, '../server/server.mjs'), join(dist, 'server.mjs'))
copyFileSync(join(here, '../server/native-sockets.mjs'), join(dist, 'native-sockets.mjs'))
copyFileSync(join(here, '../server/llm.mjs'), join(dist, 'llm.mjs'))
copyFileSync(join(here, '../server/redis-cache.mjs'), join(dist, 'redis-cache.mjs'))
writeFileSync(join(dist, 'index.html'), '<!doctype html><title>x</title><main><h1>Home</h1></main>')
writeFileSync(join(dist, '_server/slow.mjs'), 'export async function wait(ctx, ms) { await new Promise(r => setTimeout(r, ms)); return { waited: ms, pid: process.pid } }\nexport const $server = [\'wait\']\n')
writeFileSync(join(dist, '_server/manifest.json'), JSON.stringify({ v: 1, routes: [], rpc: { s: '_server/slow.mjs' } }))

// bind like the supervisor, start the child with the socket as fd 3, then close the parent's copy (only the child accepts)
async function supervise(env = {}) {
  const sock = tcp()
  await new Promise(r => sock.listen(0, '127.0.0.1', r))
  const port = sock.address().port
  const child = launch(process.execPath, [join(dist, 'server.mjs')], {
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
    const c = launch(process.execPath, [join(dist, 'server.mjs')], { stdio: ['ignore', 'pipe', 'pipe', sock._handle.fd], env: { ...ENV, BRAIN_LISTEN_FD: '3' } })
    const ready = new Promise((ok, fail) => { c.stdout.on('data', d => /brain:ready/.test(d) && ok()); c.on('exit', code => fail(new Error('exited before ready: ' + code))) })
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

test('without an inherited socket a restart waits for the port, and a live holder is never shared', async () => {
  const held = tcp()
  await new Promise(r => held.listen(0, '127.0.0.1', r))
  const port = held.address().port
  const start = () => {
    const child = launch(process.execPath, [join(dist, 'server.mjs')], { stdio: ['ignore', 'pipe', 'pipe'], env: { ...ENV, BRAIN_LISTEN_FD: '', PORT: String(port), HOST: '127.0.0.1' } })
    let err = ''
    child.stderr.on('data', d => (err += d))
    const ready = new Promise((ok, fail) => { child.stdout.on('data', d => /brain:ready/.test(d) && ok(true)); child.on('exit', code => fail(new Error('exited before ready: ' + code))) })
    ready.catch(() => {}) // the second server in the last test is expected to exit without ever being ready
    const exited = new Promise(r => child.on('exit', code => r(code)))
    return { child, ready, exited, err: () => err }
  }
  const waiting = start()
  await new Promise(r => setTimeout(r, 400))
  held.close()
  assert.equal(await waiting.ready, true)
  assert.equal((await fetch(`http://127.0.0.1:${port}/`)).status, 200)
  // The running server is now the live holder: a second one must give up with the real cause, not share the port.
  const second = start()
  assert.equal(await second.exited, 1)
  assert.match(second.err(), /another live process still holds the port/)
  assert.match(second.err(), new RegExp(`lsof -nP -iTCP:${port}`))
  waiting.child.kill('SIGTERM')
  assert.equal(await waiting.exited, 0)
})
