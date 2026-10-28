// Measures the heap retained by the signal graph. Usage: node --expose-gc runtime/perf/heap.js [runtime path]
// Scenarios with N=10 000: sig only · chained sig→drv→fx (typical view graph) · one owner with N child effects.
import { resolve } from 'node:path'
import { pathToFileURL } from 'node:url'

if (!globalThis.gc) throw Error('run with --expose-gc')
const url = pathToFileURL(resolve(process.argv[2] || new URL('../brain.js', import.meta.url).pathname)).href
const { sig, drv, fx } = await import(url)
const N = 10_000, RUNS = 7
const used = () => (gc(), gc(), process.memoryUsage().heapUsed)

function measure(build) {
  const out = []
  for (let r = 0; r < RUNS; r++) {
    const before = used()
    const keep = build()
    out.push(used() - before)
    keep.length = 0
  }
  out.sort((a, b) => a - b)
  return Math.round(out[RUNS >> 1] / N) // median, bytes per unit
}

const scen = {
  'sig only (B/sig)': () => Array.from({ length: N }, (_, i) => sig(i)),
  'sig→drv→fx (B/trio)': () => {
    const k = []
    for (let i = 0; i < N; i++) {
      const s = sig(i), d = drv(() => s.v * 2)
      k.push(s, d, fx(() => void d.v))
    }
    return k
  },
  'child fx of an owner (B/child)': () => {
    const k = []
    k.push(fx(() => { for (let i = 0; i < N; i++) fx(() => {}) }))
    return k
  },
}
for (const [name, f] of Object.entries(scen)) console.log(name.padEnd(30), measure(f))
