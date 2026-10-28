// brain-guard.js + core routing (INTERNALS §42): every component fails alone.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { install, MouseEvent } from '../shim/dom.js'

const doc = install()
const rt = await import('./rt.js')
const { sig, fx, tpl, insert, on, mount, sibling, list } = rt
const { guard } = await sibling('brain-guard.js')
const tick = (ms = 0) => new Promise(r => setTimeout(r, ms))
const click = el => el.dispatchEvent(new MouseEvent('click', { bubbles: true, cancelable: true }))
const reported = []
console.error = e => reported.push(e && e.message)
const host = () => {
  const h = doc.createElement('div')
  doc.body.appendChild(h)
  return h
}
const Ok = () => tpl('<p class="ok">fine</p>')()
const fb = h => h.querySelector('.brain-fallback')

test('render error: only that component shows the fallback with Retry; its neighbors render', () => {
  const Bad = guard(() => { throw new Error('render broken') })
  const h = host()
  mount(() => [guard(Ok)({}), Bad({}), guard(Ok)({})], h)
  assert.equal(h.querySelectorAll('.ok').length, 2)
  assert.equal(fb(h).getAttribute('role'), 'alert')
  assert.match(fb(h).textContent, /This section could not load\. Retry/)
  assert.ok(reported.includes('render broken'))
})

test('fx error later: the component is replaced by its fallback, its effects stop; custom text', async () => {
  const n = sig(0)
  let runs = 0
  const Counter = guard(() => {
    const r = tpl('<b class="c"><!></b>')()
    insert(r, () => n.v, r.firstChild)
    fx(() => {
      runs++
      if (n.v === 2) throw new RangeError('fx broken')
    })
    return r
  }, 'Counter unavailable')
  const h = host()
  mount(() => [Counter({}), guard(Ok)({})], h)
  n.v = 1
  await tick()
  assert.equal(h.querySelector('.c').textContent, '1')
  n.v = 2
  await tick()
  assert.equal(h.querySelector('.c'), null)
  assert.match(fb(h).textContent, /^Counter unavailable/)
  const before = runs
  n.v = 3
  await tick()
  assert.equal(runs, before) // stopped
  assert.ok(h.querySelector('.ok'))
})

test('handler errors (sync and a rejected await) keep the component and show a toast', async () => {
  const n = sig(0)
  const Btns = guard(() => {
    const r = tpl('<div><button class="s">sync</button><button class="a">async</button><i><!></i></div>')()
    on(r.firstChild, 'click', () => { n.v++; throw new TypeError('sync handler') })
    on(r.childNodes[1], 'click', async () => {
      await tick()
      throw Object.assign(new Error('The server is busy, try again in a minute.'), { kind: 'http', status: 503 })
    })
    insert(r.lastChild, () => n.v, r.lastChild.firstChild)
    return r
  })
  const h = host()
  mount(() => Btns({}), h)
  click(h.querySelector('.s'))
  await tick()
  assert.ok(h.querySelector('.s'), 'still there')
  assert.equal(h.querySelector('i').textContent, '1') // its state still works
  const t = () => doc.body.querySelector('.brain-toast')
  assert.equal(t().textContent, 'Something went wrong. Try again.')
  click(h.querySelector('.a'))
  await tick(5)
  assert.equal(t().textContent, 'The server is busy, try again in a minute.')
  assert.equal(t().getAttribute('role'), 'status')
  assert.ok(reported.includes('sync handler') && reported.includes('The server is busy, try again in a minute.'))
  assert.equal(fb(h), null)
})

test('nesting: the nearest boundary wins (a failing child shows its fallback inside an intact parent)', async () => {
  const boom = sig(false)
  const Child = guard(() => {
    const r = tpl('<em>child</em>')()
    fx(() => { if (boom.v) throw new Error('child broke') })
    return r
  }, 'child fallback')
  const Parent = guard(() => {
    const r = tpl('<section><h2>parent</h2><!></section>')()
    insert(r, Child({}), r.lastChild)
    return r
  })
  const h = host()
  mount(() => Parent({}), h)
  boom.v = true
  await tick()
  assert.ok(h.querySelector('h2'), 'parent intact')
  assert.equal(h.querySelectorAll('.brain-fallback').length, 1)
  assert.match(fb(h).textContent, /^child fallback/)
})

