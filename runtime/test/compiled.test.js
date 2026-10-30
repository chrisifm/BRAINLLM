// Components from the REAL compiler output (fixtures/compiled, regen.sh), not hand-written.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { mkdtempSync, readFileSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { pathToFileURL } from 'node:url'
import { install, Event, MouseEvent } from '../shim/dom.js'

const doc = install()
const { mount, base, sibling } = await import('./rt.js')
const { hmr, live } = await sibling('brain-dev.js')
const dir = mkdtempSync(join(tmpdir(), 'brain-compiled-'))
let seq = 0
const load = async name => {
  const src = readFileSync(new URL(`./fixtures/compiled/${name}`, import.meta.url), 'utf8').replaceAll('__RT__/', base)
  const f = join(dir, `${seq++}-${name}`)
  writeFileSync(f, src)
  return import(pathToFileURL(f).href)
}
const click = el => el.dispatchEvent(new MouseEvent('click', { bubbles: true }))
const flush = () => new Promise(r => queueMicrotask(r))
const log = console.log
console.log = () => {} // the example counter does fx(() => log(n))

test('compiled counter: props, insert, drv, cls and delegated click', async t => {
  t.after(() => (console.log = log))
  const { default: Counter } = await load('counter.brain.js')
  const host = doc.createElement('div')
  doc.body.appendChild(host)
  const stop = mount(Counter, host, { start: 3, label: 'Taps' })
  const b = host.firstChild
  assert.equal(b.textContent, 'Taps: 3 × 2 = 6')
  assert.equal(b.getAttribute('data-b'), 'rs62nv')
  for (let i = 0; i < 8; i++) click(b)
  await flush()
  assert.equal(b.textContent, 'Taps: 11 × 2 = 22')
  assert.ok(b.classList.contains('hot'))
  stop()
  assert.equal(host.childNodes.length, 0)
})

test('compiled counter in dev: hmr changes the text and keeps n', async () => {
  const { default: v1 } = await load('counter.dev.js')
  const host = doc.createElement('div')
  doc.body.appendChild(host)
  const stop = mount(v1, host, {})
  click(host.firstChild)
  click(host.firstChild)
  await flush()
  assert.equal(host.firstChild.textContent, 'Clicks: 2 × 2 = 4')
  assert.equal(hmr('rs62nv', await load('counter.dev2.js')), 1)
  assert.equal(host.firstChild.textContent, 'Clicks: 2 double = 4')
  click(host.firstChild)
  await flush()
  assert.equal(host.firstChild.textContent, 'Clicks: 3 double = 6')
  stop()
  assert.equal(live.has('rs62nv'), false, 'disposed component hashes do not retain empty registry sets')
  assert.equal(host.childNodes.length, 0, 'disposing the mount removes the current HMR roots, including a remounted version')
})

test('compiled todo: submit, keyed list, toggle, filters and empty', async () => {
  const { default: Todo } = await load('todo.brain.js')
  const host = doc.createElement('div')
  doc.body.appendChild(host)
  const stop = mount(Todo, host, {})
  const sec = host.firstChild
  const form = sec.childNodes[2]
  const inp = form.firstChild, btn = form.lastChild, ul = form.nextSibling.nextSibling
  const items = () => ul.childNodes.filter(n => n.nodeType === 1)
  assert.equal(items()[0].textContent, 'Nothing here.')
  assert.equal(btn.hasAttribute('disabled'), true)
  const add = async t => {
    inp.value = t
    inp.dispatchEvent(new Event('input', { bubbles: true }))
    await flush()
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }))
    await flush()
  }
  await add('bread')
  await add('milk')
  assert.deepEqual(items().map(li => li.textContent.replace('×', '').trim()), ['bread', 'milk'])
  assert.equal(inp.value, '') // value is a property, not an attribute
  const footer = ul.nextSibling.nextSibling
  assert.equal(footer.firstChild.textContent, '2 left')
  const milk = items()[1]
  milk.firstChild.dispatchEvent(new Event('change', { bubbles: true }))
  await flush()
  assert.equal(footer.firstChild.textContent, '1 left')
  assert.ok(items()[1].classList.contains('done'))
  assert.equal(items()[1], milk) // keyed: same <li>
  const [, open, done] = footer.childNodes.filter(n => n.nodeName === 'BUTTON')
  click(done)
  await flush()
  assert.deepEqual(items().map(li => li.textContent.replace('×', '').trim()), ['milk'])
  assert.ok(done.classList.contains('on'))
  click(open)
  await flush()
  click(items()[0].lastChild) // delete "bread"
  await flush()
  assert.equal(items()[0].textContent, 'Nothing here.')
  stop()
})
