// #267 brain-overlay.js (INTERNALS §20): errors panel, warnings chip, "Copy for the LLM", per-file replacement.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { install, MouseEvent } from '../shim/dom.js'

const doc = install()
const rt = await import('./rt.js')
const O = await rt.sibling('brain-overlay.js')
const box = () => doc.body.querySelector('brain-overlay')
const view = () => { const h = box(); return h && (h.shadowRoot || h) }
const click = el => el.dispatchEvent(new MouseEvent('click', { bubbles: true, cancelable: true }))
const err = { code: 'B0113', file: 'comp/inner.brain', line: 5, col: 3, level: 'error', msg: '`hits.push()` mutates the sig in place', fix: 'assign a new array: `hits = [...hits, x]`', snippet: { line: 3, text: ['let hits = sig([])', 'const add = (x) => {', '  hits.push(x)', '}', ''] } }
const warn = { code: 'B1308', file: 'comp/inner.brain', line: 9, col: 1, level: 'warning', msg: '`console.log` in a component', fix: 'use `log`' }

test('an error opens the panel with code, location, the marked snippet line and the fix; nothing covers the page', () => {
  O.show([err], { file: 'comp/inner.brain' })
  const v = view()
  assert.ok(v.querySelector('.panel'))
  assert.match(v.querySelector('.t').textContent, /B0113 comp\/inner\.brain:5:3/)
  assert.match(v.querySelector('.snip .hit').textContent, /^ {3}5 │ {3}hits\.push\(x\)$/)
  assert.match(v.querySelector('.fix').textContent, /fix: assign a new array/)
  assert.equal(v.querySelector('.chip'), null)
})

test('Copy for the LLM: location, message, numbered snippet with the line marked, fix and the file JSON', () => {
  click(view().querySelector('[data-copy="0"]'))
  const t = globalThis.__brainOverlayCopied
  assert.match(t, /^B0113 comp\/inner\.brain:5:3 \(error\)\n`hits\.push\(\)` mutates/)
  assert.match(t, /\n> {4}5 \|   hits\.push\(x\)\n/)
  assert.match(t, /\nfix: assign a new array/)
  assert.match(t, /brain check --json:\n\[\{"code":"B0113"/)
  assert.doesNotMatch(t, /"snippet"/) // the JSON is the diagnostics only
})

test('warnings only: a chip that opens the list; the file compiling again removes everything', () => {
  O.show([warn], { file: 'comp/inner.brain' }) // replaces the file's diagnostics: the error is fixed
  let v = view()
  assert.equal(v.querySelector('.panel'), null)
  assert.equal(v.querySelector('.chip').textContent, '1 warning')
  click(v.querySelector('.chip'))
  v = view()
  assert.match(v.querySelector('.panel').textContent, /B1308/)
  O.show([], 'comp/inner.brain') // the dev client passes the file as a string
  assert.equal(box(), null)
})

test('runtime errors (brain-err records) join the panel, deduplicated; clear() removes all', () => {
  O.error({ code: 'TypeError', msg: 'x is undefined', src: 'comp/list.brain:12', brainStack: ['comp/list.brain:12', 'views/home.brain:4'] })
  O.error({ code: 'TypeError', msg: 'x is undefined', src: 'comp/list.brain:12' })
  const v = view()
  assert.equal(v.querySelectorAll('li').length, 1)
  click(v.querySelector('[data-copy="0"]'))
  assert.match(globalThis.__brainOverlayCopied, /TypeError comp\/list\.brain:12 \(error\)[\s\S]*stack:\ncomp\/list\.brain:12\nviews\/home\.brain:4/)
  O.clear()
  assert.equal(box(), null)
})
