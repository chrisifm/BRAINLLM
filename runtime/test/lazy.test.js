// brain-lazy.js (INTERNALS §22): visible (IntersectionObserver), idle, interaction; owner context and disposal.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { install, MouseEvent, Event } from '../shim/dom.js'

const doc = install()
const rt = await import('./rt.js')
const { sig, tpl, insert, when, mount, sibling } = rt
const { lazy } = await sibling('brain-lazy.js')
const { key, provide, use, scope } = await sibling('brain-ch.js')
const tick = () => new Promise(r => setTimeout(r, 0))

// fake observers / idle callbacks, fired by hand
let ios = [], idles = []
globalThis.IntersectionObserver = class {
  constructor(f, o) {
    this.f = f
    this.o = o
    this.on = true
    ios.push(this)
  }
  observe(el) {
    this.el = el
  }
  disconnect() {
    this.on = false
  }
}
globalThis.requestIdleCallback = (f, o) => idles.push({ f, o, on: true }) - 1
globalThis.cancelIdleCallback = h => (idles[h].on = false)

const Chart = $p => {
  const r = tpl('<b>chart </b>')()
  insert(r, () => $p.n, null)
  return r
}
const host = () => doc.createElement('div')
const hole = () => {
  const el = doc.createElement('div')
  el.textContent = 'loading'
  return el
}

test('visible: imports when the placeholder intersects, replaces it, props stay reactive', async () => {
  ios = []
  const n = sig(1)
  let imports = 0
  const h = host()
  mount(() => lazy(hole(), async () => (imports++, { default: Chart }), undefined, { get n() { return n.v } }), h)
  assert.equal(h.textContent, 'loading')
  assert.equal(ios[0].o.rootMargin, '200px')
  ios[0].f([{ isIntersecting: false }])
  await tick()
  assert.equal(imports, 0)
  ios[0].f([{ isIntersecting: true }])
  await tick()
  assert.equal(imports, 1)
  assert.equal(ios[0].on, false)
  assert.equal(h.textContent, 'chart 1')
  n.v = 2
  await tick()
  assert.equal(h.textContent, 'chart 2')
})

test('idle: requestIdleCallback with a 2 s timeout; the component sees the context of its owner', async () => {
  idles = []
  const K = key('k')
  const C = () => doc.createTextNode('ctx=' + use(K))
  const h = host()
  mount(scope(() => (provide(K, 'yes'), lazy(hole(), () => C, 'idle'))), h)
  assert.equal(idles[0].o.timeout, 2000)
  idles[0].f()
  await tick()
  assert.equal(h.textContent, 'ctx=yes')
})

test('interaction: loads on intent (pointerover) and replays a click that landed on the placeholder', async () => {
  const clicks = []
  const Btn = () => {
    const b = doc.createElement('button')
    b.addEventListener('click', e => clicks.push(e.type))
    b.textContent = 'real'
    return b
  }
  const h = host()
  let p
  mount(() => (p = lazy(hole(), async () => Btn, 'interaction')), h)
  p.dispatchEvent(new MouseEvent('click', { bubbles: true, cancelable: true }))
  await tick()
  await tick()
  assert.equal(h.textContent, 'real')
  assert.deepEqual(clicks, ['click'])
  const h2 = host()
  let q
  mount(() => (q = lazy(hole(), async () => Btn, 'interaction')), h2)
  q.dispatchEvent(new Event('pointerover'))
  await tick()
  assert.equal(h2.textContent, 'real')
  assert.equal(clicks.length, 1) // no phantom click
})

test('disposal: before the trigger cancels it; after, removes the component nodes; import failure keeps placeholder', async () => {
  ios = []
  idles = []
  const show = sig(true)
  let imports = 0
  const h = host()
  mount(() => {
    const r = tpl('<section><!></section>')()
    when(r, () => show.v, () => lazy(hole(), async () => (imports++, Chart), undefined, { n: 'x' }), null, r.firstChild)
    return r
  }, h)
  show.v = false
  await tick()
  assert.equal(ios[0].on, false)
  ios[0].f([{ isIntersecting: true }])
  await tick()
  assert.equal(imports, 0)
  show.v = true
  await tick()
  ios[1].f([{ isIntersecting: true }])
  await tick()
  assert.equal(h.textContent, 'chart x')
  show.v = false
  await tick()
  assert.equal(h.textContent, '')
  // failure
  const errs = []
  globalThis.reportError = e => errs.push(e.message)
  const h2 = host()
  let el
  mount(() => (el = lazy(hole(), async () => { throw Error('chunk 404') }, 'idle')), h2)
  idles.at(-1).f()
  await tick()
  await tick()
  assert.deepEqual(errs, ['chunk 404'])
  assert.equal(h2.textContent, 'loading')
  assert.equal(el.$l, null) // retry possible
  delete globalThis.reportError
})

test('#131 a lazy chunk deleted by a deploy reloads the page once; afterwards it is reported and retried', async () => {
  idles = []
  const ss = new Map()
  globalThis.sessionStorage = { getItem: k => ss.get(k) ?? null, setItem: (k, v) => ss.set(k, String(v)) }
  let reloads = 0
  const loc = globalThis.location
  globalThis.location = { reload: () => reloads++ }
  const errs = []
  globalThis.reportError = e => errs.push(e.message)
  const gone = async () => { throw new TypeError('Failed to fetch dynamically imported module') }
  mount(() => lazy(hole(), gone, 'idle'), host())
  idles.at(-1).f()
  await tick()
  await tick()
  assert.equal(reloads, 1)
  assert.deepEqual(errs, [])
  mount(() => lazy(hole(), gone, 'idle'), host()) // after the reload, the same build still misses it
  idles.at(-1).f()
  await tick()
  await tick()
  assert.equal(reloads, 1) // no loop
  assert.equal(errs.length, 1)
  globalThis.location = loc
  delete globalThis.reportError
})
