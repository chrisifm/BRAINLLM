// Release gate (#11): the "passes brain check on the first try" rate must not drop against the accepted baseline.
// Usage: node bench/llm/gate.mjs results/<run>.json [--accept]   → exit 1 if it drops or if the run is not stable.
// Legacy baselines use model+mode; Codex cohorts additionally isolate provider and explicit reasoning effort.
// --accept updates it with this run (only if the gate passes). With n=20, 1 view of noise is tolerated (−5 pp).
import { readFileSync, writeFileSync, existsSync } from 'node:fs'
import { join, dirname, relative } from 'node:path'
import { fileURLToPath } from 'node:url'
import {cohortKey,validateRun} from './compare.mjs'

const here = dirname(fileURLToPath(import.meta.url))
const file = process.argv[2]
const accept = process.argv.includes('--accept')
const TOLERANCE = 1
if (!file || file === '--help' || file === '-h') { console.log('Usage: node bench/llm/gate.mjs bench/llm/results/<run>.json [--accept]'); process.exit(file ? 0 : 2) }
if (!existsSync(file)) { console.error(`no such run file: ${file} (bench/llm/results/ holds the runs written by run.mjs)`); process.exit(2) }
let run
try {run=JSON.parse(readFileSync(file,'utf8'))} catch {console.error('Invalid run JSON');process.exit(1)}
const invalid=validateRun(run)
if(invalid.length){for(const problem of invalid)console.error(problem);process.exit(1)}
const s=run.summary,key=cohortKey(s)
const pass = Number(String(s.firstPass).split('/')[0])
const basePath = join(here, 'baseline.json')
let base
try {base=existsSync(basePath)?JSON.parse(readFileSync(basePath,'utf8')):{}} catch {console.error('Invalid baseline JSON');process.exit(1)}
if(!base||typeof base!=='object'||Array.isArray(base)){console.error('Invalid baseline object');process.exit(1)}
const prev = base[key]
const fail = []
if(prev!==undefined&&(!prev||!Number.isInteger(prev.n)||prev.n<20||!Number.isInteger(prev.firstPass)||prev.firstPass<0||prev.firstPass>prev.n)){console.error('Invalid accepted baseline counts');process.exit(1)}
if(prev===undefined&&!accept)fail.push('no accepted baseline for this cohort; review and explicitly accept a complete reference run first')
if (prev && pass / s.n < (prev.firstPass - TOLERANCE) / prev.n) fail.push(`drop from ${prev.firstPass}/${prev.n} (spec ${prev.specSha}) to ${pass}/${s.n} (spec ${s.specSha})`)
console.log(`${key}: ${pass}/${s.n} on the first try · baseline ${prev ? `${prev.firstPass}/${prev.n} (spec ${prev.specSha})` : 'none'} → ${fail.length ? 'FAIL' : 'PASS'}`)
for (const f of fail) console.log('  - ' + f)
if (!fail.length && accept && (!prev || pass/s.n >= prev.firstPass/prev.n)) {
  base[key] = { firstPass: pass, n: s.n, ...(s.provider==='codex'?{provider:s.provider,effort:s.effort}:{}), specSha: s.specSha, run: relative(here, file), at: s.at }
  writeFileSync(basePath, JSON.stringify(base, null, 1) + '\n')
  console.log('  baseline updated')
}
process.exit(fail.length ? 1 : 0)
