// #106 brain-mcp.js (WebMCP agent tools) and brain-consent.js (Consent Mode v2), INTERNALS §32.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { install } from '../shim/dom.js'

const doc = install()
const rt = await import('./rt.js')
const { tpl, mount } = rt
const { tool, schema, context, definition } = await rt.sibling('brain-mcp.js')

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
