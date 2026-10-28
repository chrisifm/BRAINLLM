import { test } from 'node:test'
import assert from 'node:assert/strict'
import { sig, drv, fx, batch, untrack } from './rt.js'

const tick = () => new Promise(r => queueMicrotask(r))

test('sig is read and written through .v', () => {
  const n = sig(1)
  n.v++
  assert.equal(n.v, 2)
})

test('drv is lazy, memoized and read-only', () => {
  const n = sig(2)
  let runs = 0
  const d = drv(() => (runs++, n.v * 2))
  assert.equal(runs, 0)
  assert.equal(d.v, 4)
  assert.equal(d.v, 4)
  assert.equal(runs, 1)
  n.v = 3
  assert.equal(d.v, 6)
  assert.equal(runs, 2)
  assert.throws(() => (d.v = 1), /read-only/)
})

test('diamond: the final derived runs once and never sees a mixed state', async () => {
  const a = sig(1)
  const b = drv(() => a.v + 1)
  const c = drv(() => a.v * 10)
  let runs = 0
  const seen = []
  const d = drv(() => (runs++, b.v + c.v))
  fx(() => seen.push(d.v))
  assert.deepEqual(seen, [12])
  a.v = 2
  await tick()
  assert.deepEqual(seen, [12, 23])
  assert.equal(runs, 2)
})

test('glitch-free: an effect never sees intermediate values', async () => {
  const first = sig('a'), last = sig('b')
  const full = drv(() => first.v + ' ' + last.v)
  const log = []
  fx(() => log.push(full.v))
  batch(() => {
    first.v = 'x'
    last.v = 'y'
  })
  assert.deepEqual(log, ['a b', 'x y'])
})

test('equality cutoff: if a drv does not change, downstream is not recomputed', async () => {
  const n = sig(1)
  const par = drv(() => n.v % 2)
  let runs = 0
  fx(() => (par.v, runs++))
  n.v = 3
  await tick()
  assert.equal(runs, 1)
  n.v = 4
  await tick()
  assert.equal(runs, 2)
})

test('writes are batched per microtask', async () => {
  const n = sig(0)
  const log = []
  fx(() => log.push(n.v))
  n.v = 1
  n.v = 2
  n.v = 3
  assert.deepEqual(log, [0])
  await tick()
  assert.deepEqual(log, [0, 3])
})

test('batch applies synchronously when it ends', () => {
  const n = sig(0)
  const log = []
  fx(() => log.push(n.v))
  const r = batch(() => {
    n.v = 5
    n.v = 6
    return 'ok'
  })
  assert.equal(r, 'ok')
  assert.deepEqual(log, [0, 6])
})

test('fx cleanup runs before re-running and on dispose', () => {
  const n = sig(0)
  const log = []
  const stop = fx(() => {
    const v = n.v
    log.push('run' + v)
    return () => log.push('clean' + v)
  })
  batch(() => (n.v = 1))
  stop()
  batch(() => (n.v = 2))
  assert.deepEqual(log, ['run0', 'clean0', 'run1', 'clean1'])
})

test('nested effects are disposed when the parent re-runs', () => {
  const outer = sig(0), inner = sig(0)
  let innerRuns = 0
  fx(() => {
    outer.v
    fx(() => (inner.v, innerRuns++))
  })
  assert.equal(innerRuns, 1)
  batch(() => (outer.v = 1))
  assert.equal(innerRuns, 2)
  batch(() => (inner.v = 1))
  assert.equal(innerRuns, 3) // only the live child, not the disposed one
})

test('dynamic dependencies: a branch no longer read stops subscribing', () => {
  const cond = sig(true), a = sig(1), b = sig(2)
  let runs = 0
  fx(() => (runs++, cond.v ? a.v : b.v))
  batch(() => (cond.v = false))
  assert.equal(runs, 2)
  batch(() => (a.v = 10))
  assert.equal(runs, 2)
  batch(() => (b.v = 20))
  assert.equal(runs, 3)
})

test('untrack reads without subscribing', () => {
  const a = sig(1), b = sig(1)
  let runs = 0
  fx(() => (runs++, a.v + untrack(() => b.v)))
  batch(() => (b.v = 2))
  assert.equal(runs, 1)
  batch(() => (a.v = 2))
  assert.equal(runs, 2)
})

test('an effect that writes triggers another in the same flush', () => {
  const a = sig(0), b = sig(0)
  const log = []
  fx(() => (b.v = a.v * 2))
  fx(() => log.push(b.v))
  batch(() => (a.v = 2))
  assert.deepEqual(log, [0, 4])
})

test('an error in an effect does not leave the system stuck', async () => {
  const n = sig(0)
  fx(() => {
    if (n.v === 1) throw Error('boom')
  })
  const ce = console.error, seen = []
  console.error = e => seen.push(e.message) // without brain-err: reported and the app keeps running (INTERNALS §18)
  batch(() => (n.v = 1))
  console.error = ce
  assert.deepEqual(seen, ['boom'])
  const log = []
  const m = sig(0)
  fx(() => log.push(m.v))
  m.v = 1
  await tick()
  assert.deepEqual(log, [0, 1])
})

test('children of an owner: disposing them out of order runs each cleanup exactly once', () => {
  const log = []
  const stops = []
  const stopOwner = fx(() => {
    for (let i = 0; i < 6; i++) stops.push(fx(() => () => log.push(i)))
    return () => log.push('owner')
  })
  stops[1]()
  stops[5]()
  stops[0]()
  stops[1]() // double dispose: no-op
  stopOwner()
  assert.deepEqual(log.slice(0, 3), [1, 5, 0])
  assert.deepEqual(log.slice(3).filter(x => x !== 'owner').sort(), [2, 3, 4])
  assert.equal(log.length, 7)
})

test('out-of-order unsubscribe with several sources: only live ones run and indices stay consistent', () => {
  const a = sig(0), b = sig(0), c = sig(true)
  const runs = new Array(300).fill(0)
  const stops = runs.map((_, i) => fx(() => (runs[i]++, i % 3 ? a.v + b.v : c.v ? a.v : b.v)))
  let seed = 7
  const order = [...stops.keys()].sort(() => ((seed = (seed * 16807) % 2147483647) % 3) - 1)
  const dead = new Set(order.slice(0, 150))
  for (const i of dead) stops[i]()
  batch(() => (c.v = false)) // relinks the live dynamic ones (a → b) after the swap-removes
  batch(() => (a.v = 1))
  batch(() => (b.v = 1))
  runs.forEach((n, i) => {
    if (dead.has(i)) assert.equal(n, 1, 'dead ' + i)
    else assert.equal(n, i % 3 ? 3 : i % 3 === 0 ? 3 : 0, 'live ' + i)
  })
  for (const s of stops) s()
  batch(() => (a.v = 2))
  assert.ok(runs.every((n, i) => n === (dead.has(i) ? 1 : 3)))
})
