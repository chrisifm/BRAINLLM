import { test } from 'node:test'
import assert from 'node:assert/strict'
import { install, MouseEvent } from '../shim/dom.js'

const doc = install()
let fakeStatus = 200
globalThis.fetch = async () => ({ status: fakeStatus, headers: { get: k => (k === 'content-length' ? '42' : null) } })
const rt = await import('./rt.js')
const { sig, drv, fx, tpl, insert, on, mount, batch } = rt
const { dev } = await rt.sibling('brain-dev.js')
const I = await rt.sibling('brain-inspect.js')
const sent = []
const api = I.inspect({ send: s => sent.push(JSON.parse(s)), noPanel: true, sample: 1e9, batch: 1e9 })
const recs = k => sent.flatMap(m => m.d).filter(r => r.k === k)
const flush = () => api.drain()
const wait = (ms = 5) => new Promise(r => setTimeout(r, ms))
const click = el => el.dispatchEvent(new MouseEvent('click', { bubbles: true }))

let leakyHandle
function Clock($p) {
  const n = sig(0, 'n'), dbl = drv(() => n.v * 2)
  const r = tpl('<p><!></p>')()
  insert(r, () => dbl.v, r.firstChild)
  fx(() => {
    leakyHandle = setInterval(() => {}, 1000) // no cleanup → B0602 + timer cleanup:false (reads no signals: runs only once)
  })
  fx(() => {
    const t = setInterval(() => {}, 500)
    return () => clearInterval(t)
  })
  on(r, 'click', () => {
    n.v++
    fetch('/api/x', { method: 'POST' })
  })
  return r
}
Clock.h = 'clk'
Clock.f = 'comp/clock.brain'

test('inspector: tree, per-component counts, last writer, attributed network and timers', async t => {
  t.after(() => clearInterval(leakyHandle)) // the test releases the leaked interval so Node can exit
  const stop = mount(dev(Clock), doc.body, {})
  await wait()
  click(doc.body.firstChild)
  await wait()
  api.tick()
  const inst = recs('inst')[0]
  assert.equal(inst.name, 'Clock')
  assert.equal(inst.f, 'comp/clock.brain')
  const comp = recs('comp').find(c => c.h === 'clk')
  assert.deepEqual([comp.inst, comp.sig, comp.drv, comp.fx], [1, 1, 1, 3]) // 2 user fx + the insert
  assert.ok(comp.nodes >= 2 && comp.ev === 1 && comp.est > 0)
  const st = recs('state').find(s => s.id === inst.id)
  assert.deepEqual(st.sigs[0], { n: 'n', v: 1, w: 'Clock · on:click', c: 1 })
  assert.deepEqual(st.drv, [2])
  const net = recs('net')
  assert.equal(net[0].m, 'POST')
  assert.equal(net[0].own.o, 'on:click')
  assert.equal(net[0].own.name, 'Clock')
  assert.ok(net.some(r => r.id === net[0].id && r.st === 200 && r.b === 42))
  const timers = recs('timer').filter(t => t.kind === 'interval')
  assert.equal(timers.length, 2)
assert.ok(timers.every(t => t.own && t.own.o === 'fx' && /inspect\.test\.js:\d+/.test(t.line) && t.foreign === false))
  const cl = recs('timer').filter(t => 'cleanup' in t && t.cleanup !== null && !t.kind)
  assert.deepEqual(cl.map(t => t.cleanup).sort(), [false, true])
  const l = I.inspect().leaks().find(x => x.code === 'B0602')
  assert.equal(l.name, 'Clock')
  assert.equal(l.file, 'comp/clock.brain')
  const upd = recs('upd')
  assert.ok(upd.some(u => u.sig === 'Clock.n' && u.fx >= 1 && u.nodes >= 1), JSON.stringify(upd))
  assert.ok(recs('handler').some(h => h.name === 'Clock' && h.type === 'click'))
  const evs = recs('ev').map(e => e.e)
  assert.ok(evs.includes('mount'))
  stop()
  flush()
  assert.ok(recs('gone').some(g => g.id === inst.id))
  assert.ok(recs('timer').some(t => t.st === 'off')) // the interval with cleanup was cleared on unmount
})

test('inspector: every message is {t:"m", v:2, seq, at, d:[…]} and empty ones are not sent', () => {
  flush()
  assert.equal(api.drain(), null)
  const m = sent[sent.length - 1]
  assert.equal(m.t, 'm')
  assert.equal(m.v, 2)
  assert.ok(Array.isArray(m.d) && m.d.length)
  assert.ok(sent.every((x, j) => j === 0 || x.seq === sent[j - 1].seq + 1))
})

