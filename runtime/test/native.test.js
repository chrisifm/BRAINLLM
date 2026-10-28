// INTERNALS §19 in the core: moveBefore in list and ≤5 ms slicing of large lists.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { install } from '../shim/dom.js'

const doc = install()
const { sig, tpl, insert, list, mount } = await import('./rt.js')
const Row = tpl('<li><!></li>')
const row = s => {
  const r = Row()
  insert(r, () => s.v.t, r.firstChild)
  return r
}
const until = async (ok, ms = 3000) => {
  const t = Date.now()
  while (!ok()) {
    if (Date.now() - t > ms) throw Error('timeout')
    await new Promise(r => setTimeout(r, 1))
  }
}
const texts = ul => ul.childNodes.filter(n => n.nodeType === 1).map(n => n.textContent)

test('list: moved rows use moveBefore when the parent supports it; new rows use insertBefore', async () => {
  const ul = doc.createElement('ul')
  doc.body.appendChild(ul)
  const moved = []
  ul.moveBefore = function (n, ref) {
    moved.push(n.textContent)
    return this.insertBefore(n, ref)
  }
  const items = sig([{ id: 1, t: 'a' }, { id: 2, t: 'b' }, { id: 3, t: 'c' }])
  mount(() => (list(ul, () => items.v, x => x.id, row), []), doc.createElement('div'))
  items.v = [items.v[2], items.v[0], items.v[1], { id: 4, t: 'd' }]
  await new Promise(r => setTimeout(r, 0))
  assert.deepEqual(texts(ul), ['c', 'a', 'b', 'd'])
  assert.deepEqual(moved, ['c']) // LIS keeps a,b in place; only c moves; d is new (insertBefore)
})

test('list: a first render of a large list is built at once (no slicing: nothing on screen to keep responsive)', () => {
  const ul = doc.createElement('ul')
  const n = Array.from({ length: 3000 }, (_, i) => ({ id: i, t: String(i) }))
  mount(() => (list(ul, () => n, x => x.id, row), []), doc.createElement('div'))
  assert.equal(texts(ul).length, 3000)
})

test('list: >500 new rows added to a list already on screen render in slices, stay ordered and converge if updated mid-way', async () => {
  const ul = doc.createElement('ul')
  doc.body.appendChild(ul)
  const slow = s => {
    const t = performance.now()
    while (performance.now() - t < 0.02); // 20 µs per row → ~250 rows per 5 ms slice
    return row(s)
  }
  const mk = n => Array.from({ length: n }, (_, i) => ({ id: i, t: String(i) }))
  const items = sig(mk(3))
  mount(() => (list(ul, () => items.v, x => x.id, slow), []), doc.createElement('div'))
  items.v = mk(3000)
  await Promise.resolve()
  const first = texts(ul).length
  assert.ok(first > 0 && first < 3000, `first slice ${first}`)
  await new Promise(r => setTimeout(r, 2))
  items.v = mk(3000).reverse() // update while slicing
  await until(() => texts(ul).length === 3000)
  assert.deepEqual(texts(ul), mk(3000).reverse().map(x => x.t))
})
