import { test } from 'node:test'
import assert from 'node:assert/strict'
import { install, MouseEvent } from '../shim/dom.js'

const doc = install()
const { sig, tpl, insert, on, when, mount, batch } = await import('./rt.js')
const { dev, hmr } = await (await import('./rt.js')).sibling('brain-dev.js')
const click = el => el.dispatchEvent(new MouseEvent('click', { bubbles: true }))

// Simulates two versions of the same .brain (same hash), as the dev codegen would emit them.
const T1 = tpl('<button>v1:<!></button>'), T2 = tpl('<button>v2 [<!>]</button>')
function v1($p) {
  const n = sig($p.start, 'n'), tmp = sig('x', 'tmp')
  const r = T1()
  on(r, 'click', () => n.v++)
  insert(r, () => n.v + tmp.v.length * 0, r.lastChild)
  return r
}
function v2($p) {
  const n = sig($p.start, 'n'), extra = sig(100, 'extra')
  const r = T2()
  on(r, 'click', () => (n.v += 10))
  insert(r, () => n.v + extra.v, r.childNodes[1])
  return r
}
v1.h = v2.h = 'h1'

test('hmr: remounts in place and keeps sigs by name', () => {
  doc.body.innerHTML = '<p>before</p><p>after</p>'
  const A = dev(v1)
  const target = doc.createElement('div')
  doc.body.insertBefore(target, doc.body.lastChild)
  mount(A, target, { start: 1 })
  mount(A, target, { start: 50 })
  const [b1, b2] = target.childNodes
  click(b1)
  click(b1)
  assert.equal(b1.textContent, 'v1:3')
  assert.equal(hmr('h1', { default: dev(v2) }), 2)
  const [c1, c2] = target.childNodes
  assert.equal(c1.textContent, 'v2 [103]') // n=3 kept; `extra` is new and takes its initial value
  assert.equal(c2.textContent, 'v2 [150]')
  assert.equal(target.childNodes.length, 2)
  click(c1)
  assert.equal(c1.textContent, 'v2 [113]') // new handlers and effects; the old ones died
  assert.equal(b1.parentNode, null)
  assert.equal(hmr('other', { default: dev(v2) }), 0)
})

test('hmr: a disposed instance is no longer registered', () => {
  const A = dev(v1)
  const t = doc.createElement('div')
  const alive = hmr('h1', { default: dev(v1) }) // the ones from the previous test
  const stop = mount(A, t, { start: 0 })
  assert.equal(hmr('h1', { default: dev(v1) }), alive + 1)
  stop()
  assert.equal(hmr('h1', { default: dev(v1) }), alive)
})

test('hmr: children are rebuilt without inheriting the parent state', () => {
  const Child = $p => {
    const n = sig(7, 'n')
    const r = tpl('<i></i>')()
    insert(r, () => n.v)
    return r
  }
  Child.h = 'child'
  const C = dev(Child)
  function P1() {
    const n = sig(1, 'n')
    const r = tpl('<div>p<!></div>')()
    insert(r, C({}), r.lastChild)
    on(r, 'click', () => n.v++)
    return r
  }
  P1.h = 'parent'
  const t = doc.createElement('div')
  mount(dev(P1), t, {})
  click(t.firstChild)
  hmr('parent', { default: dev(P1) })
  assert.equal(t.textContent, 'p7') // the child keeps its own n=7, not the parent's n=2
})
