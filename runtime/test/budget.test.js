import { test } from 'node:test'
import assert from 'node:assert/strict'
import { install } from '../shim/dom.js'
install()
const { sibling } = await import('./rt.js')
const { _rt } = await sibling('brain.js')
const { budgetEnter, budgetStep, budgetLeave, budgetSuspend, budgetResume, budgetClaim, budgetWatchWorker } = await sibling('brain-budget.js')
const meta = { file: 'comp/Counter.brain', component: 'Counter', line: 7, column: 3 }
const wait = ms => new Promise(resolve => setTimeout(resolve, ms))

test('checkpoint interrupts an owned endless iterator and preserves source location', () => {
  const f = budgetEnter(meta, null, { steps: 8 })
  let iterations = 0
  try {
    assert.throws(() => { for (;;) { budgetStep(f, meta); iterations++ } }, error => error.code === 'B0910' && error.file === meta.file && error.line === 7 && error.budget === 'steps')
    assert.equal(iterations, 8)
    assert.throws(() => budgetStep(f), { code: 'B0910' })
  } finally { budgetLeave(f) }
  const other = budgetEnter(meta)
  budgetStep(other)
  budgetLeave(other)
})

test('recursion rejects before native stack overflow and ordinary finally restores independent calls', () => {
  const root = budgetEnter(meta, null, { depth: 8 })
  let entered = 0
  function recurse() {
    const f = budgetEnter(meta)
    entered++
    try { recurse() } finally { budgetLeave(f) }
  }
  try { assert.throws(recurse, { code: 'B0910', budget: 'depth' }); assert.equal(entered, 7) }
  finally { budgetLeave(root) }
  const next = budgetEnter(meta); budgetStep(next); budgetLeave(next)
})

test('await suspension excludes IO delay, preserves rejection identity and closes on throw', async () => {
  const f = budgetEnter(meta, null, { ms: 10 })
  try {
    assert.equal(budgetResume(f, await budgetSuspend(f, wait(30).then(() => 42))), 42)
    const original = new Error('original')
    assert.throws(() => budgetResume(f, { ok: false, error: original }), { code: 'B0910' })
    const outcome = await budgetSuspend(f, Promise.reject(original))
    assert.throws(() => budgetResume(f, outcome), error => error === original)
  } finally { budgetLeave(f) }
})

test('resumed work still shares the total step budget and concurrent suspended frames do not mix roots', async () => {
  const a = budgetEnter(meta, null, { steps: 1 })
  budgetStep(a)
  const pa = budgetSuspend(a, Promise.resolve('a'))
  const b = budgetEnter({ ...meta, file: 'comp/Other.brain' }, null, { steps: 2 })
  const pb = budgetSuspend(b, Promise.resolve('b'))
  try { assert.throws(() => budgetResume(a, { ok: true, value: 'a' }), { code: 'B0910', budget: 'steps' }) }
  finally { budgetLeave(a) }
  assert.equal(budgetResume(b, await pb), 'b')
  budgetStep(b)
  budgetLeave(b)
  await pa
})

test('owner quotas reject before acquisition, release once, and dispose all owned resources', () => {
  let disposeRoot, owner, released = 0
  _rt().root(dispose => { disposeRoot = dispose; owner = _rt().owner() })
  const release = budgetClaim(owner, 'resources', 1, () => released++, meta, { resources: 1 })
  assert.throws(() => budgetClaim(owner, 'resources', 1, null, meta), { code: 'B0911', budget: 'resources' })
  release(); release()
  assert.equal(released, 1)
  budgetClaim(owner, 'resources', 1, () => released++, meta)
  disposeRoot()
  assert.equal(released, 2)
  assert.throws(() => budgetClaim(owner, 'resources', 1, null, meta), { code: 'B0911', budget: 'disposed-owner' })
})

test('separate owners have separate quotas and invalid/unowned claims fail closed', () => {
  const roots = []
  for (let n = 0; n < 2; n++) _rt().root(dispose => {
    roots.push(dispose)
    budgetClaim(_rt().owner(), 'rows', 2, null, meta, { rows: 2 })
    assert.throws(() => budgetClaim(_rt().owner(), 'rows', 1, null, meta), { budget: 'rows' })
    assert.throws(() => budgetClaim(_rt().owner(), 'rows', -1, null, meta), { budget: 'invalid-claim' })
  })
  roots.forEach(dispose => dispose())
  assert.throws(() => budgetClaim(null, 'resources', 1, null, meta), { budget: 'missing-owner' })
})

test('worker watchdog caps pending work and terminates only the supplied owned worker', async () => {
  let stopped = 0, unrelated = 0, error
  const watchdog = budgetWatchWorker({ terminate: () => stopped++ }, e => { error = e }, meta, { workerMs: 10, pending: 1 })
  const other = { terminate: () => unrelated++ }
  watchdog.track('first')
  assert.throws(() => watchdog.track('second'), { code: 'B0911', budget: 'pending' })
  await wait(25)
  assert.equal(stopped, 1)
  assert.equal(unrelated, 0)
  assert.equal(error.code, 'B0910')
  assert.equal(error.file, meta.file)
  watchdog.dispose()
  assert.equal(stopped, 1)
  assert.throws(() => watchdog.track('closed'), { budget: 'worker-disposed' })
  void other
})

test('completed worker calls clear deadlines and disposal clears all remaining timers', async () => {
  let stopped = 0, rejected = 0
  const watchdog = budgetWatchWorker({ terminate: () => stopped++ }, () => rejected++, meta, { workerMs: 10 })
  const done = watchdog.track(1)
  done(); done()
  await wait(20)
  assert.equal(stopped, 0)
  watchdog.track(2)
  watchdog.dispose()
  await wait(20)
  assert.equal(stopped, 1)
  assert.equal(rejected, 1)
})

test('real core signal hook rejects allocation before constructing another owned node', async () => {
  const { sig } = await sibling('brain.js')
  let disposeRoot
  _rt().root(dispose => {
    disposeRoot = dispose
    const owner = _rt().owner()
    budgetClaim(owner, 'signals', 0, null, meta, { signals: 2 })
    sig(1); sig(2)
    assert.throws(() => sig(3), { code: 'B0911', budget: 'signals' })
  })
  disposeRoot()
})

test('ledger cleanup permits an owned effect to rerun without retaining the old allocation count', async () => {
  const { sig, fx } = await sibling('brain.js')
  const trigger = sig(0)
  let disposeRoot, runs = 0
  _rt().root(dispose => {
    disposeRoot = dispose
    fx(() => {
      trigger.v
      const owner = _rt().owner()
      budgetClaim(owner, 'signals', 0, null, meta, { signals: 1 })
      sig(1)
      runs++
    })
  })
  trigger.v++
  await wait(0)
  assert.equal(runs, 2)
  disposeRoot()
})

test('real list core hook rejects excessive rows before invoking any row renderer', async () => {
  const { list, _err } = await sibling('brain.js')
  const errors = []
  let rendered = 0, disposeRoot
  _err(error => errors.push(error))
  try {
    _rt().root(dispose => {
      disposeRoot = dispose
      const host = document.createElement('div')
      list(host, () => new Array(100001).fill(0), x => x, () => { rendered++; return [] })
      assert.equal(host.childNodes.length, 1, 'only the list end marker exists')
    })
    assert.equal(rendered, 0)
    assert.equal(errors.length, 1)
    assert.equal(errors[0].code, 'B0911')
    assert.equal(errors[0].budget, 'rows')
  } finally { disposeRoot?.(); _err(null) }
})
