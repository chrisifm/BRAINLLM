import { test } from 'node:test'
import assert from 'node:assert/strict'
import { install } from '../shim/dom.js'
const doc = install()
const rt = await import('./rt.js')
const { urlState, urlBind } = await rt.sibling('brain-url.js')
const events = new Map(), reports = [], writes = []
globalThis.addEventListener = (key, f) => { if (!events.has(key)) events.set(key, new Set()); events.get(key).add(f) }
globalThis.removeEventListener = (key, f) => events.get(key)?.delete(f)
globalThis.reportError = e => reports.push(e)
globalThis.location = new URL('https://example.test/products?campaign=keep&tab=details')
globalThis.history = { state: { y: 22 }, pushState(state, _, url) { this.state = state; writes.push(url); globalThis.location = new URL(url) }, replaceState(state, _, url) { this.state = state; globalThis.location = new URL(url) } }
const flush = () => new Promise(r => setTimeout(r, 0))
function pop(url) { globalThis.location = new URL(url); for (const f of events.get('popstate') || []) f() }

test('URL public state restores direct links, preserves unknown params and uses history without actions', async () => {
  let tab
  const stop = rt.mount(() => { tab = urlState('overview', {key:'tab'}); return doc.createElement('div') }, doc.body)
  assert.equal(tab.v, 'details')
  tab.v = 'reviews'
  await flush()
  assert.equal(location.searchParams.get('tab'), 'reviews')
  assert.equal(location.searchParams.get('campaign'), 'keep')
  assert.deepEqual(history.state, {y:22})
  const count = writes.length
  pop('https://example.test/products?campaign=keep&tab=details')
  await flush()
  assert.equal(tab.v, 'details')
  assert.equal(writes.length, count, 'restoring history must not push another entry')
  pop('https://example.test/products?campaign=keep')
  await flush()
  assert.equal(tab.v, 'overview')
  stop()
  assert.equal(events.get('popstate').size, 0)
})

test('typed modal state, invalid public values and repeated keys have bounded diagnostics', async () => {
  globalThis.location = new URL('https://example.test/products?modal=true')
  let modal, duplicate
  const stop = rt.mount(() => {
    modal = urlState(false, {key:'modal'})
    duplicate = urlState(false, {key:'modal'})
    urlState('', {key:'accessToken'})
    return doc.createElement('div')
  }, doc.body)
  assert.equal(modal.v, true)
  assert.equal(duplicate.v, false)
  assert.ok(reports.some(e => /Repeated/.test(e.message)))
  assert.ok(reports.some(e => /credentials/.test(e.message)))
  modal.v = false
  await flush()
  assert.equal(location.searchParams.has('modal'), false)
  pop('https://example.test/products?modal=bad')
  await flush()
  assert.equal(modal.v, false)
  stop()
})

test('automatic binding is idempotent; router separates public UI state from server query changes', () => {
  globalThis.location = new URL('https://example.test/products?campaign=keep')
  const stop = rt.mount(() => {
    const cell = rt.sig('a')
    assert.equal(urlBind(cell, 'ui.select'), cell)
    urlBind(cell, 'ui.select')
    return doc.createElement('div')
  }, doc.body)
  assert.equal(globalThis.__brainURLOnly('https://example.test/products?campaign=keep', 'https://example.test/products?campaign=keep&ui.select=b'), true)
  assert.equal(globalThis.__brainURLOnly('https://example.test/products?campaign=keep', 'https://example.test/products?campaign=other'), false)
  assert.equal(globalThis.__brainURLOnly('https://example.test/products', 'https://example.test/account'), false)
  stop()
})

test('server rendering restores only public typed query state without installing listeners',()=>{
  globalThis.__brainRoute={path:'/products',params:{},query:{tab:'reviews',modal:'true',password:'private'}}
  try{
    assert.equal(urlState('overview',{key:'tab'}).v,'reviews')
    assert.equal(urlState(false,{key:'modal'}).v,true)
    assert.equal(urlState('',{key:'password'}).v,'')
    assert.equal(events.get('popstate').size,0)
  }finally{delete globalThis.__brainRoute}
})
