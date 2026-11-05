import { test } from 'node:test'
import assert from 'node:assert/strict'
import { install, Event, MouseEvent } from '../shim/dom.js'
const NativeEventTarget = globalThis.EventTarget

test('installing the SSR DOM keeps events compatible with native Node EventTargets', () => {
  install()
  const target = new NativeEventTarget()
  let seen = false
  target.addEventListener('proof', event => {
    seen = true
    assert.equal(event.target, target)
    assert.equal(event.currentTarget, target)
    assert.deepEqual(event.composedPath(), [target])
    event.preventDefault()
  })
  const event = new globalThis.Event('proof', { cancelable: true })
  assert.equal(target.dispatchEvent(event), false)
  assert.ok(seen && event.defaultPrevented)
})

test('shim dispatch preserves bubbling, cancellation and immediate-stop with native-compatible events', () => {
  const doc = install(), parent = doc.createElement('div'), child = doc.createElement('button')
  parent.appendChild(child); doc.body.appendChild(parent)
  const seen = []
  parent.addEventListener('click', event => { seen.push('parent'); assert.equal(event.target, child) })
  child.addEventListener('click', event => { seen.push('first'); event.preventDefault(); event.stopImmediatePropagation() })
  child.addEventListener('click', () => seen.push('second'))
  const event = new MouseEvent('click', { bubbles: true, cancelable: true })
  assert.equal(child.dispatchEvent(event), false)
  assert.deepEqual(seen, ['first'])
  assert.equal(event.composedPath()[0], child)
  const plain = new Event('plain')
  plain.preventDefault()
  assert.equal(plain.defaultPrevented, false, 'non-cancelable events follow native semantics')
})
