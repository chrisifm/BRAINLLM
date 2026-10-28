// brain-worker.js (INTERNALS §35.5): serve(map) ↔ worker(url).call(name, args) over structured clone.
import { test } from 'node:test'
import assert from 'node:assert/strict'

const rt = await import('./rt.js')
const { serve, worker } = await rt.sibling('brain-worker.js')

// A fake Worker that runs the "worker module" in-process, with structured clone on both directions like the real one.
const modules = new Map()
let started = 0
globalThis.Worker = class {
  constructor(url, o) {
    assert.equal(o.type, 'module')
    started++
    const scope = {
      postMessage: m => {
        const c = structuredClone(m) // synchronous, like the real postMessage: throws inside serve's try
        queueMicrotask(() => this.onmessage({ data: c }))
      },
    }
    modules.get(String(url))(scope)
    this.postMessage = m => {
      const c = structuredClone(m) // throws DataCloneError like the real postMessage
      queueMicrotask(() => scope.onmessage({ data: c }))
    }
    this.terminate = () => {}
  }
}
modules.set('w:csv', scope =>
  serve(
    {
      parseCsv: s => s.split('\n').map(l => l.split(',')),
      slow: async ms => (await new Promise(r => setTimeout(r, ms)), ms),
      fails: () => { throw new RangeError('bad row 3') },
      fn: () => () => 1, // a function cannot be cloned back
      default: { mean: xs => xs.reduce((a, b) => a + b, 0) / xs.length },
    },
    scope
  )
)

test('call: lazy start, named and default.* exports, async results, concurrent ids', async () => {
  const w = worker('w:csv')
  assert.equal(started, 0)
  assert.deepEqual(await w.call('parseCsv', ['a,b\n1,2']), [['a', 'b'], ['1', '2']])
  assert.equal(started, 1)
  assert.equal(await w.call('default.mean', [[1, 2, 3, 6]]), 3)
  const [a, b] = await Promise.all([w.call('slow', [30]), w.call('slow', [5])])
  assert.deepEqual([a, b], [30, 5])
  assert.equal(started, 1) // one worker for all calls
})

test('errors: remote throw rethrown with name and message; unknown export; uncloneable args and results', async () => {
  const w = worker('w:csv')
  await assert.rejects(w.call('fails', []), e => e.name === 'RangeError' && e.message === 'bad row 3' && /bad row 3/.test(e.remoteStack))
  await assert.rejects(w.call('nope', []), /`nope` is not an exported function/)
  await assert.rejects(w.call('parseCsv', [() => 1]), e => e.name === 'DataCloneError')
  await assert.rejects(w.call('fn', []), e => e.name === 'DataCloneError')
  assert.deepEqual(await w.call('parseCsv', ['x']), [['x']]) // still usable afterwards
})
