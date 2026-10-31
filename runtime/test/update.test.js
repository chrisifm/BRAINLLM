// brain-update.js: updateReady() / applyUpdate() over a simulated ServiceWorkerContainer (cache.update: "prompt").
import { test } from 'node:test'
import assert from 'node:assert/strict'

const rt = await import('./rt.js')
const tick = () => new Promise(r => setImmediate(r))

const listeners = {}
const posted = []
let reloads = 0
const waiting = { state: 'installed', postMessage: m => posted.push(m) }
const registration = { waiting: null, installing: null, addEventListener: (t, f) => (listeners['reg:' + t] = f) }
Object.defineProperty(globalThis, 'navigator', { configurable: true, value: {
  serviceWorker: {
    controller: {},
    getRegistration: async () => registration,
    addEventListener: (t, f) => (listeners['sw:' + t] = f),
  },
} })
globalThis.location = { reload: () => void reloads++ }
const { updateReady, applyUpdate } = await rt.sibling('brain-update.js')

test('applyUpdate is inert while nothing waits', async () => {
  assert.equal(updateReady(), false)
  await tick()
  assert.equal(updateReady(), false)
  assert.equal(await applyUpdate(), false)
  assert.deepEqual(posted, [])
})

test('an installed worker makes updateReady() true; applyUpdate posts skip-waiting and reloads on takeover', async () => {
  const inst = { state: 'installing', addEventListener: (t, f) => (listeners['inst:' + t] = f) }
  registration.installing = inst
  listeners['reg:updatefound']()
  registration.waiting = waiting
  inst.state = 'installed'
  listeners['inst:statechange']()
  assert.equal(updateReady(), true)
  const done = applyUpdate()
  assert.deepEqual(posted, ['brain:skip-waiting'])
  listeners['sw:controllerchange']()
  assert.equal(await done, true)
  assert.equal(reloads, 1)
})
