// A select can only take a value its options offer: options from {#each} mount after attr() and may change later.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { install } from '../shim/dom.js'

const doc = install()
const { sig, attr } = await import('./rt.js')
// The shim's observer only queues records: deliver them as a browser would after the current task.
const observers = []
const Real = globalThis.MutationObserver
globalThis.MutationObserver = class extends Real {
  constructor(f) {
    super(f)
    observers.push(this)
  }
}
const tick = async () => {
  await new Promise(r => setTimeout(r, 0))
  for (const o of observers) if (o.takeRecords().length) o._f()
}

// The shim's elements keep any value; this one behaves like a browser select.
const select = () => {
  const el = doc.createElement('select')
  let index = -1
  const options = () => el.childNodes.filter(n => n.tagName == 'OPTION')
  Object.defineProperty(el, 'selectedIndex', { get: () => index, set: i => (index = i) })
  Object.defineProperty(el, 'value', {
    get: () => options()[index]?.getAttribute('value') ?? '',
    set: v => (index = options().findIndex(o => o.getAttribute('value') === String(v))),
  })
  doc.body.appendChild(el)
  return el
}
const add = (el, v) => {
  const o = doc.createElement('option')
  o.setAttribute('value', v)
  el.appendChild(o)
  if (el.selectedIndex < 0) el.selectedIndex = 0 // a browser selects the first option
}

test('a bound select keeps its value when the options mount after it', async () => {
  const el = select()
  attr(el, 'value', () => 'SENIOR')
  add(el, 'JUNIOR')
  add(el, 'SENIOR')
  await tick()
  assert.equal(el.value, 'SENIOR')
})

test('options arriving later still select the current value, and a state change still applies', async () => {
  const el = select()
  const v = sig('C')
  attr(el, 'value', () => v.v)
  add(el, 'A')
  await tick()
  assert.equal(el.value, 'A', 'no option matches yet: the browser choice stays')
  add(el, 'C')
  await tick()
  assert.equal(el.value, 'C')
  add(el, 'B')
  v.v = 'B'
  await tick()
  assert.equal(el.value, 'B')
})
