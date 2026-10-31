// #106 brain-mcp.js (WebMCP agent tools) and brain-consent.js (Consent Mode v2), INTERNALS §32.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { install } from '../shim/dom.js'

const doc = install()
const rt = await import('./rt.js')
const { tpl, mount } = rt
const { tool, schema, context, definition, registerTools } = await rt.sibling('brain-mcp.js')

// A model context like Chrome's: registerTool() refuses a name already registered and returns the handle.
function fakeContext() {
  const reg = new Map(), calls = { register: 0, unregister: 0 }
  const mc = {
    registerTool(t) {
      if (reg.has(t.name)) throw Error(`InvalidStateError: ${t.name} is already registered`)
      calls.register++
      reg.set(t.name, t)
      return { unregister: () => (calls.unregister++, reg.delete(t.name)) }
    },
  }
  return { mc, reg, calls }
}
// What the codegen emits for `<meta tools="add_task: Add a task (text:str!, due:date, tags:list)">` (INTERNALS §32).
const addTaskDef = (fn, extra) => ({ name: 'add_task', description: 'Add a task', inputSchema: { type: 'object', properties: { text: { type: 'string' }, due: { type: 'string', format: 'date' }, tags: { type: 'array' } }, required: ['text'] }, execute: fn, ...extra })

test('schema(): form() mini-language → JSON Schema', () => {
  assert.deepEqual(schema({ q: 'str!<=80', max: 'num<=50', tags: '[str]', email: 'email' }), {
    type: 'object',
    properties: { q: { type: 'string', maxLength: 80 }, max: { type: 'number', maximum: 50 }, tags: { type: 'array', items: { type: 'string' } }, email: { type: 'string', format: 'email' } },
    required: ['q'],
  })
})

test('tool(): registered on mount, validated before running, unregistered on unmount; a no-op without a model context', async () => {
  assert.equal(context(), null)
  const Plain = () => (tool('noop', 'nothing', {}, () => 1), tpl('<p></p>')())
  mount(Plain, doc.body.appendChild(doc.createElement('div')))() // no model context: nothing happens, nothing throws
  const reg = new Map()
  doc.modelContext = { registerTool: t => (reg.set(t.name, t), { unregister: () => reg.delete(t.name) }) }
  const Shop = () => {
    tool('search_products', 'Search the catalog by text', { q: 'str!', max: 'num<=50' }, ({ q, max }) => ({ q, max, hits: 2 }))
    return tpl('<p>shop</p>')()
  }
  const stop = mount(Shop, doc.body.appendChild(doc.createElement('div')))
  const t = reg.get('search_products')
  assert.equal(t.description, 'Search the catalog by text')
  assert.deepEqual(t.inputSchema.required, ['q'])
  assert.deepEqual(await t.execute({ q: 'mug', max: 5 }), { content: [{ type: 'text', text: '{"q":"mug","max":5,"hits":2}' }] })
  assert.deepEqual(await t.execute({ max: 5 }), { content: [{ type: 'text', text: 'q: Required' }] }) // never ran
  assert.match((await t.execute({ q: 'x', max: 99 })).content[0].text, /^max: /)
  stop()
  assert.equal(reg.has('search_products'), false)
  delete doc.modelContext
})

