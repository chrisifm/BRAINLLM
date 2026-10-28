// Runs compiled .brain modules against runtime/brain.js on the runtime's DOM shim.
// Usage: node e2e.mjs <dir-with-compiled-modules> <runtime/shim/dom.js>
import assert from 'node:assert/strict'
import { pathToFileURL } from 'node:url'
import { join } from 'node:path'

const [dir, shim] = process.argv.slice(2)
const { install, Event, MouseEvent } = await import(pathToFileURL(shim).href)
const doc = install()
const load = async name => (await import(pathToFileURL(join(dir, name + '.brain.js')).href)).default
const { mount } = await import(process.env.BRAIN_RUNTIME_URL)

const click = el => el.dispatchEvent(new MouseEvent('click', { bubbles: true }))
const fire = (el, type) => el.dispatchEvent(new Event(type, { bubbles: true, cancelable: true }))
const text = el => el.textContent.replace(/\s+/g, ' ').trim()
// The shim only supports [attr] selectors: walk the tree by tag name instead.
const all = (root, tag) => {
  const out = []
  const walk = n => {
    for (const c of n.childNodes) {
      if (c.nodeType === 1 && (c.tagName || c.nodeName).toLowerCase() === tag) out.push(c)
      walk(c)
    }
  }
  walk(root)
  return out
}

// ── counter ──
{
  const Counter = await load('comp/counter')
  assert.equal(typeof Counter.h, 'string')
  const host = doc.createElement('div')
  doc.body.appendChild(host)
  mount(Counter, host, { start: 9 })
  const btn = all(host, 'button')[0]
  assert.equal(btn.getAttribute('data-b'), Counter.h)
  assert.equal(text(btn), 'Clicks: 9 × 2 = 18')
  click(btn)
  assert.equal(text(btn), 'Clicks: 10 × 2 = 20')
  assert.equal(btn.classList.contains('hot'), false)
  click(btn)
  assert.equal(text(btn), 'Clicks: 11 × 2 = 22')
  assert.equal(btn.classList.contains('hot'), true)
  console.log('ok counter')
}

// ── todo ──
{
  const Todo = await load('comp/todo')
  const host = doc.createElement('div')
  doc.body.appendChild(host)
  mount(Todo, host, {})
  const [input] = all(host, 'input')
  const form = all(host, 'form')[0]
  const add = all(host, 'button')[0]
  assert.equal(text(all(host, 'h2')[0]), 'To do')
  assert.equal(add.hasAttribute('disabled'), true)
  assert.equal(text(all(host, 'ul')[0]), 'Nothing here.')
  for (const t of ['Milk', 'Bread']) {
    input.value = t
    fire(input, 'input')
    assert.equal(add.hasAttribute('disabled'), false)
    const ev = new Event('submit', { bubbles: true, cancelable: true })
    form.dispatchEvent(ev)
    assert.equal(ev.defaultPrevented, true)
  }
  let lis = all(all(host, 'ul')[0], 'li')
  assert.equal(lis.length, 2)
  assert.equal(input.value, '')
  assert.match(text(all(host, 'footer')[0]), /^2 left/)
  // toggle the first item: same key, new object → row updated in place
  const first = lis[0]
  const box = all(first, 'input')[0]
  fire(box, 'change')
  assert.equal(first.classList.contains('done'), true)
  assert.equal(all(all(host, 'ul')[0], 'li')[0], first)
  assert.match(text(all(host, 'footer')[0]), /^1 left/)
  // filters
  const [bAll, bOpen, bDone] = all(all(host, 'footer')[0], 'button')
  click(bDone)
  assert.deepEqual(all(all(host, 'ul')[0], 'li').map(text), ['Milk ×'])
  assert.equal(bDone.classList.contains('on'), true)
  click(bOpen)
  assert.deepEqual(all(all(host, 'ul')[0], 'li').map(text), ['Bread ×'])
  click(bAll)
  // remove both
  for (let i = 0; i < 2; i++) click(all(all(all(host, 'ul')[0], 'li')[0], 'button')[0])
  assert.equal(text(all(host, 'ul')[0]), 'Nothing here.')
  console.log('ok todo')
}

// ── dashboard: child components, {#if}, emit, load()/props, async handlers ──
{
  const mod = await import(pathToFileURL(join(dir, 'views/dashboard.brain.js')).href)
  assert.equal(mod.load, undefined, 'load() must not reach the client')
  const server = await import(pathToFileURL(join(dir, 'views/dashboard.brain.server.js')).href)
  const data = await server.load({})
  const host = doc.createElement('div')
  doc.body.appendChild(host)
  mount(mod.default, host, data)
  const rows = () => all(all(host, 'tbody')[0], 'tr')
  assert.equal(rows().length, 2)
  assert.deepEqual(rows().map(r => text(all(r, 'td')[2])), ['Active', 'Inactive'])
  assert.equal(all(rows()[0], 'span')[0].getAttribute('data-tone'), 'ok')
  assert.equal(text(all(host, 'header')[0]).includes('1 active of 2'), true)
  // search
  const search = all(all(host, 'header')[0], 'input')[0]
  search.value = 'turing'
  fire(search, 'input')
  assert.deepEqual(rows().map(r => text(all(r, 'td')[0])), ['Alan Turing'])
  search.value = 'zzz'
  fire(search, 'input')
  assert.equal(text(rows()[0]), 'No results for this search.')
  search.value = ''
  fire(search, 'input')
  // edit → UserForm appears with the user's data, save emits back to the view
  click(all(rows()[1], 'button')[0])
  const form = all(host, 'form')[0]
  assert.ok(form, 'UserForm did not appear')
  const [name, , active] = all(form, 'input')
  assert.equal(name.value, 'Alan Turing')
  assert.equal(active.checked, false)
  active.checked = true
  fire(active, 'change')
  form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }))
  await new Promise(r => setTimeout(r, 150))
  assert.equal(all(host, 'form').length, 0, 'the form should close after saving')
  assert.deepEqual(rows().map(r => text(all(r, 'td')[2])), ['Active', 'Active'])
  // cancel via emit without data
  click(all(all(host, 'header')[0], 'button')[0])
  assert.equal(all(host, 'form').length, 1)
  click(all(all(host, 'form')[0], 'button')[1])
  assert.equal(all(host, 'form').length, 0)
  // async remove
  click(all(rows()[0], 'button')[1])
  await new Promise(r => setTimeout(r, 150))
  assert.equal(rows().length, 1)
  console.log('ok dashboard')
}

// ── dev build: dev() wrapper + sig names; hmr() swaps the module and keeps `n` ──
{
  const { hmr } = await import(process.env.BRAIN_DEV_URL)
  const v1 = await import(pathToFileURL(join(dir, 'counter.dev.js')).href)
  const host = doc.createElement('div')
  doc.body.appendChild(host)
  mount(v1.default, host, { start: 1 })
  click(all(host, 'button')[0])
  click(all(host, 'button')[0])
  assert.equal(text(all(host, 'button')[0]), 'Clicks: 3 × 2 = 6')
  const v2 = await import(pathToFileURL(join(dir, 'counter.dev2.js')).href)
  assert.equal(v2.default.h, v1.default.h)
  assert.equal(hmr(v1.default.h, v2), 1)
  assert.equal(text(all(host, 'button')[0]), 'Clicks: 3 double = 6')
  click(all(host, 'button')[0])
  assert.equal(text(all(host, 'button')[0]), 'Clicks: 4 double = 8')
  console.log('ok dev-hmr')
}
