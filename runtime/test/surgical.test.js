// Surgical updates (INTERNALS §41, QA encapsulation probe): changing one field of one row touches only that field's node.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { install, MutationObserver } from '../shim/dom.js'

const doc = install()
const { sig, tpl, insert, attr, cls, list, mount } = await import('./rt.js')
const tick = () => new Promise(r => setTimeout(r, 0))

test('one field of one row in a 200-row list → exactly one DOM mutation (no same-value rewrites)', async () => {
  const T = tpl('<li><span class="lbl"><!></span><b><!></b></li>')
  const rows = sig(Array.from({ length: 200 }, (_, i) => ({ id: i, label: 'row ' + i, n: i, on: i % 2 === 0 })))
  const ul = doc.createElement('ul')
  doc.body.appendChild(ul)
  mount(() => {
    list(ul, () => rows.v, r => r.id, r => {
      const li = T(), lbl = li.firstChild, b = li.lastChild
      attr(li, 'data-id', () => r.v.id)
      cls(li, 'on', () => r.v.on)
      insert(lbl, () => r.v.label, lbl.firstChild)
      insert(b, () => r.v.n, b.firstChild)
      return li
    })
    return []
  }, doc.createElement('div'))
  const seen = []
  const mo = new MutationObserver(rs => seen.push(...rs))
  mo.observe(ul, { subtree: true, childList: true, attributes: true, characterData: true })
  rows.v = rows.v.map(r => (r.id === 42 ? { ...r, n: 4200 } : r)) // a new object for row 42, one field changed
  await tick()
  await tick()
  seen.push(...mo.takeRecords())
  mo.disconnect()
  assert.equal(seen.length, 1, seen.map(m => m.type + ':' + (m.target.nodeName || '')).join(', '))
  assert.equal(ul.childNodes[42].lastChild.textContent, '4200')
})
