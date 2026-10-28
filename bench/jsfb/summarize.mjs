#!/usr/bin/env node
// Summarize js-framework-benchmark results (webdriver-ts/results/*.json) into a compact JSON and a Markdown table.
// Statistics follow webdriver-ts-results/src/Common.ts at the pinned commit:
//   value  = median (default display mode), CPU uses the "total" duration
//   factor = value / fastest value among the compared frameworks
//   CPU    = weighted geometric mean of factors with the runner's weights; memory/startup/size = unweighted.
// The "vs vanillajs" columns use vanillajs-keyed as the denominator instead of the fastest framework.
// Usage: node bench/jsfb/summarize.mjs <jsfb>/webdriver-ts/results bench/jsfb/results/summary.json
import fs from 'node:fs'
import path from 'node:path'

const [dir, out] = process.argv.slice(2)
if (!dir || !out) {
  console.error('usage: summarize.mjs <results-dir> <summary.json>')
  process.exit(2)
}

const CPU = ['01_run1k', '02_replace1k', '03_update10th1k_x16', '04_select1k', '05_swap1k', '06_remove-one-1k', '07_create10k', '08_create1k-after1k_x2', '09_clear1k_x8']
const CPU_WEIGHTS = [0.64280248137063, 0.5607178150466176, 0.5643800750716564, 0.1925635870170522, 0.13200612879341714, 0.5277091212292658, 0.5644449600965534, 0.5508359820582848, 0.4225836631419211]

const rows = {}
for (const f of fs.readdirSync(dir).filter(f => f.endsWith('.json'))) {
  const r = JSON.parse(fs.readFileSync(path.join(dir, f), 'utf8'))
  const fw = r.framework
  rows[fw] ??= {}
  const key = r.type === 'cpu' ? 'total' : Object.keys(r.values)[0]
  for (const [k, v] of Object.entries(r.values)) {
    const name = k === key || r.type !== 'cpu' && k === 'DEFAULT' ? r.benchmark : `${r.benchmark}:${k}`
    if (r.type === 'cpu' && k !== 'total') continue
    rows[fw][name] = { type: r.type, median: v.median, mean: v.mean, stddev: v.stddev ?? v.standardDeviation, n: v.values?.length ?? 1 }
  }
}
const frameworks = Object.keys(rows).sort()
const benchmarks = [...new Set(frameworks.flatMap(f => Object.keys(rows[f])))].sort()
const typeOf = b => frameworks.map(f => rows[f][b]?.type).find(Boolean)
const vanilla = frameworks.find(f => f.startsWith('vanillajs'))

const factor = (b, f, base) => {
  const v = rows[f][b]?.median
  if (v == null) return null
  const d = base === 'fastest' ? Math.min(...frameworks.map(x => rows[x][b]?.median).filter(x => x != null)) : rows[base]?.[b]?.median
  return d ? v / d : null
}
const geo = (list, weights, f, base) => {
  let s = 0
  let w = 0
  list.forEach((b, i) => {
    const x = factor(b, f, base)
    if (x != null && x > 0) {
      s += (weights ? weights[i] : 1) * Math.log(x)
      w += weights ? weights[i] : 1
    }
  })
  return w ? Math.exp(s / (weights ? weights.reduce((a, c) => a + c, 0) : w)) : null
}

const groups = {
  cpu: CPU,
  memory: benchmarks.filter(b => typeOf(b) === 'memory'),
  // The runner tags 43_first-paint as type "size"; it is a startup metric (ms), so it is grouped separately.
  startup: benchmarks.filter(b => typeOf(b) === 'startup' || /first-paint/.test(b)),
  size: benchmarks.filter(b => typeOf(b) === 'size' && !/first-paint/.test(b))
}
const summary = { generatedFrom: path.relative(process.cwd(), path.resolve(dir)), statistic: 'median', frameworks: {} }
for (const f of frameworks) {
  summary.frameworks[f] = {
    medians: Object.fromEntries(benchmarks.map(b => [b, rows[f][b] ? +rows[f][b].median.toFixed(3) : null])),
    ci95: Object.fromEntries(CPU.filter(b => rows[f][b]).map(b => [b, +(1.959964 * (rows[f][b].stddev || 0) / Math.sqrt(rows[f][b].n)).toFixed(2)])),
    runs: Object.fromEntries(CPU.filter(b => rows[f][b]).map(b => [b, rows[f][b].n])),
    geomean: {
      cpuWeightedVsFastest: geo(CPU, CPU_WEIGHTS, f, 'fastest'),
      cpuWeightedVsVanillajs: vanilla ? geo(CPU, CPU_WEIGHTS, f, vanilla) : null,
      memoryVsFastest: geo(groups.memory, null, f, 'fastest'),
      startupVsFastest: geo(groups.startup, null, f, 'fastest'),
      sizeVsFastest: geo(groups.size, null, f, 'fastest')
    }
  }
}
fs.mkdirSync(path.dirname(out), { recursive: true })
fs.writeFileSync(out, JSON.stringify(summary, null, 1) + '\n')

// Markdown: frameworks ordered by CPU geomean.
const order = [...frameworks].sort((a, b) => summary.frameworks[a].geomean.cpuWeightedVsFastest - summary.frameworks[b].geomean.cpuWeightedVsFastest)
const short = f => f.replace(/-keyed$/, '')
const fmt = (x, d = 1) => (x == null ? '—' : x.toFixed(d))
const table = (title, list, digits) => {
  const lines = [`| ${title} | ${order.map(short).join(' | ')} |`, `|---|${order.map(() => '---:').join('|')}|`]
  for (const b of list) lines.push(`| ${b} | ${order.map(f => fmt(rows[f][b]?.median, digits)).join(' | ')} |`)
  return lines.join('\n')
}
const gm = (label, key) => `| **${label}** | ${order.map(f => '**' + fmt(summary.frameworks[f].geomean[key], 2) + '**').join(' | ')} |`
console.log(table('CPU benchmark (median ms)', CPU, 1))
console.log(gm('weighted geomean vs fastest', 'cpuWeightedVsFastest'))
console.log(gm('weighted geomean vs vanillajs', 'cpuWeightedVsVanillajs'))
console.log()
console.log(table('Memory (MB)', groups.memory, 2))
console.log(gm('geomean vs fastest', 'memoryVsFastest'))
console.log()
console.log(table('Startup / size', [...groups.startup, ...groups.size], 1))