test('inspector: B0601 when an unmounted instance is still retained after 2 GCs', { skip: !globalThis.gc && 'requires --expose-gc' }, async () => {
  const retained = []
  function Leaky() {
    const r = tpl('<b>x</b>')()
    retained.push(r) // external reference: the node survives dispose
    return r
  }
  Leaky.h = 'leaky'
  function Clean() {
    return tpl('<i>y</i>')()
  }
  Clean.h = 'clean'
  const t = doc.createElement('div')
  mount(dev(Leaky), t, {})()
  mount(dev(Clean), t, {})()
  for (let k = 0; k < 4; k++) {
    gc()
    await wait(20)
    api.tick()
  }
  const b601 = I.inspect().leaks().filter(x => x.code === 'B0601').map(x => x.h)
  assert.deepEqual(b601, ['leaky'])
})

test('#295: minor GCs do not count: a clean instance whose nodes were promoted is never a B0601', { skip: !globalThis.gc && 'requires --expose-gc' }, async () => {
  function Promoted() {
    return tpl('<i>z</i>')()
  }
  Promoted.h = 'promoted'
  const t = doc.createElement('div')
  let stop = mount(dev(Promoted), t, {})
  for (let k = 0; k < 4; k++) gc({ type: 'minor' }), await wait(20) // survives into the old generation while mounted
  stop()
  stop = null // its closure holds the nodes
  for (let k = 0; k < 6; k++) gc({ type: 'minor' }), await wait(20), api.tick() // a page allocating: only minor GCs
  for (let k = 0; k < 4; k++) gc(), await wait(20), api.tick() // then major ones collect it
  assert.deepEqual(I.inspect().leaks().filter(x => x.code === 'B0601' && x.h === 'promoted'), [])
})

test('inspector: channel edges (provide/use, store, bus) reach the Graph view once per component', async () => {
  const C = await rt.sibling('brain-ch.js')
  const Theme = C.key('theme'), toast = C.bus('toast'), cart = C.store({ n: 0 }, {}, 'channels/cart')
  function Child() {
    C.use(Theme)
    C.use(Theme)
    toast.on(() => {})
    void cart.n
    return tpl('<i>c</i>')()
  }
  Child.h = 'chd'
  function Parent() {
    C.provide(Theme, 'dark')
    return dev(Child)({})
  }
  Parent.h = 'par'
  const stop = mount(dev(Parent), doc.createElement('div'), {})
  flush()
  const edges = recs('chan').map(r => `${r.comp}:${r.kind}:${r.name}:${r.role}`)
  assert.deepEqual(edges.sort(), ['Child:bus:toast:on', 'Child:ctx:theme:use', 'Child:store:channels/cart:read', 'Parent:ctx:theme:provide'])
  stop()
})

test('inspector: Jank view — LoAF scripts attributed to the component file, Event Timing → INP per component', () => {
  function Slow() {
    return tpl('<button>slow</button>')()
  }
  Slow.h = 'slw'
  Slow.f = 'comp/slow.brain'
  const host = doc.createElement('div')
  const stop = mount(dev(Slow), host, {})
  api.onLoaf([{ duration: 180.04, blockingDuration: 130, scripts: [{ sourceURL: 'http://localhost:5180/comp/slow.brain', sourceFunctionName: 'onClick', invoker: 'BUTTON.onclick', duration: 150.2 }] }])
  api.onEvents([{ name: 'click', target: host.firstChild, duration: 212, startTime: 10, processingStart: 18, processingEnd: 200 }])
  flush()
  const j = recs('jank').at(-1)
  assert.equal(j.ms, 180)
  assert.deepEqual(j.scripts[0], { src: '/comp/slow.brain', fn: 'onClick', inv: 'BUTTON.onclick', ms: 150.2, comp: { h: 'slw', name: 'Slow' } })
  const i = recs('inp').at(-1)
  assert.deepEqual([i.type, i.ms, i.delay, i.proc, i.comp.name], ['click', 212, 8, 182, 'Slow'])
  assert.equal(api.comps().find(c => c.h === 'slw').inp, 212)
  stop()
})

test('TOOLCHAIN #24: the real component name (not the guard/dev wrapper W); unmounted nodes drop their instance links', async () => {
  const { guard } = await rt.sibling('brain-guard.js')
  function Counter() {
    return tpl('<p class="cnt">1</p>')()
  }
  Counter.h = 'cnt24'
  Counter.f = 'comp/counter.brain'
  const t = doc.createElement('div')
  doc.body.appendChild(t)
  const stop = mount(() => dev(guard(Counter))({}), t, {})
  const p = t.querySelector('.cnt')
  assert.ok(p.$i && p.$g && p.$c)
  const c = I.comps().find(x => x.h === 'cnt24')
  assert.equal(c.name, 'Counter')
  stop()
  assert.deepEqual([p.$i, p.$g, p.$c], [null, null, null]) // a node a browser keeps does not keep the instance
})