test('Retry remounts only that component; 3 failures in 10 s leave the fallback without the button', () => {
  let attempts = 0
  const Flaky = guard(() => {
    attempts++
    if (attempts < 2) throw new Error('first try fails')
    return tpl('<p class="back">back</p>')()
  })
  const h = host()
  mount(() => Flaky({}), h)
  click(fb(h).querySelector('button'))
  assert.ok(h.querySelector('.back'))
  assert.equal(fb(h), null)
  const Always = guard(() => { throw new Error('always') })
  const h2 = host()
  mount(() => Always({}), h2)
  click(fb(h2).querySelector('button'))
  click(fb(h2).querySelector('button'))
  assert.ok(fb(h2), 'still the fallback')
  assert.equal(fb(h2).querySelector('button'), null) // no retry loop
})

test('SSR: the fallback is in the HTML and the error is collected for the server log; unmount removes the fallback', () => {
  globalThis.__brainSSR = []
  const Bad = guard(() => { throw new Error('ssr broken') })
  Bad.f = 'comp/bad.brain'
  const h = host()
  const stop = mount(() => [guard(Ok)({}), Bad({})], h)
  assert.match(h.innerHTML, /<div role="alert" class="brain-fallback">This section could not load\./)
  assert.equal(globalThis.__brainSSR.length, 1)
  assert.equal(globalThis.__brainSSR[0].comp, 'comp/bad.brain')
  assert.equal(globalThis.__brainSSR[0].e.message, 'ssr broken')
  globalThis.__brainSSR = null
  stop()
  assert.equal(h.innerHTML, '')
})

test('guard() and dev() keep every static of the component (seo, head, h, f…) visible on the wrapper', async () => {
  const { dev } = await sibling('brain-dev.js')
  const View = () => tpl('<p>v</p>')()
  Object.assign(View, { h: 'v1', f: 'views/v.brain', seo: p => ({ title: 'T ' + p.x }), head: () => '<meta name="x">', extra: 7 })
  const W = dev(guard(View))
  assert.equal(W.h, 'v1')
  assert.equal(W.f, 'views/v.brain')
  assert.deepEqual(W.seo({ x: 1 }), { title: 'T 1' })
  assert.equal(W.head(), '<meta name="x">')
  assert.equal(W.extra, 7)
  assert.equal(W.C.C, View)
})

test("a cancelled request (kind 'aborted') in a handler shows no toast and is not reported", async () => {
  const before = reported.length
  const t = () => doc.body.querySelector('.brain-toast')
  if (t()) t().textContent = ''
  const C = guard(() => {
    const r = tpl('<button class="x">go</button>')()
    on(r, 'click', async () => {
      await tick()
      throw Object.assign(new Error('The request was cancelled.'), { kind: 'aborted', code: 'api' })
    })
    return r
  })
  const h = host()
  mount(() => C({}), h)
  click(h.querySelector('.x'))
  await tick(5)
  assert.equal(reported.length, before) // nothing reported
  assert.ok(!t() || t().textContent === '') // no toast
  assert.equal(fb(h), null)
})

test('#90 breaker: an instance over its effect-runs budget is stopped and reported (B0908); its neighbors keep running', async () => {
  const n = sig(0)
  let runs = 0
  const Loop = guard(() => {
    const r = tpl('<b class="loop"><!></b>')()
    insert(r, () => (runs++, n.v), r.firstChild)
    return r
  }, null, { runs: 2 }) // 30 runs in well under a second: over 10× the budget
  Loop.f = 'comp/loop.brain'
  const ok = sig(0)
  const Fine = guard(() => {
    const r = tpl('<i class="fine"><!></i>')()
    insert(r, () => ok.v, r.firstChild)
    return r
  })
  const h = host()
  mount(() => [Loop({}), Fine({})], h)
  const before = reported.length
  for (let i = 1; i <= 30; i++) {
    n.v = i
    await tick()
  }
  assert.ok(!h.querySelector('.loop'), 'stopped')
  assert.match(fb(h).textContent, /^This part stopped to protect the page\.$/) // no Retry: it would trip again
  assert.ok(runs <= 23, String(runs)) // stopped right after crossing 10× the budget
  assert.ok(reported.slice(before).some(m => /B0908: comp\/loop\.brain crossed its runs budget \(21 > 2\)/.test(m)), reported.filter(m => /B0908/.test(m)).join(' | '))
  ok.v = 7
  await tick()
  assert.equal(h.querySelector('.fine').textContent, '7') // the rest of the page keeps running
})

