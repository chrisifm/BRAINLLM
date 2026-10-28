// #91 runtime/shim/probe.js (INTERNALS §30.2): fake timers, 30 s of virtual time, B0907 on runaway growth only.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { install } from '../shim/dom.js'

install()
const rt = await import('./rt.js')
const core = await import(rt.base + 'brain.js')
const { probe } = await import('../shim/probe.js')
const { sig, tpl, insert, fx, mount } = core
const view = (h, body) => {
  const C = () => {
    const r = tpl(`<section data-b="${h}"><ul></ul></section>`)()
    body(r)
    return r
  }
  return root => mount(C, root, {})
}

test('a list that grows forever from an interval: B0907 for the nodes and for the list, in ~200 ms real', async () => {
  const t = Date.now()
  const items = sig([])
  const f = await probe({
    core,
    mount: view('grow1', r => {
      fx(() => {
        const id = setInterval(() => (items.v = [...items.v, items.v.length]), 100)
        return () => clearInterval(id)
      })
      insert(r.firstChild, () => items.v.map(i => tpl(`<li>${i}</li>`)()), null)
    }),
  })
  assert.ok(Date.now() - t < 2000, `${Date.now() - t} ms`)
  const nodes = f.find(x => x.what === 'nodes'), list = f.find(x => x.what === 'list')
  assert.ok(nodes && list, JSON.stringify(f))
  assert.deepEqual([nodes.code, nodes.comp], ['B0907', 'grow1'])
  assert.match(nodes.growth, /^\+\d+ nodes\/s$/)
  assert.equal(list.growth, '+1 item/tick')
})

test('healthy components pass: a clock, a stopwatch-like bounded list updated every 100 ms, a one-shot timeout', async () => {
  const time = sig(0), log = sig([]), done = sig(false)
  const f = await probe({
    core,
    mount: view('ok1', r => {
      fx(() => {
        const a = setInterval(() => (time.v = Date.now()), 1000) // a clock: legit
        const b = setInterval(() => (log.v = [...log.v, 1].slice(-5)), 100) // bounded
        const c = setTimeout(() => (done.v = true), 500)
        return () => (clearInterval(a), clearInterval(b), clearTimeout(c))
      })
      insert(r.firstChild, () => [time.v, log.v.length, done.v].join(' '), null)
    }),
  })
  assert.deepEqual(f, [])
})

test('a requestAnimationFrame loop that never stops is flagged; timers are restored afterwards', async () => {
  const n = sig(0)
  const st = globalThis.setTimeout
  const f = await probe({
    core,
    seconds: 5,
    mount: view('raf1', r => {
      const loop = () => ((n.v = n.v + 1), requestAnimationFrame(loop))
      requestAnimationFrame(loop)
      insert(r.firstChild, () => n.v, null)
    }),
  })
  assert.ok(f.some(x => x.what === 'raf'), JSON.stringify(f))
  assert.equal(globalThis.setTimeout, st)
})
