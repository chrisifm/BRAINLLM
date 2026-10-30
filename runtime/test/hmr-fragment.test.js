import { test } from 'node:test'
import assert from 'node:assert/strict'
import { install, MouseEvent } from '../shim/dom.js'
const doc = install()
const { sig, tpl, insert, on, when, list, mount } = await import('./rt.js')
const { dev, hmr, fragment, live } = await (await import('./rt.js')).sibling('brain-dev.js')
const tick = () => new Promise(resolve => setTimeout(resolve, 0))
const click = node => node.dispatchEvent(new MouseEvent('click', { bubbles: true }))
function version(hash, label, { wiring = 'exact same generated wiring', extra = '', title = 'initial', attr = 'title', branch = false } = {}) {
  const html = `<div><input placeholder="Entry"><button ${attr}="${title}">${label}<!></button>${extra}${branch ? '<!>' : ''}</div>`
  const later = '<strong>Later old</strong>'
  const first = fragment(tpl(html), hash, 0), second = fragment(tpl(later), hash, 1)
  let mounts = 0
  function Component() {
    mounts++
    const n = sig(0, 'n'), show = sig(false, 'show'), root = first()
    const button = root.childNodes[1]
    on(button, 'click', () => { n.v++; show.v = !show.v })
    insert(button, () => n.v, button.lastChild)
    if (branch) when(root, () => show.v, () => second(), null, root.lastChild)
    return root
  }
  Component.h = hash
  Component.$hmr = { version: 1, wiring, templates: [html, later] }
  return { Component, mounts: () => mounts }
}

test('static text and safe attributes preserve original DOM, handlers, signals and user input', async () => {
  const old = version('static-one', 'Before ', { title: 'old' }), next = version('static-one', 'After ', { title: 'new' })
  const host = doc.createElement('div'); doc.body.appendChild(host)
  const stop = mount(dev(old.Component), host)
  const root = host.firstChild, input = root.firstChild, button = root.childNodes[1]
  input.value = 'typed by user'; input.selectionStart = 3; input.selectionEnd = 7
  click(button); await tick()
  assert.equal(button.textContent, 'Before 1')
  assert.equal(hmr('static-one', { default: dev(next.Component) }), 1)
  assert.equal(host.firstChild, root)
  assert.equal(root.firstChild, input)
  assert.equal(root.childNodes[1], button)
  assert.equal(button.textContent, 'After 1')
  assert.equal(button.getAttribute('title'), 'new')
  assert.equal(input.value, 'typed by user')
  assert.equal(input.selectionStart, 3)
  assert.equal(input.selectionEnd, 7)
  assert.equal(old.mounts(), 1)
  assert.equal(next.mounts(), 0)
  click(button); await tick()
  assert.equal(button.textContent, 'After 2')
  stop()
})

test('a branch created after a static patch uses latest HTML and disposes its reference record', async () => {
  const old = version('future-branch', 'Before ', { branch: true }), next = version('future-branch', 'After ', { branch: true })
  next.Component.$hmr.templates[1] = '<strong>Later new</strong>'
  const host = doc.createElement('div'); doc.body.appendChild(host)
  const stop = mount(dev(old.Component), host)
  const instance = [...live.get('future-branch')][0]
  const originalRecords = instance.fragments.size
  hmr('future-branch', { default: dev(next.Component) })
  click(host.firstChild.childNodes[1]); await tick()
  assert.ok(host.textContent.includes('Later new'))
  assert.ok(instance.fragments.size > originalRecords)
  click(host.firstChild.childNodes[1]); await tick()
  assert.equal(instance.fragments.size, originalRecords)
  stop()
  assert.equal(instance.fragments.size, 0)
  assert.equal(instance.fragmentRefs, 0)
})

test('logic, structure and stateful attribute changes remount only the changed component', async () => {
  for (const [hash, change] of [['logic-change', { wiring: 'different compiled code' }], ['shape-change', { extra: '<span>added</span>' }], ['stateful-change', { attr: 'name' }]]) {
    const old = version(hash, 'Before '), next = version(hash, 'After ', change)
    const host = doc.createElement('div'); doc.body.appendChild(host)
    const sibling = doc.createElement('aside'); sibling.textContent = 'Neighbor'; host.appendChild(sibling)
    const stop = mount(dev(old.Component), host)
    const root = host.childNodes.find(node => node.tagName === 'DIV')
    click(root.childNodes[1]); await tick()
    hmr(hash, { default: dev(next.Component) })
    const replacement = host.childNodes.find(node => node.tagName === 'DIV')
    assert.notEqual(replacement, root)
    assert.equal(sibling.textContent, 'Neighbor')
    assert.ok(replacement.textContent.includes('1'), 'named signal restored')
    assert.equal(next.mounts(), 1)
    stop()
  }
})

test('unexpected current text/attribute state prevents a partial patch before remount', () => {
  const old = version('dom-conflict', 'Before ', { title: 'old' }), next = version('dom-conflict', 'After ', { title: 'new' })
  const host = doc.createElement('div'); doc.body.appendChild(host)
  const stop = mount(dev(old.Component), host), previous = host.firstChild
  previous.childNodes[1].setAttribute('title', 'runtime-owned')
  const text = previous.childNodes[1].firstChild
  hmr('dom-conflict', { default: dev(next.Component) })
  assert.equal(text.data, 'Before ', 'no text patch happened before the conflicting attribute was found')
  assert.notEqual(host.firstChild, previous)
  stop()
})

