// #198 brain-why.js (INTERNALS §43): which signal write caused each DOM update (component, field, file:line, drv
// chain), and how many mutations each user interaction causes. Dev only.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { install, MouseEvent } from '../shim/dom.js'

const doc = install()
const rt = await import('./rt.js')
const { sig, drv, tpl, insert, attr, on, mount } = rt
const { dev } = await rt.sibling('brain-dev.js')
const { why, interactions } = await rt.sibling('brain-why.js')
const { store } = await rt.sibling('brain-ch.js')
const tick = () => new Promise(r => setTimeout(r, 5))

test('why(): the text that changed points to the signal write (name, component, test file line) through the drv', async () => {
  let count
  const Counter = () => {
    count = sig(1, 'count')
    const double = drv(() => count.v * 2)
    const r = tpl('<div><b class="out"><!></b><button class="inc">+</button></div>')()
    insert(r.firstChild, () => double.v, r.firstChild.firstChild)
    attr(r.firstChild, 'data-n', () => count.v)
    on(r.lastChild, 'click', () => (count.v = count.v + 1))
    return r
  }
  Counter.f = 'views/counter.brain'
  const h = doc.body.appendChild(doc.createElement('div'))
  const stop = mount(dev(Counter), h)
  assert.deepEqual(await why('.out'), []) // the first render is not an update
  count.v = 5 // the write the answer must point to
  await tick()
  assert.equal(h.querySelector('.out').textContent, '10')
  const recs = await why('.out')
  const text = recs.find(x => x.kind === 'text'), at = recs.find(x => x.kind === 'attr:data-n')
  assert.ok(text && at, recs.map(x => x.kind).join())
  const c = text.causes[0]
  assert.deepEqual([c.signal, c.comp, c.from, c.to, c.via], ['count', 'views/counter.brain', 1, 5, ['drv']]) // through `double`
  const line = s => s.split('\n').findIndex(l => l.includes('count.v = 5 // the write')) + 1
  assert.equal(c.src.split(':').pop(), String(line(readFileSync(new URL(import.meta.url), 'utf8')))) // the very line
  assert.deepEqual(at.causes[0].via, []) // the attribute reads count directly
  stop()
})

test('store fields are named (cart.total) and an interaction reports how many mutations it caused', async () => {
  const cart = store({ total: 0, items: [] }, {}, 'cart')
  const View = () => {
    const r = tpl('<p><span class="t"><!></span><i class="n"><!></i><button class="add">add</button></p>')()
    insert(r.firstChild, () => cart.total, r.firstChild.firstChild)
    insert(r.childNodes[1], () => cart.items.length, r.childNodes[1].firstChild)
    on(r.lastChild, 'click', () => cart.$set({ total: cart.total + 3, items: [...cart.items, 'x'] }))
    return r
  }
  View.f = 'views/cart.brain'
  const h = doc.body.appendChild(doc.createElement('div'))
  const stop = mount(dev(View), h)
  h.querySelector('.add').dispatchEvent(new MouseEvent('click', { bubbles: true, cancelable: true }))
  await tick()
  const [r] = await why('.t')
  assert.equal(r.causes[0].signal, 'cart.total')
  const [last] = interactions()
  assert.deepEqual([last.type, last.target], ['click', 'button.add'])
  assert.ok(last.mutations >= 2, JSON.stringify(last)) // the total and the count
  assert.ok(globalThis.__brainWhy && typeof globalThis.__brainWhy.why === 'function')
  assert.deepEqual(await why('#nope'), { error: 'no element matches #nope' })
  stop()
})

test('mounts(): the last component mounts with their mount time, HMR remounts flagged', async () => {
  const { mounts } = await rt.sibling('brain-why.js')
  const { hmr } = await rt.sibling('brain-dev.js')
  const Card = () => tpl('<p class="card">card</p>')()
  Card.f = 'comp/card.brain'
  Card.h = 'cardh1'
  const h = doc.body.appendChild(doc.createElement('div'))
  const stop = mount(dev(Card), h)
  const [m] = mounts()
  assert.deepEqual([m.comp, m.hmr, typeof m.ms, typeof m.at], ['comp/card.brain', false, 'number', 'number'])
  const Card2 = () => tpl('<p class="card">card v2</p>')()
  Object.assign(Card2, { f: 'comp/card.brain', h: 'cardh1' })
  assert.equal(hmr('cardh1', { default: dev(Card2) }), 1)
  assert.deepEqual([mounts()[0].comp, mounts()[0].hmr], ['comp/card.brain', true])
  assert.ok(globalThis.__brainWhy.mounts().length >= 2)
  stop()
})
