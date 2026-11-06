import { test } from 'node:test'
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import vm from 'node:vm'

const client = readFileSync(new URL('../../crates/brain-serve/src/client.js', import.meta.url), 'utf8')
const begin = client.indexOf('function documentNavigation(')
const end = client.indexOf('// After a swap, for assistive technology', begin)
assert.ok(begin >= 0 && end > begin, 'execute the actual dev navigation implementation')
const source = client.slice(begin, end)

function harness(options = {}) {
  const assigned = [], events = [], reports = [], mounted = [], landed = []
  const head = { querySelectorAll: () => [], append() {} }
  const parsed = {
    documentElement: { lang: options.language || 'en' }, title: 'Next', head,
    getElementById: () => options.missingRoute ? null : { textContent: '{}' }, querySelector: () => null,
  }
  const context = vm.createContext({
    URL, base: '', scrollY: 0, globalThis: {},
    location: { href: 'https://example.test/start', assign: url => assigned.push(url) },
    history: { state: {}, replaceState() {}, pushState() {} },
    document: { documentElement: { lang: 'en' }, head, querySelector: () => null },
    DOMParser: class { parseFromString() { if (options.parseError) throw options.parseError; return parsed } },
    fetch: async () => {
      if (options.fetchError) throw options.fetchError
      return { redirected: !!options.redirect, status: options.status ?? 200,
        url: options.redirect || 'https://example.test/next', text: async () => '<html></html>' }
    },
    // Keep the unrelated loading timer pending; this test observes the navigation branches themselves.
    setTimeout: () => 0, dispatchEvent() {}, CustomEvent: class {}, scrollTo() {},
    route: async props => { if ('routeError' in options) throw options.routeError; mounted.push(props) },
    landed: url => landed.push(url), now: () => 123,
    applied: (kind, file, at, outcome) => events.push({ kind, file, at, ...outcome }),
    report: value => { if (options.reportError) throw Error('diagnostic sink unavailable'); reports.push(value) },
  })
  vm.runInContext(source, context, { timeout: 1000 })
  return { assigned, events, reports, mounted, landed,
    go: () => vm.runInContext("go('https://example.test/next', true)", context, { timeout: 1000 }) }
}

test('dev client retains client navigation when the destination mounts', async () => {
  const h = harness()
  await h.go()
  assert.equal(h.mounted.length, 1)
  assert.deepEqual(h.landed, ['https://example.test/next'])
  assert.deepEqual(h.assigned, [])
  assert.deepEqual(h.events, [])
  assert.deepEqual(h.reports, [])
})

test('dev client distinguishes intentional document fallbacks without reporting them as exceptions', async () => {
  for (const [options, reason, destination] of [
    [{ redirect: 'https://example.test/login' }, 'navigation-redirect', 'https://example.test/login'],
    [{ status: 503 }, 'navigation-response-status', 'https://example.test/next'],
    [{ missingRoute: true }, 'navigation-missing-route', 'https://example.test/next'],
    [{ language: 'fr' }, 'navigation-language', 'https://example.test/next'],
  ]) {
    const h = harness(options)
    await h.go()
    assert.deepEqual(h.assigned, [destination])
    assert.equal(h.events.length, 1)
    assert.equal(h.events[0].mode, 'full-reload')
    assert.equal(h.events[0].reason, reason)
    assert.equal(h.events[0].file, null, 'fallback events contain no destination/query values')
    assert.deepEqual(h.reports, [])
    assert.deepEqual(h.mounted, [])
  }
})

test('dev client records bounded fetch, document and route failures before its existing full navigation', async () => {
  const long = new Error('x'.repeat(2000)); long.stack = 's'.repeat(8000)
  for (const [options, phase] of [
    [{ fetchError: Error('network unavailable') }, 'fetch'],
    [{ parseError: Error('invalid document') }, 'document'],
    [{ routeError: long }, 'route'],
    [{ routeError: null }, 'route'],
  ]) {
    const h = harness(options)
    await h.go()
    assert.deepEqual(h.assigned, ['https://example.test/next'])
    assert.equal(h.events[0].reason, `navigation-${phase}-failed`)
    assert.equal(h.reports.length, 1)
    assert.equal(h.reports[0].where, 'dev-router')
    assert.equal(h.reports[0].type, 'navigation')
    assert.ok(h.reports[0].message.length <= 500)
    assert.ok(h.reports[0].stack.length <= 4096)
  }
})

test('a failing diagnostic sink cannot prevent the fallback navigation', async () => {
  const h = harness({ fetchError: Error('network unavailable'), reportError: true })
  await h.go()
  assert.deepEqual(h.assigned, ['https://example.test/next'])
})

test('socket reconnect and watcher recovery retain distinct fixed reload reasons', async () => {
  const start = client.indexOf('function connect() {')
  const stop = client.indexOf('\nconnect();', start)
  assert.ok(start >= 0 && stop > start)
  const sockets = [], timers = [], reloaded = []
  const context = vm.createContext({
    ws: null, url: 'ws://example.test/@brain/hmr', base: '', pending: [], box: null,
    WebSocket: class { constructor() { sockets.push(this) } }, traceAttach() {},
    now: () => 123, reload: (...args) => reloaded.push(args),
    setTimeout: (callback, delay) => { timers.push({ callback, delay }); return timers.length },
    fetch: async () => ({ status: 200 }),
  })
  vm.runInContext(client.slice(start, stop) + '\nconnect();', context, { timeout: 1000 })
  const socket = sockets[0]
  socket.onmessage({ data: JSON.stringify({ type: 'reload', path: null, reason: 'watch-overflow' }) })
  socket.onmessage({ data: JSON.stringify({ type: 'reload', path: null, reason: 'arbitrary reason' }) })
  assert.equal(reloaded[0][2], 'watch-overflow')
  assert.equal(reloaded[1][2], undefined, 'unknown reasons retain the existing default')
  socket.onclose()
  assert.equal(timers.length, 1)
  assert.equal(timers[0].delay, 500)
  timers[0].callback()
  await Promise.resolve()
  await Promise.resolve()
  assert.deepEqual(reloaded[2], [null, 123, 'dev-server-reconnected'])
})
