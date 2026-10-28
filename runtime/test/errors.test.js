// INTERNALS §18: native capture (fx, drv, handlers, render), per-component fallback, B0902 cutoff and brain-err.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { install, Event, MouseEvent } from '../shim/dom.js'

const doc = install()
const beacons = []
globalThis.navigator.sendBeacon = (url, body) => (beacons.push({ url, r: JSON.parse(body) }), true)
globalThis.fetch = async () => ({ status: 500 })
const rt = await import('./rt.js')
const { sig, fx, tpl, insert, on, mount, sibling } = rt
const E = await sibling('brain-err.js')
const { fallback } = E
const errors = E.capture({ report: '/_err', build: 'b42', log: false, max: 3 })
const tick = () => new Promise(r => setTimeout(r, 0))
const click = el => el.dispatchEvent(new MouseEvent('click', { bubbles: true }))
const last = () => errors[errors.length - 1]

const T = tpl('<div class="ok"><button>+</button><!></div>')
// Component that fails depending on `mode`: in the fx on reaching 2, in the handler or in the render.
function Fragile($p) {
  const n = sig(0)
  if ($p.mode === 'render') throw Error('render broken')
  const r = T()
  on(r.firstChild, 'click', () => {
    if ($p.mode === 'handler') throw TypeError('handler broken')
    n.v++
  })
  fx(() => {
    if (n.v === 2) throw RangeError('fx broken')
  })
  insert(r, () => n.v, r.lastChild)
  return r
}
Fragile.h = 'frg'
const Safe = fallback(Fragile, 'Could not load')

function Neighbor() {
  const n = sig(0)
  const r = tpl('<p><b>v</b><!></p>')()
  on(r.firstChild, 'click', () => n.v++)
  insert(r, () => n.v, r.lastChild)
  return r
}

test('failing fx: the component shows its fallback and the neighbor stays alive', async () => {
  const host = doc.createElement('main')
  doc.body.appendChild(host)
  mount(() => [Safe({ mode: 'fx' }), Neighbor()], host)
  const btn = host.firstChild.firstChild
  click(btn)
  await tick()
  assert.equal(host.firstChild.getAttribute('class'), 'ok')
  click(btn)
  await tick()
  const fb = host.firstChild
  assert.equal(fb.nodeName, 'P')
  assert.equal(fb.getAttribute('role'), 'alert')
  assert.equal(fb.textContent, 'Could not load')
  const r = last()
  assert.equal(r.type, 'RangeError')
  assert.equal(r.where, 'fx')
  assert.equal(r.build, 'b42')
  assert.equal(r.crumbs.filter(c => c.k === 'click' && c.d === 'button "+"').length, 2, JSON.stringify(r.crumbs))
  assert.ok(r.crumbs.some(c => c.k === 'sig'))
  const neighbor = host.lastChild
  click(neighbor.firstChild)
  await tick()
  assert.equal(neighbor.textContent, 'v1')
})

test('failing handler (§42): captured as on:<event>, the component stays mounted and interactive', async () => {
  const host = doc.createElement('section')
  doc.body.appendChild(host)
  mount(() => Safe({ mode: 'handler' }), host)
  const btn = host.firstChild.firstChild
  click(btn)
  await tick()
  assert.equal(last().where, 'on:click')
  assert.equal(last().msg, 'handler broken')
  assert.notEqual(host.firstChild.textContent, 'Could not load')
  assert.equal(host.firstChild.firstChild, btn) // same nodes, still there
  click(btn)
  await tick()
  assert.equal(last().n, 2) // still interactive: the second click runs (and fails) again
})

test('failing render: direct fallback, no half-built nodes', () => {
  const host = doc.createElement('section')
  doc.body.appendChild(host)
  mount(() => Safe({ mode: 'render' }), host)
  assert.equal(host.childNodes.length, 1)
  assert.equal(host.firstChild.textContent, 'Could not load')
  assert.equal(last().where, 'render')
})

test('no fallback: the error is reported and the app keeps running', async () => {
  const host = doc.createElement('section')
  doc.body.appendChild(host)
  mount(() => Fragile({ mode: 'fx' }), host)
  const b = host.firstChild.firstChild
  click(b), click(b), click(b)
  await tick()
  assert.equal(host.firstChild.textContent, '+3') // the broken fx does not take down the sibling insert
})

test('B0902: a sig→fx→sig cycle is cut, reported once and the other effects keep running', async () => {
  const a = sig(0), b = sig(0), c = sig(0), log = []
  fx(() => (b.v = a.v + 1))
  fx(() => (a.v = b.v + 1))
  fx(() => log.push(c.v))
  await tick() // the cycle runs in the microtask flush
  const r = errors.filter(e => e.code === 'B0902')
  assert.equal(r.length, 1)
  assert.equal(r[0].where, 'B0902')
  c.v = 1
  await tick()
  assert.deepEqual(log, [0, 1])
})

test('dedupe by fingerprint and sendBeacon send cap', async () => {
  const host = doc.createElement('section')
  doc.body.appendChild(host)
  const before = errors.length
  for (let i = 0; i < 3; i++) mount(() => Safe({ mode: 'render' }), host)
  assert.equal(errors.length, before) // same fingerprint: no new records
  assert.ok(errors.find(e => e.where === 'render').n >= 4)
  assert.ok(beacons.length <= 3 && beacons.every(b => b.url === '/_err' && b.r.fp))
})

test('breadcrumbs without values: no inputs, query strings or signals', async () => {
  const inp = doc.createElement('input')
  inp.setAttribute('aria-label', 'Password')
  doc.body.appendChild(inp)
  inp.value = 'secret123'
  inp.dispatchEvent(new Event('input', { bubbles: true }))
  await fetch('/api/users?token=abc', { method: 'POST' })
  const s = sig('private')
  s.v = 'other-private'
  let r
  fx(() => {
    if (s.v === 'other-private') throw Error('to read crumbs')
  })
  r = last()
  const txt = JSON.stringify(r.crumbs)
  assert.ok(txt.includes('input[Password]'), txt)
  assert.ok(txt.includes('POST /api/users → 500'), txt)
  for (const no of ['secret123', 'token', 'abc', 'private']) assert.ok(!txt.includes(no), no)
})

test("a cancelled request (kind 'aborted') never becomes a brain_errors entry", async () => {
  const n = errors.length
  const { sig, fx } = rt
  const k = sig(0)
  const stop = fx(() => {
    if (k.v) throw Object.assign(new Error('The request was cancelled.'), { kind: 'aborted' })
  })
  k.v = 1
  await new Promise(r => setTimeout(r, 0))
  assert.equal(errors.length, n)
  stop()
})
