// Exercise the actual harness implementation with controlled module registration in separate document realms.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import vm from 'node:vm'
let nextDocument = 0
const source = readFileSync(new URL('../../crates/brain-serve/src/test-api.js', import.meta.url), 'utf8')
  .replace("import { mount as rtMount } from '/@brain/brain.js';", 'const rtMount = () => () => {};')
  .replace('await import(url);', 'await __register(url);')
function document(register) {
  const events = new Map(), imported = new Set(), state = { calls: [] }
  const context = { performance: { timeOrigin: 1, now: () => 1 }, crypto: { randomUUID: () => `document-${++nextDocument}` }, document: { getElementById: () => ({ replaceChildren() {} }) }, addEventListener: (event, fn) => events.set(event, fn), fetch: async () => {}, URL, Response, setTimeout, clearTimeout, requestAnimationFrame: fn => setTimeout(fn, 0), cancelAnimationFrame: clearTimeout }
  context.window = context
  context.__register = async url => { if (!imported.has(url)) { imported.add(url); await register(context, state, events) } }
  vm.runInNewContext(source, context)
  return { context, state, events }
}
test('a new document or registry cannot execute a scenario from a previous plan', async () => {
  const register = (c, state) => { c.test('first', () => state.calls.push('first')); c.test('second', () => state.calls.push('second')) }
  const first = document(register), plan = await first.context.__brainLoad('/example.test.brain')
  assert.equal((await first.context.__brainStep(0, plan.id)).ok, true)
  const missing = await first.context.__brainStep(99, plan.id)
  assert.equal(missing.documentChanged, true); assert.equal(missing.started, false)
  assert.deepEqual(first.state.calls, ['first'])
  const next = document(register)
  assert.equal(next.context.__brainMatches(plan.id), false)
  assert.equal((await next.context.__brainStep(1, plan.id)).started, false)
  const restored = await next.context.__brainLoad('/example.test.brain')
  assert.notEqual(restored.id, plan.id)
  assert.equal((await next.context.__brainStep(1, plan.id)).started, false)
  assert.deepEqual(next.state.calls, [])
  assert.equal((await next.context.__brainStep(1, restored.id)).ok, true)
  assert.deepEqual(next.state.calls, ['second'], 'the already completed/failed first assertion is never replayed')
  const registeredAgain = await next.context.__brainLoad('/example.test.brain')
  assert.notEqual(registeredAgain.id, restored.id)
  assert.equal(next.context.__brainMatches(restored.id), false)
})
test('trusted navigation invalidates a running step but a synthetic lifecycle event does not', async () => {
  const page = document((c, state, events) => {
    c.test('synthetic', () => { events.get('beforeunload')({ isTrusted: false }); state.calls.push('synthetic') })
    c.test('real navigation', () => { events.get('beforeunload')({ isTrusted: true }); state.calls.push('navigation') })
  })
  const plan = await page.context.__brainLoad('/example.test.brain')
  assert.equal((await page.context.__brainStep(0, plan.id)).ok, true)
  const interrupted = await page.context.__brainStep(1, plan.id)
  assert.equal(interrupted.ok, false); assert.equal(interrupted.documentChanged, true); assert.equal(interrupted.started, true)
  assert.equal(page.context.__brainMatches(plan.id), false)
})
