// Release gate (#11): the "passes brain check on the first try" rate must not drop against the accepted baseline.
// Usage: node bench/llm/gate.mjs results/<run>.json [--accept]   → exit 1 if it drops or if the run is not stable.
// The baseline lives in baseline.json, one entry per model+mode: { "<model>|<mode>": { firstPass, n, specSha, run, at } }.
// --accept updates it with this run (only if the gate passes). With n=20, 1 view of noise is tolerated (−5 pp).
import { readFileSync, writeFileSync, existsSync } from 'node:fs'
import { join, dirname, relative } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
const file = process.argv[2]
const accept = process.argv.includes('--accept')
const TOLERANCE = 1
if (!file || file === '--help' || file === '-h') { console.log('Usage: node bench/llm/gate.mjs bench/llm/results/<run>.json [--accept]'); process.exit(file ? 0 : 2) }
if (!existsSync(file)) { console.error(`no such run file: ${file} (bench/llm/results/ holds the runs written by run.mjs)`); process.exit(2) }
const { summary: s } = JSON.parse(readFileSync(file, 'utf8'))
const key = `${s.model}|${s.mode ?? 'spec'}`
const pass = Number(String(s.firstPass).split('/')[0])
const basePath = join(here, 'baseline.json')
const base = existsSync(basePath) ? JSON.parse(readFileSync(basePath, 'utf8')) : {}
const prev = base[key]
const fail = []
if (s.stable !== true) fail.push(`unstable run (spec ${s.specSha}→${s.specShaEnd}, tokens ${s.tokensSha}→${s.tokensShaEnd})`)
if (s.infraFail) fail.push(`${s.infraFail} generations with an infrastructure failure`)
if (prev && pass / s.n < (prev.firstPass - TOLERANCE) / prev.n) fail.push(`drop from ${prev.firstPass}/${prev.n} (spec ${prev.specSha}) to ${pass}/${s.n} (spec ${s.specSha})`)
console.log(`${key}: ${pass}/${s.n} on the first try · baseline ${prev ? `${prev.firstPass}/${prev.n} (spec ${prev.specSha})` : 'none'} → ${fail.length ? 'FAIL' : 'PASS'}`)
for (const f of fail) console.log('  - ' + f)
if (!fail.length && accept && (!prev || pass >= prev.firstPass)) {
  base[key] = { firstPass: pass, n: s.n, specSha: s.specSha, run: relative(here, file), at: s.at }
  writeFileSync(basePath, JSON.stringify(base, null, 1) + '\n')
  console.log('  baseline updated')
}
process.exit(fail.length ? 1 : 0)
