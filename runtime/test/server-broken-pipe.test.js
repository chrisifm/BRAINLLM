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
