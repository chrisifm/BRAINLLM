// A server function that throws a deliberate public error ({ message } or a typed 4xx error class) reaches the calling
// form's f.error through RPC and through a form posted without JS; arbitrary exceptions stay private (500, generic).
import { test } from 'node:test'
import assert from 'node:assert/strict'

const { callRpc, callForm } = await import('../server/server.mjs')
const rules = await import('../brain-schema.js')
const ctx = { url: new URL('http://x.test/posts/new'), user: null }

class Conflict extends Error { status = 409 }

test('callRpc: a plain { message } is a public 400 and a typed 4xx error class keeps its status and message', async () => {
  assert.deepEqual(await callRpc({ fn: async () => { throw { message: 'Title already used' } }, args: [], ctx }),
    { status: 400, body: { error: 'Title already used' } })
  // plain { status, message } keeps the existing contract: status preserved, message private (driver detail)
  assert.deepEqual(await callRpc({ fn: async () => { throw { status: 409, message: 'private row detail' } }, args: [], ctx }),
    { status: 409, body: { error: 'Conflict' } })
  assert.deepEqual(await callRpc({ fn: async () => { throw new Conflict('Post was edited elsewhere') }, args: [], ctx }),
    { status: 409, body: { error: 'Post was edited elsewhere' } })
  // a plain status without a message keeps the standard reason phrase
  assert.deepEqual(await callRpc({ fn: async () => { throw { status: 403 } }, args: [], ctx }), { status: 403, body: { error: 'Forbidden' } })
})

test('callRpc: arbitrary exceptions, built-in errors, 5xx and oversized messages stay private', async () => {
  for (const thrown of [new Error('password=hunter2'), new TypeError('x is undefined'), Object.assign(new Error('db'), { status: 404 }),
    { status: 503, message: 'upstream secret' }, { message: 'x'.repeat(501) }, { message: 42 }]) {
    const r = await callRpc({ fn: async () => { throw thrown }, args: [], ctx })
    assert.ok(r.status >= 500, `${String(thrown.message).slice(0, 20)} must stay private`)
    assert.ok(!/hunter2|undefined|secret|xxxx/.test(JSON.stringify(r.body)))
  }
})

test('callForm without JS: the public message reaches the re-rendered form error', async () => {
  const module = { $forms: { save: { title: 'str!' } }, save: async () => { throw { message: 'Title already used' } } }
  const r = await callForm({ module, raw: '_brain=h%2Fsave&title=Hello', ctx, rules })
  assert.equal(r.status, 422)
  assert.equal(r.state['h/save'].error, 'Title already used')
  const typed = { $forms: { save: { title: 'str!' } }, save: async () => { throw new Conflict('Post was edited elsewhere') } }
  assert.equal((await callForm({ module: typed, raw: '_brain=h%2Fsave&title=Hello', ctx, rules })).state['h/save'].error, 'Post was edited elsewhere')
})
