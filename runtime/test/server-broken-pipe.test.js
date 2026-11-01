// A closed log pipe must not take the server down: stdout belongs to the operator, requests do not depend on it.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import { once } from 'node:events'
import { createServer } from 'node:net'

const server = fileURLToPath(new URL('../server/server.mjs', import.meta.url))
const freePort = () => new Promise((resolve) => {
  const s = createServer().listen(0, '127.0.0.1', () => { const { port } = s.address(); s.close(() => resolve(port)) })
})

test('the server keeps answering after its stdout closes and still shuts down cleanly', async () => {
  const port = await freePort()
  const child = spawn(process.execPath, [server], { env: { ...process.env, PORT: String(port), HOST: '127.0.0.1' }, stdio: ['ignore', 'pipe', 'pipe'] })
  let err = ''
  child.stderr.on('data', (d) => { err += d })
  await new Promise((resolve, reject) => {
    child.once('exit', (code) => reject(new Error(`exited ${code}: ${err}`)))
    let out = ''
    child.stdout.on('data', (d) => { out += d; if (/"msg":"listening"/.test(out)) resolve() })
  })
  child.removeAllListeners('exit')
  const gone = once(child, 'exit')
  child.stdout.destroy()
  for (let i = 0; i < 20; i++) {
    const res = await fetch(`http://127.0.0.1:${port}/_health`)
    assert.equal(res.status, 200, err)
    await fetch(`http://127.0.0.1:${port}/missing-${i}`).then((r) => r.text())
  }
  child.kill('SIGTERM')
  const [code, signal] = await gone
  assert.ok(code === 0 || signal === 'SIGTERM', `exit ${code}/${signal}: ${err}`)
  assert.doesNotMatch(err, /EPIPE/)
})

// A bare server writes nothing after it starts, so a preload stands in for the request logs of a real app.
test('the server survives log lines written after its pipes close', async () => {
  const port = await freePort()
  const noisy = 'data:text/javascript,setInterval(() => { console.log("line"); console.error("line") }, 10)'
  const child = spawn(process.execPath, ['--import', noisy, server], { env: { ...process.env, PORT: String(port), HOST: '127.0.0.1' }, stdio: ['ignore', 'pipe', 'pipe'] })
  let exited = null
  child.once('exit', (code, signal) => { exited = { code, signal } })
  let out = ''
  await new Promise((resolve, reject) => {
    child.once('exit', () => reject(new Error('exited before listening')))
    child.stdout.on('data', (d) => { out += d; if (/"msg":"listening"/.test(out)) resolve() })
  })
  child.removeAllListeners('exit')
  child.once('exit', (code, signal) => { exited = { code, signal } })
  child.stdout.destroy()
  child.stderr.destroy()
  await new Promise((resolve) => setTimeout(resolve, 500))
  assert.equal(exited, null, `the server exited: ${JSON.stringify(exited)}`)
  assert.equal((await fetch(`http://127.0.0.1:${port}/_health`)).status, 200)
  const gone = once(child, 'exit')
  child.kill('SIGTERM')
  await gone
})