test('#90 breaker: list rows budget (nodes and rows are measured after each flush) and work per flush', async () => {
  const n = sig(0)
  const Grow = guard(() => {
    const r = tpl('<ul class="grow"><!></ul>')()
    insert(r, () => Array.from({ length: n.v }, () => doc.createElement('li')), r.firstChild)
    return r
  }, null, { rows: 30, runs: 1e6 })
  const h = host()
  mount(() => Grow({}), h)
  for (let i = 1; i <= 70 && h.querySelector('.grow'); i++) {
    n.v = i
    await tick()
  }
  assert.ok(!h.querySelector('.grow'), 'stopped')
  assert.ok(reported.some(m => /crossed its rows budget \(\d+ > 30\)/.test(m)), reported.filter(m => /B0908/.test(m)).join(' | '))
  const k = sig(0)
  const Slow = guard(() => {
    const r = tpl('<p class="slow"><!></p>')()
    insert(r, () => {
      const v = k.v, t = Date.now()
      if (v) while (Date.now() - t < 15);
      return v
    }, r.firstChild)
    return r
  }, null, { ms: 5 })
  const h2 = host()
  mount(() => Slow({}), h2)
  k.v = 1 // one slow flush: a busy device, not runaway work
  await tick()
  assert.ok(h2.querySelector('.slow'), 'one slow flush does not trip')
  for (let i = 2; i <= 4 && h2.querySelector('.slow'); i++) {
    k.v = i
    await tick()
  }
  await tick()
  assert.ok(!h2.querySelector('.slow'), 'stopped')
  assert.ok(reported.some(m => /crossed its ms budget/.test(m)))
})

test('QA #176: an effect that fails while the component builds (first mount and Retry with the fault still on) leaves only the fallback', async () => {
  let broken = true
  const Inner = guard(() => {
    const r = tpl('<p class="half">inner <!></p>')()
    insert(r, () => {
      if (broken) throw new Error('fx broken at mount')
      return 'ok'
    }, r.lastChild)
    return r
  })
  const h = host()
  mount(() => Inner({}), h)
  assert.ok(!h.querySelector('.half'), 'no half-built Inner on first mount')
  assert.ok(fb(h))
  click(fb(h).querySelector('button')) // Retry with the fault still active
  assert.ok(!h.querySelector('.half'), 'no half-built Inner after Retry')
  assert.equal(h.querySelectorAll('.brain-fallback').length, 1)
  broken = false
  click(fb(h).querySelector('button'))
  assert.equal(h.querySelector('.half').textContent, 'inner ok')
  assert.ok(!fb(h))
})

test('QA #176: without brain-err, a boundary-caught error reaches window error listeners through reportError', () => {
  const seen = []
  globalThis.reportError = e => seen.push(e.message)
  const Bad = guard(() => { throw new Error('render broken for the dev client') })
  mount(() => Bad({}), host())
  delete globalThis.reportError
  assert.deepEqual(seen, ['render broken for the dev client'])
})

test('#90 breaker: a big legitimate burst (one table update of many rows) does not trip it', async () => {
  const items = sig(Array.from({ length: 300 }, (_, i) => i))
  const Big = guard(() => {
    const r = tpl('<ul class="big"></ul>')()
    for (let i = 0; i < 300; i++) {
      const li = doc.createElement('li')
      insert(li, () => items.v[i], null)
      r.appendChild(li)
    }
    return r
  }, null, { runs: 100 }) // 300 runs in one flush: 3× the budget, a burst
  const h = host()
  mount(() => Big({}), h)
  items.v = items.v.map(x => x + 1)
  await tick()
  assert.ok(h.querySelector('.big'), 'still there')
  assert.equal(h.querySelector('.big').firstChild.textContent, '1')
})