test('updating an unmounted component changes future mounts through the previous module wrapper', () => {
  const old = version('cold-component', 'Before '), next = version('cold-component', 'After ')
  const OldWrapper = dev(old.Component)
  assert.equal(hmr('cold-component', { default: dev(next.Component) }), 0)
  const host = doc.createElement('div'); doc.body.appendChild(host)
  const stop = mount(OldWrapper, host)
  assert.equal(host.textContent, 'After 0')
  stop()
})

test('incompatible child HMR rebuilds its owned ancestor once so keyed row moves/removal never resurrect stale roots', async () => {
  function oldChild(props) {
    const node = tpl('<li><button>old</button></li>')()
    on(node.firstChild, 'click', () => { node.firstChild.textContent = 'old clicked' })
    node.setAttribute('data-row', props.id)
    return node
  }
  function newChild(props) {
    const node = tpl('<li><button>new</button><em>added</em></li>')()
    on(node.firstChild, 'click', () => { node.firstChild.textContent = 'new clicked' })
    node.setAttribute('data-row', props.id)
    return node
  }
  oldChild.h = newChild.h = 'keyed-child'
  const Child = dev(oldChild)
  let mounts = 0
  function Parent() {
    mounts++
    const rows = sig([{ id: 'one' }, { id: 'two' }], 'rows')
    const node = tpl('<section><button>Reverse</button><button>Drop</button><ul></ul></section>')()
    on(node.firstChild, 'click', () => { rows.v = [...rows.v].reverse() })
    on(node.childNodes[1], 'click', () => { rows.v = rows.v.slice(1) })
    list(node.lastChild, () => rows.v, item => item.id, item => Child({ id: item.v.id }))
    return node
  }
  Parent.h = 'keyed-parent'
  const host = doc.createElement('div'); doc.body.appendChild(host)
  const stop = mount(dev(Parent), host)
  const outside = doc.createElement('aside'); outside.textContent = 'Outside'; doc.body.appendChild(outside)
  click(host.firstChild.firstChild); await tick()
  const oldRows = [...host.firstChild.lastChild.childNodes].filter(node => node.nodeType === 1)
  assert.deepEqual(oldRows.map(node => node.getAttribute('data-row')), ['two', 'one'])
  assert.equal(hmr('keyed-child', { default: dev(newChild) }), 2)
  assert.equal(mounts, 2, 'one ancestor rebuild covers both child instances')
  let parent = host.firstChild, rows = () => [...parent.lastChild.childNodes].filter(node => node.nodeType === 1)
  assert.deepEqual(rows().map(node => node.getAttribute('data-row')), ['two', 'one'], 'ancestor named list state restored')
  assert.ok(rows().every(node => node.textContent === 'newadded'))
  click(parent.firstChild); await tick()
  assert.deepEqual(rows().map(node => node.getAttribute('data-row')), ['one', 'two'])
  assert.ok(rows().every(node => !oldRows.includes(node)))
  click(parent.childNodes[1]); await tick()
  assert.equal(rows().length, 1)
  click(rows()[0].firstChild); await tick()
  assert.equal(rows()[0].textContent, 'new clickedadded')
  assert.equal(outside.textContent, 'Outside')
  stop()
  assert.equal(host.childNodes.length, 0)
  assert.equal(live.has('keyed-parent'), false)
  assert.equal(live.has('keyed-child'), false)
})

test('branch toggles after an incompatible nested update dispose the new child roots without orphans', async () => {
  const old = () => tpl('<i>old</i>')(), next = () => tpl('<b>new</b>')()
  old.h = next.h = 'conditional-child'
  const Child = dev(old)
  function Parent() {
    const visible = sig(true, 'visible')
    const node = tpl('<section><button>Toggle</button><!></section>')()
    on(node.firstChild, 'click', () => { visible.v = !visible.v })
    when(node, () => visible.v, () => Child({}), null, node.lastChild)
    return node
  }
  Parent.h = 'conditional-parent'
  const host = doc.createElement('div'); doc.body.appendChild(host)
  const stop = mount(dev(Parent), host)
  assert.equal(hmr('conditional-child', { default: dev(next) }), 1)
  assert.equal(host.textContent, 'Togglenew')
  click(host.firstChild.firstChild); await tick()
  assert.equal(host.textContent, 'Toggle')
  click(host.firstChild.firstChild); await tick()
  assert.equal(host.textContent, 'Togglenew')
  stop()
  assert.equal(host.childNodes.length, 0)
})

test('an empty component keeps a stable invisible anchor for its first later hot render', () => {
  const old = () => null, next = () => tpl('<strong>now visible</strong>')()
  old.h = next.h = 'empty-child'
  const host = doc.createElement('div'); doc.body.appendChild(host)
  const stop = mount(dev(old), host)
  assert.equal(host.childNodes.length, 1)
  assert.equal(host.firstChild.nodeType, 8)
  assert.equal(hmr('empty-child', { default: dev(next) }), 1)
  assert.equal(host.textContent, 'now visible')
  stop()
  assert.equal(host.childNodes.length, 0)
})
