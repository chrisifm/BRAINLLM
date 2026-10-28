// #169 brain-env.js (INTERNALS §41): mounted, watch, tick, every, media, visible, debounce, dyn, portal — each cleans
// up with its component and does nothing that would keep Node alive while the server renders.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { install } from '../shim/dom.js'

const doc = install()
const rt = await import('./rt.js')
const { sig, tpl, mount } = rt
const env = await rt.sibling('brain-env.js')
const { mounted, watch, tick, every, media, visible, debounce, dyn, portal } = env
const wait = (ms = 0) => new Promise(r => setTimeout(r, ms))
const host = () => doc.body.appendChild(doc.createElement('div'))

test('mounted: runs after the nodes are in the document; its cleanup runs on unmount; skipped while the server renders', async () => {
  const log = []
  const C = () => {
    const r = tpl('<p class="m">x</p>')()
    mounted(() => (log.push('in:' + !!doc.body.querySelector('.m')), () => log.push('out')))
    return r
  }
  const stop = mount(C, host())
  assert.deepEqual(log, [])
  await wait()
  assert.deepEqual(log, ['in:true'])
  stop()
  assert.deepEqual(log, ['in:true', 'out'])
  globalThis.__brainRoute = { path: '/', params: {}, query: {} }
  const stop2 = mount(C, host())
  await wait()
  assert.deepEqual(log, ['in:true', 'out'])
  stop2()
  delete globalThis.__brainRoute
})

test('watch: the callback runs on changes only (not the first read), gets (value, old), is untracked; tick waits for the DOM', async () => {
  const a = sig(1), other = sig(0), seen = []
  const C = () => {
    watch(() => a.v, (v, o) => seen.push([v, o, other.v]))
    const r = tpl('<b><!></b>')()
    rt.insert(r, () => a.v, r.firstChild)
    return r
  }
  const h = host()
  const stop = mount(C, h)
  assert.deepEqual(seen, [])
  a.v = 2
  await tick()
  assert.deepEqual(seen, [[2, 1, 0]])
  assert.equal(h.textContent, '2')
  other.v = 5 // not tracked by the callback
  await tick()
  assert.equal(seen.length, 1)
  stop()
  a.v = 3
  await tick()
  assert.equal(seen.length, 1) // gone with the component
})

test('every: runs on an interval, pauses while the tab is hidden, stops on unmount', async () => {
  let n = 0
  const C = () => (every('20ms', () => n++), tpl('<i></i>')())
  const stop = mount(C, host())
  await wait(70)
  assert.ok(n >= 2, String(n))
  doc.hidden = true
  doc.dispatchEvent(new Event('visibilitychange'))
  const paused = n
  await wait(60)
  assert.equal(n, paused)
  doc.hidden = false
  doc.dispatchEvent(new Event('visibilitychange'))
  await wait(50)
  assert.ok(n > paused)
  stop()
  const after = n
  await wait(50)
  assert.equal(n, after)
  assert.throws(() => every('soon', () => {}), /not a duration/)
})

test('media and visible: read-only .v that follow matchMedia and IntersectionObserver, cleaned up on unmount', async () => {
  const mq = { matches: false, l: null, addEventListener: (t, f) => (mq.l = f), removeEventListener: () => (mq.l = null) }
  globalThis.matchMedia = () => mq
  let cb = null, disconnected = 0
  globalThis.IntersectionObserver = class {
    constructor(f) { cb = f }
    observe() {}
    disconnect() { disconnected++ }
  }
  let m, v
  const ref = sig(null)
  const C = () => {
    m = media('(max-width: 600px)')
    v = visible(ref)
    return tpl('<p></p>')()
  }
  const stop = mount(C, host())
  assert.equal(m.v, false)
  mq.l({ matches: true })
  assert.equal(m.v, true)
  assert.equal(cb, null) // the ref is still empty
  ref.v = doc.createElement('div')
  await tick()
  cb([{ isIntersecting: true }])
  assert.equal(v.v, true)
  assert.throws(() => (m.v = false)) // read-only
  stop()
  assert.equal(mq.l, null)
  assert.equal(disconnected, 1)
  delete globalThis.matchMedia
  delete globalThis.IntersectionObserver
})

test('debounce: .v follows the value once it has been still for the delay', async () => {
  const q = sig('a')
  const d = debounce(() => q.v, '30ms')
  assert.equal(d.v, 'a')
  q.v = 'ab'
  await wait(10)
  q.v = 'abc'
  await wait(10)
  assert.equal(d.v, 'a')
  await wait(40)
  assert.equal(d.v, 'abc')
})

test('dyn: mounts the current component, remounts only when it changes (disposing the old one), null renders nothing', async () => {
  const log = []
  const A = p => (rt.fx(() => () => log.push('A gone')), tpl(`<p>A ${p.x}</p>`)())
  const B = () => tpl('<p>B</p>')()
  const which = sig(A), x = sig(1)
  const P = () => {
    const r = tpl('<div><!></div>')()
    dyn(null, () => which.v, { x: 1 }, r.firstChild)
    return r
  }
  const h = host()
  const stop = mount(P, h)
  assert.equal(h.textContent, 'A 1')
  x.v = 2
  which.v = A // same component: nothing happens
  await tick()
  assert.deepEqual(log, [])
  which.v = B
  await tick()
  assert.equal(h.textContent, 'B')
  assert.deepEqual(log, ['A gone'])
  which.v = null
  await tick()
  assert.equal(h.textContent, '')
  which.v = A
  await tick()
  stop()
  assert.deepEqual(log, ['A gone', 'A gone'])
  assert.equal(h.textContent, '')
})

test('portal: children render into the target and leave with the component; in place while the server renders', async () => {
  const layer = doc.body.appendChild(doc.createElement('section'))
  layer.setAttribute('id', 'layer')
  const n = sig(1)
  const C = () => {
    const r = tpl('<div class="host"><!></div>')()
    rt.insert(r, portal('#layer', () => {
      const t = tpl('<em><!></em>')()
      rt.insert(t, () => n.v, t.firstChild)
      return t
    }), r.firstChild)
    return r
  }
  const h = host()
  const stop = mount(C, h)
  assert.equal(h.querySelector('em'), null)
  assert.equal(layer.textContent, '1')
  n.v = 2
  await tick()
  assert.equal(layer.textContent, '2') // still reactive under its owner
  stop()
  assert.equal(layer.textContent, '')
  globalThis.__brainSSR = []
  const h2 = host()
  const stop2 = mount(C, h2)
  assert.equal(h2.querySelector('em').textContent, '2')
  stop2()
  globalThis.__brainSSR = null
})