test('QA #290: create 1,000 rows, clear, run, ten times (jsfb) never trips the runs budget: many effects once is not a loop', async () => {
  const data = sig([])
  const rows = n => Array.from({ length: n }, (_, i) => ({ id: i + Math.random(), label: 'row ' + i }))
  const Table = guard(() => {
    const t = tpl('<table class="jsfb"><tbody><!></tbody></table>')()
    list(t.firstChild, () => data.v, r => r.id, r => {
      const tr = tpl('<tr><td><!></td><td><!></td></tr>')()
      insert(tr.firstChild, () => r.v.id, tr.firstChild.firstChild)
      insert(tr.lastChild, () => r.v.label, tr.lastChild.firstChild)
      return tr
    }, t.firstChild.firstChild)
    return t
  })
  Table.f = 'views/jsfb.brain'
  const h = host(), before = reported.length
  mount(() => Table({}), h)
  for (let i = 0; i < 10; i++) {
    data.v = rows(1000)
    await tick()
    data.v = []
    await tick()
    data.v = rows(1000)
    await tick()
  }
  assert.ok(h.querySelector('.jsfb'), 'still there')
  assert.equal(h.querySelectorAll('tr').length, 1000)
  assert.ok(!reported.slice(before).some(m => /B0908/.test(m)), reported.slice(before).join(' | '))
})

test('QA #290: a real loop (an fx that writes what it reads, through a microtask) still trips at the default budget', async t => {
  // A component created near the process clock boundary still gets a full first window.
  let clock = 999
  t.mock.method(performance, 'now', () => clock)
  const n = sig(0)
  const Spin = guard(() => {
    const r = tpl('<b class="spin"></b>')()
    fx(() => {
      const v = n.v
      if (v === 1500) clock = 1001
      queueMicrotask(() => (n.v = v + 1))
    })
    return r
  })
  Spin.f = 'comp/spin.brain'
  const h = host(), before = reported.length
  mount(() => Spin({}), h)
  await tick(20)
  assert.ok(!h.querySelector('.spin'), 'stopped')
  assert.ok(reported.slice(before).some(m => /B0908: comp\/spin\.brain crossed its runs budget \(10001 > 1000\)/.test(m)), reported.slice(before).join(' | '))
})

test('QA #303: 1,000 rows then 10,000 (a legitimate big list) stays under the default nodes budget; a runaway still trips it', async () => {
  const data = sig([])
  const rows = n => Array.from({ length: n }, (_, i) => ({ id: i + Math.random(), label: 'row ' + i }))
  const Table = guard(() => {
    const t = tpl('<table class="big10k"><tbody><!></tbody></table>')()
    list(t.firstChild, () => data.v, r => r.id, r => {
      const tr = tpl('<tr><td><!></td><td><a><!></a></td><td><a>x</a></td></tr>')()
      insert(tr.firstChild, () => r.v.id, tr.firstChild.firstChild)
      insert(tr.childNodes[1].firstChild, () => r.v.label, tr.childNodes[1].firstChild.firstChild)
      return tr
    }, t.firstChild.firstChild)
    return t
  })
  Table.f = 'views/home.brain'
  const h = host(), before = reported.length
  mount(() => Table({}), h)
  data.v = rows(1000)
  await tick()
  data.v = rows(10000)
  for (let i = 0; i < 200 && h.querySelectorAll('tr').length < 10000; i++) await tick() // created in slices
  assert.equal(h.querySelectorAll('tr').length, 10000)
  assert.ok(!reported.slice(before).some(m => /B0908/.test(m)), reported.slice(before).join(' | '))
  // runaway growth (an effect adding rows on every tick) still crosses the nodes budget (a low one: the shim is slow)
  const n = sig(0)
  const Grow = guard(() => {
    const r = tpl('<ul class="runaway"><!></ul>')()
    insert(r, () => Array.from({ length: n.v * 100 }, () => doc.createElement('li')), r.firstChild)
    return r
  }, null, { nodes: 3000, rows: 1e9, ms: 1e9 }) // only the nodes budget can stop it here
  const h2 = host()
  mount(() => Grow({}), h2)
  for (let i = 1; i <= 60 && h2.querySelector('.runaway'); i++) (n.v = i), await tick()
  assert.ok(!h2.querySelector('.runaway'), 'stopped')
  assert.ok(reported.some(m => /crossed its nodes budget \(\d+ > 3000\)/.test(m)))
})
