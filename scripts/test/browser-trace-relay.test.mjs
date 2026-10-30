import { test } from 'node:test'
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import vm from 'node:vm'

const client = readFileSync(new URL('../../crates/brain-serve/src/client.js', import.meta.url), 'utf8')
const begin = client.indexOf('// Browser errors → server')
const end = client.indexOf('// before brain-err loads', begin)
assert.ok(begin >= 0 && end > begin, 'the test evaluates the actual browser error relay block')
const relay = client.slice(begin, end)
const runtimeImport = "import(base + '/@brain/brain-err.js')"
assert.equal(relay.split(runtimeImport).length, 2, 'replace only the runtime module import, not relay behavior')

async function harness() {
  const callbacks = new Set(), timers = new Map(), sent = [], captureOptions = []
  let timerId = 0
  const sandbox = {
    base: '/app',
    location: { pathname: '/app/checkout', href: 'https://app.test/app/checkout?token=page-secret#private' },
    Date: { now: () => 123456 },
    ov: Promise.resolve(), O: null,
    __errorModule: { capture: options => captureOptions.push(options), sub: { add: callback => callbacks.add(callback) } },
    __socket: { readyState: 1, send: text => sent.push(JSON.parse(text)) },
    setTimeout: (callback, delay) => { assert.equal(delay, 300); timers.set(++timerId, callback); return timerId },
    clearTimeout: id => timers.delete(id),
  }
  const context = vm.createContext(sandbox)
  vm.runInContext(relay.replace(runtimeImport, 'Promise.resolve(__errorModule)') + '\nws = __socket;', context, { timeout: 1000 })
  await Promise.resolve()
  assert.equal(callbacks.size, 1, 'real relay subscribed to runtime capture')
  assert.deepEqual(JSON.parse(JSON.stringify(captureOptions)), [{ maps: true, log: true }])
  return {
    sent, timers,
    emit(record) { for (const callback of callbacks) callback(record) },
    async flush() {
      const ready = [...timers.values()]
      timers.clear()
      for (const callback of ready) callback()
      await Promise.resolve()
    },
  }
}

function mappedRecord() {
  const source = { file: 'views/checkout.brain', line: 17, column: null }
  const generated = { file: '/views/checkout.brain', line: 48, column: 6 }
  return {
    fp: 'checkout-failure', msg: 'Unable to submit', type: 'TypeError', code: 'E_SUBMIT',
    src: 'views/checkout.brain:17', line: '/views/checkout.brain:48',
    stack: ['/views/checkout.brain:48:6'], brainStack: ['views/checkout.brain:17'],
    comp: { name: 'Checkout', file: 'views/checkout.brain', chain: ['Checkout', 'App'] },
    where: 'on:submit', n: 3,
    location: { component: 'Checkout', ...source, precision: 'source-line' },
    generatedLocation: generated,
    frames: [{ generated, source, runtime: false }],
    causes: [{ depth: 1, type: 'Error', code: 'E_STORE', msg: 'Storage unavailable', location: { component: 'Checkout', file: '/api/orders.jsbrain', line: 9, column: 2, precision: 'generated' }, generatedLocation: { file: '/api/orders.jsbrain', line: 9, column: 2 }, frames: [], truncated: { stack: false, frames: false } }],
    truncated: { stack: false, frames: false, causes: false, componentChain: false, breadcrumbs: false, locations: false, mapping: false, output: false },
    mapping: { status: 'mapped' }, crumbs: [{ k: 'fetch', d: 'POST /api/orders', n: 1, at: 123450 }],
  }
}

test('real browser relay preserves structured runtime traces and sends only the page pathname', async () => {
  const h = await harness(), record = mappedRecord()
  h.emit(record)
  assert.equal(h.sent.length, 0, 'the actual relay keeps its source-map debounce')
  await h.flush()
  assert.equal(h.sent.length, 1)
  const event = h.sent[0]
  for (const field of ['location', 'generatedLocation', 'frames', 'causes', 'truncated', 'mapping', 'brainStack', 'crumbs']) assert.deepEqual(event[field], record[field], `${field} survives runtime → WebSocket`)
  assert.deepEqual(event.componentChain, ['Checkout', 'App'])
  assert.equal(event.location.column, null, 'source column precision is not invented by the relay')
  assert.equal(event.generatedLocation.column, 6)
  assert.equal(event.url, '/app/checkout')
  assert.equal(event.t, 'brain:error')
  assert.equal(event.message, record.msg)
  assert.equal(event.stack, record.brainStack.join('\n'))
  assert.equal(event.src, record.src)
  assert.equal(event.comp, 'Checkout')
  assert.equal(event.count, 3)
  assert.equal(event.fp, record.fp)
  assert.equal(h.timers.size, 0)
  const text = JSON.stringify(event)
  for (const secret of ['page-secret', 'token=', '#private', 'https://app.test']) assert.ok(!text.includes(secret), secret)
})

test('real browser relay coalesces enrichment by fingerprint and retains generated fallback fields', async () => {
  const h = await harness(), mapped = mappedRecord()
  const pending = { ...mapped, src: undefined, brainStack: undefined, location: { component: 'Checkout', ...mapped.generatedLocation, precision: 'generated' }, mapping: { status: 'pending' } }
  h.emit(pending)
  h.emit(mapped)
  assert.equal(h.timers.size, 1, 'the second notification replaces its own pending timer')
  await h.flush()
  assert.equal(h.sent.length, 1)
  assert.equal(h.sent[0].mapping.status, 'mapped')
  assert.deepEqual(h.sent[0].location, mapped.location)
  h.emit({ ...pending, fp: 'generated-failure', comp: { file: 'comp/form.brain' }, mapping: { status: 'unavailable' } })
  await h.flush()
  const generated = h.sent[1]
  assert.equal(generated.src, pending.line)
  assert.equal(generated.stack, pending.stack.join('\n'))
  assert.equal(generated.brainStack, null)
  assert.equal(generated.comp, 'comp/form.brain')
  assert.deepEqual(generated.componentChain, [])
  assert.equal(generated.mapping.status, 'unavailable')
})
