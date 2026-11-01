import { test } from 'node:test'
import assert from 'node:assert/strict'
import { install } from '../shim/dom.js'
const doc = install()
const { default: adapter, post } = await import('../templates/captcha.js')
const flush = async () => { for (let n = 0; n < 8; n++) await Promise.resolve() }
const fake = () => {
  const calls = { render: [], reset: [], remove: [] }
  return { calls, render(element, options) { calls.render.push(options); return 'widget-1' }, reset(id) { calls.reset.push(id) }, remove(id) { calls.remove.push(id) } }
}
const mount = props => {
  const states = [], element = doc.createElement('div')
  return { states, element, widget: adapter.mount(element, { sitekey: 'synthetic-site-key', onState: state => states.push(state), ...props }), last: () => states.at(-1) }
}
test('a token arrives through its state, expires, and a reset clears it for the next submit', async () => {
  const api = fake(); globalThis.turnstile = api
  const { widget, last, states, element } = mount({})
  try {
    await flush()
    assert.equal(last().status, 'ready'); assert.equal(api.calls.render[0].sitekey, 'synthetic-site-key')
    api.calls.render[0].callback('synthetic-token')
    assert.deepEqual(last(), { status: 'verified', token: 'synthetic-token' })
    api.calls.render[0]['expired-callback']()
    assert.deepEqual(last(), { status: 'expired', token: '' })
    api.calls.render[0].callback('second-token')
    widget.update({ action: 'reset' })
    assert.deepEqual(api.calls.reset, ['widget-1'])
    assert.deepEqual(last(), { status: 'ready', token: '' })
    assert.equal(api.calls.render.length, 1, 'a reset keeps the same widget')
    assert.equal(element.getAttribute('aria-busy'), 'false')
    assert.equal(states[0].status, 'loading')
  } finally { widget.destroy(); delete globalThis.turnstile }
  assert.deepEqual(api.calls.remove, ['widget-1'])
})
test('a provider error drops the token and tells the provider it was handled', async () => {
  const api = fake(); globalThis.hcaptcha = api
  const { widget, last } = mount({ provider: 'hcaptcha' })
  try {
    await flush()
    api.calls.render[0].callback('synthetic-token')
    assert.equal(api.calls.render[0]['error-callback'](), true)
    assert.deepEqual(last(), { status: 'error', token: '' })
    widget.update({ action: 'reset' }); assert.equal(last().status, 'ready')
  } finally { widget.destroy(); delete globalThis.hcaptcha }
})
test('without a known provider or a site key it reports an error and loads nothing', async () => {
  for (const props of [{ provider: 'other' }, { sitekey: '' }]) {
    const { widget, last } = mount(props)
    await flush(); assert.equal(last().status, 'error'); widget.destroy()
  }
  assert.equal(doc.querySelector('script'), null)
})
test('a destroyed widget reports nothing further', async () => {
  const api = fake(); globalThis.turnstile = api
  const { widget, states } = mount({})
  await flush(); widget.destroy()
  const count = states.length
  api.calls.render[0].callback('late-token')
  assert.equal(states.length, count)
  delete globalThis.turnstile
})
test('post sends the headers it is given, same-origin, and answers a failure without throwing', async () => {
  const previous = globalThis.fetch
  let seen
  globalThis.fetch = async (path, options) => { seen = [path, options]; return { status: 403, json: async () => ({ error: { code: 'CAPTCHA_FAILED' } }) } }
  try {
    const answer = await post('/_brain/auth/signup', { email: 'x@example.test' }, { 'x-brain-captcha': 'synthetic-token' })
    assert.equal(answer.status, 403); assert.equal(answer.body.error.code, 'CAPTCHA_FAILED')
    assert.equal(seen[1].headers['x-brain-captcha'], 'synthetic-token'); assert.equal(seen[1].credentials, 'same-origin')
  } finally { globalThis.fetch = previous }
})
