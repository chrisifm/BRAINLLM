// Headers a visitor controls never cost a 500: an undecodable cookie keeps its raw value, an unparsable target is a 400.
import { test, after } from 'node:test'
import assert from 'node:assert/strict'
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { connect } from 'node:net'
import { createServer } from '../server/server.mjs'

const dist = mkdtempSync(join(tmpdir(), 'brain-hostile-'))
mkdirSync(join(dist, '_server'))
writeFileSync(join(dist, '_server/data.mjs'), `export async function read(ctx) { return ctx.cookies }\nexport const $server = ['read']`)
writeFileSync(join(dist, '_server/manifest.json'), JSON.stringify({ routes: [], rpc: { data: '_server/data.mjs' } }))
const logs = []
const server = createServer({ dist, secret: 'synthetic-hostile-secret', log: (level, msg, extra) => logs.push({ level, msg, ...extra }) })
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve))
const port = server.address().port, base = `http://127.0.0.1:${port}`
after(async () => {
  await new Promise(resolve => { server.close(resolve); server.closeAllConnections() })
  rmSync(dist, { recursive: true, force: true })
})

const raw = request => new Promise((resolve, reject) => {
  let out = ''
  const socket = connect(port, '127.0.0.1', () => socket.write(request))
  socket.on('data', chunk => { out += chunk })
  socket.on('end', () => resolve(out))
  socket.on('error', reject)
})

test('a cookie that is not valid percent-encoding keeps its raw value and the others decode', async () => {
  const r = await fetch(`${base}/_r/data/read`, { method: 'POST', headers: { origin: base, 'x-brain-rpc': '1', 'content-type': 'application/json', cookie: 'foo=%%%; good=a%20b; bad=%E0%A4%A' }, body: '[]' })
  assert.equal(r.status, 200)
  assert.deepEqual(await r.json(), { foo: '%%%', good: 'a b', bad: '%E0%A4%A' })
})

test('a Host or a target the URL parser refuses answers 400 and logs no error', async () => {
  for (const request of ['GET / HTTP/1.1\r\nHost: %%%\r\nConnection: close\r\n\r\n', 'GET http://[ HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n']) {
    const out = await raw(request)
    assert.match(out, /^HTTP\/1\.1 400 /, out.slice(0, 200))
  }
  assert.deepEqual(logs.filter(entry => entry.level === 'error'), [])
})
