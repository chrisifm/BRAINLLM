// Reclassifies a saved run without generating again. Usage: node bench/llm/analyze.mjs results/claude-haiku-5-5.json
// Classes: ahead = SPEC documents it and the checker does not yet (toolchain gap) · token = var(--x) missing from the scaffold
// · a11y = B04xx/B05xx · lang = LLM error.
import { readFileSync, existsSync } from 'node:fs'
import { dirname, join, basename } from 'node:path'
const AHEAD = /`(form|resource|api|provide|use|key|store|bus|sanitize)` is not declared|bind:value` only goes on HTML elements|`bind:api` does not exist|bind:value` needs the name of a `sig`/
// form(): `bind:value={f.v.x}` is the SPEC "Forms" API; until #38 the checker rejects it → gap, not an LLM error.
const FORM_BIND = /bind:\w+=\{\w+\.v\./
let SRC = {}
const cls = (d) => ((d.level ?? 'error') !== 'error' ? 'nonerror' : AHEAD.test(d.msg) || (d.code === 'B0204' && FORM_BIND.test(SRC[d.line] || '')) ? 'ahead' : d.code === 'B0302' && /token `--/.test(d.msg) ? 'token' : /^B0[45]/.test(d.code) ? 'a11y' : 'lang')
const input = process.argv[2]
if (!input || input === '--help' || input === '-h') { console.log('Usage: node bench/llm/analyze.mjs bench/llm/results/<tag>.json'); process.exit(input ? 0 : 2) }
if (!existsSync(input)) { console.error(`no such run file: ${input} (bench/llm/results/ holds the runs written by run.mjs)`); process.exit(2) }
const { summary, rows } = JSON.parse(readFileSync(input, 'utf8'))
const ok = rows.filter((r) => !r.infraFail)
const runDir = join(dirname(input), basename(input, '.json'))
const srcOf = (r) => { const f = join(runDir, r.id, 'try0.brain'); SRC = existsSync(f) ? Object.fromEntries(readFileSync(f, 'utf8').split('\n').map((l, i) => [i + 1, l])) : {} }
const passIf = (allowed) => ok.filter((r) => { srcOf(r); return r.tries[0].diags.every((d) => allowed.includes(cls(d))) }).length
const by = {}; for (const r of ok) { srcOf(r); for (const d of r.tries[0].diags) { const k = `${cls(d)} ${d.code} ${d.msg}`; by[k] = (by[k] || 0) + 1 } }
console.log(`${summary.model} · mode ${summary.mode ?? 'spec'} · spec ${summary.specSha} (${summary.specBytes} B) · tokens ${summary.tokensSha ?? '?'} · stable ${summary.stable ?? 'no guard'} · n=${ok.length}`)
console.log(`1st try without diagnostics: ${passIf([])}/${ok.length}`)
console.log(`+ if the scaffold had the guessed tokens: ${passIf(['token'])}/${ok.length}`)
console.log(`+ and if the checker implemented what SPEC already documents: ${passIf(['token', 'ahead'])}/${ok.length}`)
console.log(`final after ≤${summary.fix} fixes: ${summary.finalPass} · mean iterations ${summary.meanIterations} · USD ${summary.costUSD}`)
console.log('\noccurrences on the 1st try:'); for (const [k, v] of Object.entries(by).sort((a, b) => b[1] - a[1])) console.log(String(v).padStart(4), k)

// Taxonomy by cause (INTERNALS §21): each class with its preferred structural fix.
// Level: 1 make it impossible · 2 normalize (info) · 3 scaffold/template · 4 spec line · E stays an error with a fix (needs judgment).
const TAX = [
  [/^B0302$/, 'missing token', '2 normalize exact synonym → canonical token; if there is no unique synonym, E with the closest one'],
  [/^B0119$/, 'import of a missing file', 'E (needs judgment); mitigate with 3: `brain map` in the context (mcp mode)'],
  [/^B0203$/, 'component not imported', '1 ui kit and project components resolved by name without import'],
  [/^B020[45]$/, 'bind on something that is not a sig', '1 accept what the model writes (field, form, ~) if the semantics are unique'],
  [/^B040[36]$/, 'control without label / button without text', 'E (alt/label need judgment); 3 `brain add` ships Field with a label'],
  [/^B040[45]$/, 'h1/main hierarchy', '3 the scaffold view already ships <main><h1>; E if the model adds another'],
  [/^B0408$/, 'token contrast', '1 expose only AA pairs in the spec; 4 if it persists'],
  [/^B0501$/, 'view without meta seo', '3 `brain add view` ships <meta seo>'],
  [/^B01/, 'forbidden BrainScript syntax', '2 if the fix is unique (var→let, console→log); otherwise E'],
  [/^B00/, 'file structure', '2 if the block order can be reordered without loss; otherwise E'],
  [/^B03/, 'CSS (global, reduced-motion)', '2 wrap animations in reduced-motion; E on global selectors'],
  [/.*/, 'unclassified', 'review by hand'],
]
const tax = {}
for (const r of ok) { srcOf(r); for (const d of r.tries[0].diags) {
  const [, name, fix] = TAX.find(([re]) => re.test(d.code)); const k = name
  tax[k] ??= { name, fix, occ: 0, views: new Set(), codes: new Set(), ahead: 0 }
  tax[k].occ++; tax[k].views.add(r.id); tax[k].codes.add(d.code); if (cls(d) === 'ahead') tax[k].ahead++
} }
console.log(`\nmetrics: delivery (after check+fix) ${summary.finalPass} · first pass ${passIf([])}/${ok.length}`)
console.log('\n| class | views | occurrences | codes | of which SPEC>checker | §21 fix |\n|---|---:|---:|---|---:|---|')
for (const t of Object.values(tax).sort((a, b) => b.views.size - a.views.size)) console.log(`| ${t.name} | ${t.views.size} | ${t.occ} | ${[...t.codes].join(' ')} | ${t.ahead} | ${t.fix} |`)
