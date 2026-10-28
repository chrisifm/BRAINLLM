// #169 brain-anim.js: enter animation once connected, exit animation awaited by the core before removal (when/list),
// nothing under prefers-reduced-motion.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { install } from '../shim/dom.js'

const doc = install()
const rt = await import('./rt.js')
const { sig, tpl, when, mount } = rt
const { transition } = await rt.sibling('brain-anim.js')
const wait = (ms = 0) => new Promise(r => setTimeout(r, ms))

test('transition: enter plays when inserted, exit is awaited before the node leaves (when)', async () => {
  const played = []
  let finish
  const show = sig(true)
  const C = () => {
    const r = tpl('<div><!></div>')()
    when(null, () => show.v, () => {
      const p = tpl('<p class="t">hi</p>')()
      p.animate = (k, o) => {
        played.push([k[0].opacity, o.duration, o.fill || ''])
        return { finished: new Promise(r => (finish = r)) }
      }
      transition(p, 'fade', { ms: 120 })
      return p
    }, null, r.firstChild)
    return r
  }
  const h = doc.body.appendChild(doc.createElement('div'))
  const stop = mount(C, h)
  await wait()
  assert.deepEqual(played, [[0, 120, '']]) // enter: from transparent
  show.v = false
  await wait()
  assert.deepEqual(played[1], [1, 120, 'forwards']) // exit: to transparent
  assert.ok(h.querySelector('.t'), 'still there while the exit plays')
  finish()
  await wait()
  assert.equal(h.querySelector('.t'), null)
  stop()
  assert.throws(() => transition(doc.createElement('p'), 'spin'), /unknown kind/)
})

test('transition: nothing with prefers-reduced-motion or without the Web Animations API', () => {
  const p = doc.createElement('p')
  transition(p, 'slide') // no el.animate in the shim: a no-op
  assert.equal(p.$out, undefined)
  globalThis.matchMedia = () => ({ matches: true })
  const q = doc.createElement('p')
  q.animate = () => assert.fail('animated under reduced motion')
  transition(q, 'scale')
  assert.equal(q.$out, undefined)
  delete globalThis.matchMedia
})
