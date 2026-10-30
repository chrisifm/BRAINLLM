import { test } from 'node:test'
import assert from 'node:assert/strict'
import { install } from '../shim/dom.js'
const doc = install(), rt = await import('./rt.js')
const { _rt } = await rt.sibling('brain.js')
const reports = [], original = console.error
console.error = (...args) => reports.push(args)

test('production core errors never expose exception messages, paths or stacks', () => {
  try {
    const e = new Error('private-query-and-credential')
    e.stack = 'Error: private-query-and-credential\n at /private/server/data.brain:22:7'
    _rt().oops(e, null, 'render')
    assert.equal(reports.length, 1)
    assert.equal(typeof reports[0][0], 'string')
    assert.match(reports[0][0], /Operation failed/)
    assert.doesNotMatch(JSON.stringify(reports), /private-query|data.brain|credential|stack/)
  } finally { reports.length = 0 }
})

test('production capture retains private diagnostics but public logging contains only a reference', async () => {
  const E = await rt.sibling('brain-err.js')
  E.capture({log:true})
  const e = new Error('private-provider-token')
  e.stack = 'Error: private-provider-token\n at https://example.test/secret.brain:9:4'
  _rt().oops(e, null, 'render')
  assert.ok(E.errors.some(x => x.msg === 'private-provider-token'))
  assert.match(reports.at(-1)[0], /Reference:/)
  assert.doesNotMatch(JSON.stringify(reports), /private-provider|secret.brain|at https/)
  console.error = original
})
