import { test } from 'node:test'
import assert from 'node:assert/strict'
import { install } from '../shim/dom.js'
const doc = install()
const { mount, sibling } = await import('./rt.js')
const { interop } = await sibling('brain-interop.js')
const flush = () => new Promise(resolve => queueMicrotask(resolve))

test('mounts after insertion, delivers pending props, updates and cleans once on unmount', async () => {
  const host = doc.createElement('div'), calls = []
  doc.body.appendChild(host)
  let control
  const stop = mount(() => {
    const el = doc.createElement('div')
    control = interop(() => el, { mount(node, props) {
      assert.equal(node.parentNode, host)
      calls.push(props.value)
      return { update(next) { calls.push(next.value) }, destroy() { calls.push('disposed') } }
    } }, { value: 1 })
    control.update({ value: 2 })
    return el
  }, host)
  await flush()
  control.update({ value: 3 })
  stop()
  control.destroy()
  assert.deepEqual(calls, [2, 3, 'disposed'])
  assert.throws(() => control.update({}), /destroyed/)
})

test('unmount before the microtask and SSR do not mount foreign code', async () => {
  let calls = 0
  const adapter = { mount() { calls++; return () => {} } }
  const stop = mount(() => { const el = doc.createElement('div'); interop(el, adapter); return el }, doc.createElement('div'))
  stop()
  globalThis.__brainSSR = true
  const stopSSR = mount(() => { const el = doc.createElement('div'); interop(el, adapter); return el }, doc.createElement('div'))
  delete globalThis.__brainSSR
  await flush()
  stopSSR()
  assert.equal(calls, 0)
})

test('adapter errors include phase and disposal remains idempotent', async () => {
  const errors = [], host = doc.createElement('div')
  let control
  const stop = mount(() => {
    const el = doc.createElement('div')
    control = interop(el, { mount() { return { update() { throw Error('update failed') }, destroy() { throw Error('cleanup failed') } } } }, {}, { onError(e, phase) { errors.push([e.message, phase]) } })
    return el
  }, host)
  await flush()
  control.update({})
  stop()
  control.destroy()
  assert.deepEqual(errors, [['update failed', 'update'], ['cleanup failed', 'destroy']])
})

test('rejects invalid and asynchronous adapters without hiding failure', async () => {
  const errors = []
  const stop = mount(() => {
    const el = doc.createElement('div')
    interop(el, { mount() { return Promise.resolve() } }, {}, { onError(error, phase) { errors.push([error.message, phase]) } })
    return el
  }, doc.createElement('div'))
  await flush()
  stop()
  assert.match(errors[0][0], /asynchronous mounts are unsupported/)
  assert.equal(errors[0][1], 'mount')
})
