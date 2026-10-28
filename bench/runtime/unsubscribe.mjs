// One signal with N subscriber effects that are disposed one by one (RUNTIME's suggested worst case).
// Measures dispose time for creation order, reverse order and random order at N and 2N → scaling exponent.
// Usage: node bench/runtime/unsubscribe.mjs [path to brain.js]   (median of 5 runs, ms)
import { resolve } from 'node:path'
import { pathToFileURL } from 'node:url'

const here = new URL('.', import.meta.url).pathname
const url = pathToFileURL(resolve(process.argv[2] || `${here}../../runtime/brain.js`)).href
const { sig, fx } = await import(url)
const RUNS = 5

function shuffle(a, seed = 42) {
  for (let i = a.length - 1; i > 0; i--) { seed = (seed * 1103515245 + 12345) & 0x7fffffff; const j = seed % (i + 1); [a[i], a[j]] = [a[j], a[i]] }
  return a
}

function run(n, order) {
  const times = []
  for (let r = 0; r < RUNS; r++) {
    const s = sig(0)
    const disposers = Array.from({ length: n }, () => fx(() => void s.v))
    const seq = order === 'creation' ? disposers : order === 'reverse' ? [...disposers].reverse() : shuffle([...disposers])
    const t0 = performance.now()
    for (const d of seq) d()
    times.push(performance.now() - t0)
  }
  times.sort((a, b) => a - b)
  return times[RUNS >> 1]
}

const rows = []
for (const order of ['creation', 'reverse', 'random']) {
  const N = Number(process.env.N ?? 10_000), a = run(N, order), b = run(2 * N, order)
  rows.push({ order, N, ms_N: +a.toFixed(2), ms_2N: +b.toFixed(2), exponent: +Math.log2(b / a).toFixed(2) })
}
console.table(rows)
console.log(JSON.stringify({ node: process.version, runtime: url, rows }))
