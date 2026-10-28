// #106 brain-mcp.js (WebMCP agent tools) and brain-consent.js (Consent Mode v2), INTERNALS §32.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { install } from '../shim/dom.js'

const doc = install()
const rt = await import('./rt.js')
const { tpl, mount } = rt
const { tool, schema, context } = await rt.sibling('brain-mcp.js')

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
