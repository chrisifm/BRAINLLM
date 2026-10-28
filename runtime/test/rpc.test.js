// INTERNALS §11.1: rpc(id, args) → POST /_r/<id>, body = JSON array, header X-Brain-RPC: 1, JSON response.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { install } from '../shim/dom.js'

install()
const calls = []
let next = { status: 200, body: '{"id":7,"ok":true}' }
globalThis.fetch = async (url, init) => {
  calls.push({ url, init })
  if (next === 'network') throw TypeError('fetch failed')
  return { status: next.status, ok: next.status < 400, text: async () => next.body }
}
const rt = await import('./rt.js')
const { rpc } = await import(rt.base + 'brain.js')
const E = await rt.sibling('brain-err.js')
const errors = E.capture({ log: false })
const silence = async f => {
  const ce = console.error
  console.error = () => {}
  try {
    return await f()
  } finally {
    console.error = ce
  }
}

test('rpc: same-origin POST with anti-CSRF header; args as a JSON array; returns the JSON', async () => {
  const r = await rpc('k3x9/save', [{ name: 'Ana' }, 2])
  assert.deepEqual(r, { id: 7, ok: true })
  const { url, init } = calls[0]
  assert.equal(url, '/_r/k3x9/save')
  assert.equal(init.method, 'POST')
  assert.equal(init.headers['x-brain-rpc'], '1')
  assert.equal(init.headers['content-type'], 'application/json')
  assert.equal(init.body, '[{"name":"Ana"},2]')
})

test('rpc: empty body → undefined', async () => {
  next = { status: 204, body: '' }
  assert.equal(await rpc('k3x9/ping', []), undefined)
})

test('rpc: 422 carries the field errors for form() and is not reported as an app error', async () => {
  next = { status: 422, body: '{"error":"Invalid input","errors":{"email":"Already taken"}}' }
  await silence(() => assert.rejects(rpc('k3x9/save', [{}]), e => e.status === 422 && e.errors.email === 'Already taken'))
  assert.equal(errors.filter(e => e.where === 'rpc').length, 0)
})

test('rpc: other status ≥400 throws Error with the server message and reaches the capturer (where rpc)', async () => {
  next = { status: 403, body: '{"error":"Cross-origin request"}' }
  await silence(() => assert.rejects(rpc('k3x9/save', [{}]), e => e.message === 'Cross-origin request' && e.status === 403 && e.code === 'rpc'))
  next = { status: 500, body: '' }
  await silence(() => assert.rejects(rpc('k3x9/save', [{}]), /rpc k3x9\/save: 500/))
  const r = errors.filter(e => e.where === 'rpc')
  assert.equal(r.reduce((s, e) => s + e.n, 0), 2) // same fingerprint (code rpc + line + where): one with n=2
})

test('rpc: network failure is captured and rethrown', async () => {
  next = 'network'
  await silence(() => assert.rejects(rpc('k3x9/save', []), /fetch failed/))
  assert.ok(errors.some(e => e.where === 'rpc' && e.type === 'TypeError'))
})

test('#216 under a base path (<meta name="brain-base" content="/app/">) rpc posts to /app/_r/', async () => {
  next = { status: 200, body: '1' }
  const doc = globalThis.document, m = doc.createElement('meta')
  m.setAttribute('name', 'brain-base')
  m.setAttribute('content', '/app/')
  doc.head.appendChild(m)
  await rpc('k3x9/save', [])
  assert.equal(calls.at(-1).url, '/app/_r/k3x9/save')
  m.remove()
})
