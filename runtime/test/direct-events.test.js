// Native non-bubbling element events execute at their target and share normal handler safety/lifecycle.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { install, Event } from '../shim/dom.js'
const doc = install()
const { on, mount, sibling } = await import('./rt.js')
const { _err } = await sibling('brain.js')

test('dialog, disclosure, validation, pointer and media events run directly and dispose with their owner', () => {
  for (const type of ['close', 'cancel', 'toggle', 'beforetoggle', 'invalid', 'pointerenter', 'pointerleave', 'play', 'pause', 'ended', 'timeupdate', 'loadedmetadata', 'waiting', 'focus', 'blur', 'load', 'error', 'scroll', 'scrollend']) {
    const host = doc.createElement('main'), parent = doc.createElement('section'), target = doc.createElement('dialog')
    doc.body.appendChild(host)
    let seen = 0, parentSeen = 0
    const stop = mount(() => {
      parent.appendChild(target)
      on(parent, type, () => parentSeen++)
      on(target, type, function (event) {
        assert.equal(this, target)
        assert.equal(event.target, target)
        assert.equal(event.currentTarget, target)
        event.preventDefault()
        seen++
      })
      return parent
    }, host)
    const event = new Event(type, { cancelable: true })
    assert.equal(target.dispatchEvent(event), false, type)
    assert.equal(seen, 1, type)
    assert.equal(parentSeen, 0, type + ' must not simulate bubbling')
    stop()
    target.dispatchEvent(new Event(type))
    assert.equal(seen, 1, type + ' must release its retained target listener on unmount')
    host.remove()
  }
})

test('direct handlers route thrown errors and rejected promises through the normal error hook', async () => {
  const errors = [], host = doc.createElement('main'), target = doc.createElement('dialog')
  doc.body.appendChild(host)
  _err((error, element, where) => errors.push([error.message, element, where]))
  const stop = mount(() => {
    on(target, 'close', () => { throw Error('synchronous proof') })
    on(target, 'cancel', async () => { throw Error('asynchronous proof') })
    return target
  }, host)
  try {
    assert.doesNotThrow(() => target.dispatchEvent(new Event('close')))
    target.dispatchEvent(new Event('cancel'))
    await new Promise(resolve => setTimeout(resolve, 0))
    assert.deepEqual(errors, [['synchronous proof', target, 'on:close'], ['asynchronous proof', target, 'on:cancel']])
  } finally { stop(); host.remove(); _err(null) }
})
