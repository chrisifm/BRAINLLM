import test from 'node:test'
import assert from 'node:assert/strict'
import { reportVitals } from '../brain-vitals.js'
const environment = () => {
 const listeners = new Map(), beacons = []
 const add = (key, fn) => listeners.set(key, fn)
 const remove = (key, fn) => { if (listeners.get(key) === fn) listeners.delete(key) }
 const env = { document: { visibilityState: 'visible', addEventListener: add, removeEventListener: remove }, location: { pathname: '/example', origin: 'https://example.test', href: 'https://example.test/example?private=value' }, performance: { getEntriesByType: () => [{ responseStart: 42 }] }, navigator: { sendBeacon: (url, body) => { beacons.push({ url, body }); return true } }, addEventListener: add, removeEventListener: remove }
 return { env, listeners, beacons }
}
test('vitals refuses protocol-relative and browser-normalized foreign destinations before installing observers', () => {
 for (const url of ['//foreign.example.test/collect', '/\\foreign.example.test/collect', '/\t/foreign.example.test/collect', '/api/vitals\n', 'https://foreign.example.test/collect', 'relative', '/'.repeat(2049)]) {
  const fixture = environment()
  assert.throws(() => reportVitals(url, fixture.env), TypeError, JSON.stringify(url))
  assert.equal(fixture.listeners.size, 0)
  assert.equal(fixture.beacons.length, 0)
 }
})
test('a declared same-origin endpoint sends only metrics/path and releases listeners', async () => {
 const { env, listeners, beacons } = environment()
 const stop = reportVitals('/api/vitals', env)
 env.document.visibilityState = 'hidden'
 listeners.get('visibilitychange')()
 assert.equal(beacons.length, 1)
 assert.equal(beacons[0].url, 'https://example.test/api/vitals')
 assert.equal(new URL(beacons[0].url, 'https://foreign.example.test/').origin, env.location.origin, 'an authored base element cannot redirect the beacon')
 assert.deepEqual(JSON.parse(await beacons[0].body.text()), { path: '/example', metrics: [{ name: 'TTFB', value: 42, rating: 'good' }] })
 listeners.get('visibilitychange')()
 assert.equal(beacons.length, 1)
 stop()
 assert.equal(listeners.size, 0)
})
