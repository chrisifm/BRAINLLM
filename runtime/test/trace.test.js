import { test } from 'node:test'
import assert from 'node:assert/strict'
import { install } from '../shim/dom.js'

install()
globalThis.addEventListener ??= () => {}
globalThis.location ??= { href: 'http://app.test/page', origin: 'http://app.test', pathname: '/page', search: '' }
const seen = []
globalThis.fetch = async (input, init) => {
  seen.push({ url: String(input), headers: new Headers(init?.headers) })
  if (String(input).includes('offline')) throw new TypeError('Failed to fetch')
  return new Response('{"error":"no"}', { status: 500, headers: { 'content-type': 'application/json' } })
}
const T = await import('../brain-trace.js')
const batches = []
T.attach(text => { batches.push(JSON.parse(text)); return true })
const events = async () => {
  await new Promise(resolve => setTimeout(resolve, 150))
  return batches.flatMap(batch => batch.events)
}

test('a same-origin call carries the id it was recorded under; a cross-origin one carries nothing', async () => {
  await fetch('/_r/a/save', { method: 'POST', body: '["x"]' })
  await fetch('https://other.test/api')
  await assert.rejects(fetch('https://offline.test/api'))
  const [same, other] = seen
  assert.match(same.headers.get('x-brain-trace'), /^[0-9a-f-]{36}$/)
  assert.equal(other.headers.get('x-brain-trace'), null)
  const all = await events()
  const rpc = all.find(event => event.kind === 'rpc')
  assert.equal(rpc.request_id, same.headers.get('x-brain-trace'))
  assert.equal(rpc.status, 500)
  assert.equal(rpc.data.payload, '["x"]')
  assert.equal(rpc.data.response, '{"error":"no"}')
  assert.equal(T.lastFailed(), rpc.request_id)
  const refused = all.find(event => event.kind === 'cors')
  assert.equal(refused.level, 'error')
  assert.match(refused.title, /offline\.test/)
})

test('dev modules and runtime requests are not recorded', async () => {
  batches.length = 0
  await fetch('/views/home.brain')
  await fetch('/@brain/brain.js')
  assert.deepEqual(await events(), [])
})

test('leak findings of the inspector become memory events; other batches pass', async () => {
  batches.length = 0
  T.inspected(JSON.stringify({ t: 'm', d: [{ k: 'conn', id: 1 }] }))
  T.inspected(JSON.stringify({ t: 'm', d: [{ k: 'leak', code: 'B0601', name: 'Chart', file: 'comp/chart.brain', line: 4, msg: 'still running', fix: 'stop it' }] }))
  const all = await events()
  assert.equal(all.length, 1)
  assert.deepEqual([all[0].module, all[0].kind, all[0].code, all[0].file, all[0].line], ['memory', 'leak', 'B0601', 'comp/chart.brain', 4])
})

test('any module records with one call and no registration', async () => {
  batches.length = 0
  globalThis.__brainTrace('payments', 'declined', { level: 'error', title: 'Card declined' })
  const [event] = await events()
  assert.deepEqual([event.module, event.kind, event.title], ['payments', 'declined', 'Card declined'])
})