test('tool(): title and annotations (WebMCP draft 2026-10-27) reach the registration; only the keys given', async () => {
  const reg = new Map()
  // navigator.modelContext (Chrome origin trial name) works the same as document.modelContext
  const nav = Object.getOwnPropertyDescriptor(globalThis, 'navigator')
  Object.defineProperty(globalThis, 'navigator', { configurable: true, value: { modelContext: { registerTool: t => (reg.set(t.name, t), { unregister: () => reg.delete(t.name) }) } } })
  try {
    const Cart = () => {
      tool('list_cart', 'List the items in the cart', {}, () => [], { title: 'List cart', readOnly: true })
      tool('place_order', 'Place the order for the items in the cart', { note: 'str<=200' }, () => 'ok', { title: 'Place order', consequential: true, untrusted: false })
      tool('read_reviews', 'Read customer reviews of a product', { id: 'num!' }, () => [], { untrusted: true })
      tool('plain', 'No options at all', {}, () => 1)
      return tpl('<p>cart</p>')()
    }
    const stop = mount(Cart, doc.body.appendChild(doc.createElement('div')))
    const pick = (n) => { const { execute, ...rest } = reg.get(n); assert.equal(typeof execute, 'function'); return rest }
    assert.deepEqual(pick('list_cart'), { name: 'list_cart', description: 'List the items in the cart', inputSchema: { type: 'object', properties: {}, required: [] }, title: 'List cart', annotations: { readOnlyHint: true } })
    assert.deepEqual(pick('place_order').annotations, { consequentialHint: true, untrustedContentHint: false })
    assert.equal(pick('place_order').title, 'Place order')
    assert.deepEqual(pick('read_reviews').annotations, { untrustedContentHint: true })
    assert.equal('title' in pick('read_reviews'), false)
    assert.deepEqual(Object.keys(pick('plain')), ['name', 'description', 'inputSchema'])
    assert.deepEqual(await reg.get('place_order').execute({ note: 'leave at the door' }), { content: [{ type: 'text', text: 'ok' }] })
    stop()
    assert.equal(reg.size, 0)
  } finally {
    if (nav) Object.defineProperty(globalThis, 'navigator', nav)
    else delete globalThis.navigator
  }
  assert.deepEqual(definition('x', 'd', {}, { title: '', readOnly: 0 }), { name: 'x', description: 'd', inputSchema: { type: 'object', properties: {}, required: [] }, annotations: { readOnlyHint: false } })
})

test('registerTools(): a no-op without a model context (nothing registered, nothing thrown, the component renders)', () => {
  assert.equal(context(), null)
  let ran = 0
  const Tasks = () => (registerTools([addTaskDef(() => ran++)]), tpl('<p>tasks</p>')())
  const host = doc.body.appendChild(doc.createElement('div'))
  const stop = mount(Tasks, host)
  assert.equal(host.textContent, 'tasks')
  stop()
  assert.equal(ran, 0)
})

test('registerTools(): `<meta tools>` registered on mount with the compiled schema, validated, unregistered on unmount', async () => {
  const { mc, reg } = fakeContext()
  doc.modelContext = mc
  try {
    const seen = []
    const add_task = async ({ text, due }) => (seen.push([text, due]), { id: 1, text })
    const clear_done = () => 'cleared'
    const Tasks = () => {
      registerTools([addTaskDef(add_task), { name: 'clear_done', description: 'Remove completed tasks', inputSchema: { type: 'object', properties: {}, required: [] }, execute: clear_done }])
      return tpl('<p>tasks</p>')()
    }
    const stop = mount(Tasks, doc.body.appendChild(doc.createElement('div')))
    assert.deepEqual([...reg.keys()], ['add_task', 'clear_done'])
    const { execute, ...def } = reg.get('add_task')
    assert.equal(typeof execute, 'function')
    assert.deepEqual(def, { name: 'add_task', description: 'Add a task', inputSchema: addTaskDef().inputSchema })
    assert.deepEqual(await execute({ text: 'Buy milk', due: '2026-11-02' }), { content: [{ type: 'text', text: '{"id":1,"text":"Buy milk"}' }] })
    assert.deepEqual(seen, [['Buy milk', '2026-11-02']])
    // Arguments are checked against the schema before the author's function runs.
    assert.deepEqual(await execute({ due: '2026-11-02' }), { content: [{ type: 'text', text: 'text: Required' }] })
    assert.match((await execute({ text: 'x', due: 'tomorrow' })).content[0].text, /^due: /)
    assert.match((await execute({ text: 'x', tags: 'urgent' })).content[0].text, /^tags: /)
    assert.match((await execute({ text: 42 })).content[0].text, /^text: /)
    assert.equal(seen.length, 1)
    assert.deepEqual(await reg.get('clear_done').execute(), { content: [{ type: 'text', text: 'cleared' }] })
    // A throwing function answers with its message instead of breaking the page.
    const Broken = () => (registerTools([{ name: 'fail_now', description: 'Fails', inputSchema: { type: 'object', properties: {}, required: [] }, execute: () => { throw Error('offline') } }]), tpl('<p></p>')())
    const stopBroken = mount(Broken, doc.body.appendChild(doc.createElement('div')))
    assert.deepEqual(await reg.get('fail_now').execute({}), { content: [{ type: 'text', text: 'Error: offline' }] })
    stopBroken()
    stop()
    assert.equal(reg.size, 0)
  } finally {
    delete doc.modelContext
  }
})

