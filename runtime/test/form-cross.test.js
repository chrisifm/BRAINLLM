import { test } from 'node:test'
import assert from 'node:assert/strict'
import { install } from '../shim/dom.js'

install()
const { sibling } = await import('./rt.js')
const { form } = await sibling('brain-form.js')

const submitEvent = { preventDefault() {}, target: null }

test('a cross-field rule that returns an empty message does not block the submit', async () => {
  let sent = null
  const f = form({ password: 'str!', confirm: 'str!' }, {
    init: { password: 'one-long-secret', confirm: 'one-long-secret' },
    validate: v => ({ confirm: v.password === v.confirm ? '' : 'Passwords do not match' }),
    submit: async v => { sent = v },
  })
  await f.submit(submitEvent)
  assert.deepEqual(sent, { password: 'one-long-secret', confirm: 'one-long-secret' })
})

test('a cross-field rule marks the field and stops the submit', async () => {
  let sent = false
  const f = form({ password: 'str!', confirm: 'str!' }, {
    init: { password: 'a', confirm: 'b' },
    validate: v => ({ confirm: v.password === v.confirm ? '' : 'Passwords do not match' }),
    submit: async () => { sent = true },
  })
  await f.submit(submitEvent)
  assert.equal(f.err.confirm, 'Passwords do not match')
  assert.equal(sent, false)
  f.v.confirm = 'a'
  await f.submit(submitEvent)
  assert.equal(f.err.confirm, '')
  assert.equal(sent, true)
})