test('registerTools(): optional title and annotations reach the registration exactly as tool() passes them', () => {
  const { mc, reg } = fakeContext()
  doc.modelContext = mc
  try {
    const List = () => {
      registerTools([
        addTaskDef(() => 1, { title: 'Add task', consequential: true }),
        { name: 'list_tasks', description: 'List the tasks', inputSchema: { type: 'object', properties: {}, required: [] }, execute: () => [], title: 'List tasks', readOnly: true, untrusted: false },
        { name: 'plain', description: 'No options', inputSchema: { type: 'object', properties: {}, required: [] }, execute: () => 1, title: '' },
      ])
      return tpl('<p></p>')()
    }
    const stop = mount(List, doc.body.appendChild(doc.createElement('div')))
    const pick = n => { const { execute, ...rest } = reg.get(n); return rest }
    assert.deepEqual(pick('add_task'), { ...definition('add_task', 'Add a task', {}, { title: 'Add task', consequential: true }), inputSchema: addTaskDef().inputSchema })
    assert.deepEqual(pick('list_tasks'), { name: 'list_tasks', description: 'List the tasks', inputSchema: { type: 'object', properties: {}, required: [] }, title: 'List tasks', annotations: { readOnlyHint: true, untrustedContentHint: false } })
    assert.deepEqual(Object.keys(pick('plain')), ['name', 'description', 'inputSchema'])
    stop()
  } finally {
    delete doc.modelContext
  }
})

test('registerTools(): no leak across mounts; a second instance whose name is taken leaves the first registered and the page working', () => {
  const { mc, reg, calls } = fakeContext()
  doc.modelContext = mc
  try {
    const Tasks = () => (registerTools([addTaskDef(() => 1)]), tpl('<p>tasks</p>')())
    for (let i = 0; i < 3; i++) {
      const stop = mount(Tasks, doc.body.appendChild(doc.createElement('div')))
      assert.equal(reg.size, 1)
      stop()
      assert.equal(reg.size, 0)
    }
    assert.deepEqual(calls, { register: 3, unregister: 3 })
    // Two instances at once: the browser refuses the duplicate name; the second still renders and its unmount does
    // not remove the first one's tool.
    const first = mount(Tasks, doc.body.appendChild(doc.createElement('div')))
    const host = doc.body.appendChild(doc.createElement('div'))
    const second = mount(Tasks, host)
    assert.equal(host.textContent, 'tasks')
    second()
    assert.equal(reg.size, 1)
    first()
    assert.equal(reg.size, 0)
    assert.deepEqual(calls, { register: 4, unregister: 4 })
  } finally {
    delete doc.modelContext
  }
})

test('registerTools(): skipped while rendering on the server', () => {
  const { mc, reg } = fakeContext()
  doc.modelContext = mc
  globalThis.__brainSSR = true
  try {
    const Tasks = () => (registerTools([addTaskDef(() => 1)]), tpl('<p></p>')())
    const stop = mount(Tasks, doc.body.appendChild(doc.createElement('div')))
    assert.equal(reg.size, 0)
    stop()
  } finally {
    delete globalThis.__brainSSR
    delete doc.modelContext
  }
})

test('consent(): gtag consent update per kind, remembered and applied again on the next load', async () => {
  const calls = [], store = new Map()
  globalThis.localStorage = { getItem: k => store.get(k) ?? null, setItem: (k, v) => store.set(k, v) }
  globalThis.gtag = (...a) => calls.push(a)
  const { consent, consented } = await rt.sibling('brain-consent.js')
  assert.equal(consented('analytics'), false)
  consent('analytics')
  assert.deepEqual(calls.at(-1), ['consent', 'update', { analytics_storage: 'granted' }])
  consent('ads', false)
  assert.deepEqual(calls.at(-1), ['consent', 'update', { ad_storage: 'denied', ad_user_data: 'denied', ad_personalization: 'denied' }])
  assert.equal(consented('analytics'), true)
  assert.throws(() => consent('cookies'), /unknown kind/)
  calls.length = 0
  await import(rt.base + 'brain-consent.js?next-visit') // a new page load
  assert.deepEqual(calls[0], ['consent', 'update', { analytics_storage: 'granted', ad_storage: 'denied', ad_user_data: 'denied', ad_personalization: 'denied' }])
  delete globalThis.gtag
})
