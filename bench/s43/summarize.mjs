// Summary of the §43 bench (task #202, INTERNALS §43.5), from the autonomy harness output in
// bench/autonomy/results/s43-*.json and the apps next to it.
//   (a) agent loop before vs after §43: the same five briefs (bench/autonomy/briefs.json), per model; "before" runs
//       with --deny brain_card,brain_impact,brain_why,brain_split. Regression = after slower than before in median
//       wall time → FAIL.
//   (b) large app (bench/s43/briefs-202.json): components ≥ 60 (brain map --json of the final app), bigReads = 0
//       (no brain_read of a file over 200 lines) and brain doctor exit 0 → PASS.
// A run that hung until the harness timeout is re-run once under `<tag>-r2`; the re-run replaces it and the hang is
// kept as `hungBefore`.
// Output: bench/s43/results/summary.json and summary.md (home directory written as ~).
import { execFileSync } from 'node:child_process'
import { existsSync, readFileSync, writeFileSync, mkdirSync } from 'node:fs'
import { homedir } from 'node:os'
import { join } from 'node:path'

const ROOT = new URL('../..', import.meta.url).pathname
const RES = join(ROOT, 'bench/autonomy/results')
const OUT = join(ROOT, 'bench/s43/results')
mkdirSync(OUT, { recursive: true })
const tilde = (s) => JSON.parse(JSON.stringify(s).split(homedir()).join('~'))
const MODELS = ['haiku', 'sonnet']
const S43_TOOLS = ['brain_card', 'brain_impact', 'brain_why', 'brain_split', 'brain_map']
const median = (xs) => { const v = xs.filter((x) => x != null).sort((a, b) => a - b); if (!v.length) return null; const m = v.length >> 1; return v.length % 2 ? v[m] : (v[m - 1] + v[m]) / 2 }
const toolCount = (row, t) => row.calls?.[`mcp__brain__${t}`] ?? 0

function load(tag) {
  const f = join(RES, `${tag}.json`)
  if (!existsSync(f)) return null
  const r = JSON.parse(readFileSync(f, 'utf8'))
  const r2f = join(RES, `${tag}-r2.json`)
  const r2 = existsSync(r2f) ? JSON.parse(readFileSync(r2f, 'utf8')) : null
  const rows = r.rows.map((orig) => {
    const again = r2?.rows.find((x) => x.id === orig.id)
    const row = again ?? orig
    return {
      id: row.id, run: again ? `${tag}-r2` : tag,
      hungBefore: again ? { s: Math.round(orig.ms / 1000), toolCalls: orig.toolCalls, doctorOk: orig.doctorOk, timedOut: orig.timedOut } : undefined,
      done: row.done, doctorOk: row.doctorOk, doctorErrors: row.doctor?.errors ?? null, views: row.views, pages: row.pages,
      s: Math.round(row.ms / 1000), timedOut: row.timedOut, toolCalls: row.toolCalls, turns: row.turns,
      inputTokens: row.inputTokens, outputTokens: row.outputTokens, cost: row.cost ?? 0,
      reads: row.reads ?? null, bigReads: row.bigReads ?? null, maxReadLines: row.maxReadLines ?? null,
      doctorCalls: row.doctorCalls ?? null, firstDoctorOk: row.firstDoctorOk ?? null, stuckLoops: row.stuckLoops,
      s43Calls: Object.fromEntries(S43_TOOLS.map((t) => [t, toolCount(row, t)])),
      deniedSeen: (row.tools ?? []).filter((t) => (r.deny ?? []).includes(t)),
      bigReadList: (row.readList ?? []).filter((x) => x.big).map((x) => ({ path: x.path, lines: x.lines, paged: x.paged })),
    }
  })
  return { tag, brainSha: r.brainSha, specSha: r.specSha, deny: r.deny ?? [], at: r.at, rows }
}

// (a) agent loop
const loop = []
// `aa` = a replica of "after" (a first "before" launch whose --deny was not applied, kept as an A/A noise reference)
for (const model of MODELS) for (const arm of ['before', 'after', 'aa']) {
  const r = load(`s43-${arm}-${model}`)
  if (!r) { if (arm !== 'aa') loop.push({ model, arm, missing: true }); continue }
  const sum = (k) => r.rows.reduce((a, x) => a + (x[k] ?? 0), 0)
  loop.push({
    model, arm, tag: r.tag, brainSha: r.brainSha, specSha: r.specSha, deny: r.deny, n: r.rows.length,
    done: r.rows.filter((x) => x.done).length, firstDoctorOk: r.rows.filter((x) => x.firstDoctorOk).length,
    medianS: median(r.rows.map((x) => x.s)), medianSNoTimeout: median(r.rows.filter((x) => !x.timedOut).map((x) => x.s)), medianToolCalls: median(r.rows.map((x) => x.toolCalls)),
    s: sum('s'), toolCalls: sum('toolCalls'), inputTokens: sum('inputTokens'), outputTokens: sum('outputTokens'), cost: +sum('cost').toFixed(4),
    reads: sum('reads'), bigReads: sum('bigReads'), doctorCalls: sum('doctorCalls'),
    s43Calls: Object.fromEntries(S43_TOOLS.map((t) => [t, r.rows.reduce((a, x) => a + x.s43Calls[t], 0)])),
    timedOut: r.rows.filter((x) => x.timedOut).length, hungBefore: r.rows.filter((x) => x.hungBefore).length, rows: r.rows,
  })
}
const verdictA = MODELS.map((model) => {
  const b = loop.find((x) => x.model === model && x.arm === 'before'), a = loop.find((x) => x.model === model && x.arm === 'after')
  if (!b || !a || b.missing || a.missing) return { model, verdict: 'MISSING' }
  return {
    model, beforeMedianS: b.medianS, afterMedianS: a.medianS, beforeMedianToolCalls: b.medianToolCalls, afterMedianToolCalls: a.medianToolCalls,
    beforeDone: `${b.done}/${b.n}`, afterDone: `${a.done}/${a.n}`,
    verdict: a.medianS <= b.medianS ? 'PASS' : 'FAIL',
    beforeMedianSNoTimeout: b.medianSNoTimeout, afterMedianSNoTimeout: a.medianSNoTimeout,
    verdictNoTimeout: a.medianSNoTimeout <= b.medianSNoTimeout ? 'PASS' : 'FAIL',
    aaMedianS: loop.find((x) => x.model === model && x.arm === 'aa')?.medianS ?? null,
    toolCallsNote: a.medianToolCalls <= b.medianToolCalls ? 'after ≤ before' : 'after > before',
  }
})

// (b) large app
function appStats(dir, brain) {
  const out = { components: null, nonViewComponents: null, routes: null, b1010: null, maxCompLines: null }
  if (!existsSync(join(dir, 'app.brain'))) return out
  try {
    const m = JSON.parse(execFileSync(brain, ['map', '--json'], { cwd: dir, maxBuffer: 1 << 28 }).toString())
    out.components = m.total ?? m.components.length
    out.componentsListed = m.components.length
    out.nonViewComponents = m.components.filter((c) => !String(c.file).startsWith('views/')).length
    out.routes = m.routes.length
  } catch (e) { out.mapError = String(e.stderr ?? e).slice(0, 300) }
  try {
    let raw
    try { raw = execFileSync(brain, ['check', '--json'], { cwd: dir, maxBuffer: 1 << 28 }).toString() } catch (e) { raw = e.stdout.toString() }
    const j = JSON.parse(raw)
    const diags = Array.isArray(j) ? j : j.diags ?? []
    out.b1010 = diags.filter((d) => d.code === 'B1010').length
    out.b1010Files = [...new Set(diags.filter((d) => d.code === 'B1010').map((d) => d.file))]
    out.checkErrors = diags.filter((d) => (d.level ?? 'error') === 'error').length
  } catch (e) { out.checkError = String(e).slice(0, 300) }
  // the independent doctor again, three times: the heap-leak probe (B0601) is not deterministic
  out.doctorRepeats = [0, 1, 2].map(() => {
    try { execFileSync(brain, ['doctor', '--json'], { cwd: dir, maxBuffer: 1 << 28, timeout: 600_000 }); return { exit: 0 } } catch (e) {
      let j = null
      try { j = JSON.parse(e.stdout.toString()) } catch {}
      return { exit: e.status ?? -1, checkErrors: j?.check?.errors ?? null, testFailed: j?.test?.failed ?? null, buildOk: j?.build?.ok ?? null, leaks: (j?.heap?.leaks ?? []).map((l) => `${l.code} ${l.name} (${l.test})`) }
    }
  })
  return out
}
const SCAFFOLD = { components: 14, nonViewComponents: 2, routes: 8 } // `brain new` (sha in summary.json) before the agent writes anything
const large = []
const briefs202 = JSON.parse(readFileSync(join(ROOT, 'bench/s43/briefs-202.json'), 'utf8'))
for (const model of ['sonnet', 'haiku']) {
  const r = load(`s43-large-${model}`)
  if (!r) { large.push({ model, missing: true }); continue }
  for (const row of r.rows) {
    const brain = join(RES, row.run, 'brain')
    const st = appStats(join(RES, row.run, row.id), brain)
    const b = briefs202.find((x) => x.id === row.id)
    const pass = row.doctorOk && (st.components ?? 0) >= b.minComponents && row.bigReads === 0
    large.push({ model, brainSha: r.brainSha, ...row, ...st, minComponents: b.minComponents, verdict: pass ? 'PASS' : 'FAIL' })
  }
}

const report = tilde({
  at: new Date().toISOString(), task: '#202 (INTERNALS §43.5)', harness: 'bench/autonomy/run.mjs --mode mcp',
  bigReadLines: 200, scaffold: SCAFFOLD, verdictA, loop,
  verdictB: large.map((x) => ({ model: x.model, verdict: x.verdict ?? 'MISSING', components: x.components, bigReads: x.bigReads, doctorOk: x.doctorOk })),
  large,
})
writeFileSync(join(OUT, 'summary.json'), JSON.stringify(report, null, 1) + '\n')

const k = (n) => (n == null ? '–' : n >= 1e6 ? (n / 1e6).toFixed(2) + ' M' : n >= 1e3 ? Math.round(n / 1e3) + ' k' : String(n))
let md = `# §43 bench: agent loop before vs after, and a large app by components (task #202)\n\n`
md += `Harness: \`bench/autonomy/run.mjs --mode mcp\` (only the brain MCP server, no human turns, release binary frozen per run). `
md += `"before" = the same binary with \`brain_card\`, \`brain_impact\`, \`brain_why\` and \`brain_split\` denied (\`--deny\`, they are removed from the agent's tool list); "after" = all tools. `
md += `Done = independent \`brain doctor\` exit 0 and \`views/\` holds at least the routes the brief asks for. A read is big when \`brain_read\` returned a file over 200 lines (or a paged one, over 60 KB).\n\n`
md += `## (a) Agent loop, before vs after (bench/autonomy/briefs.json, 5 briefs)\n\n`
md += '| model | verdict | median time before → after | same, timed-out runs left out | median tool calls before → after | done before / after |\n|---|---|---:|---:|---:|---|\n'
for (const v of verdictA) md += v.verdict === 'MISSING' ? `| ${v.model} | MISSING | | | | |\n` : `| ${v.model} | **${v.verdict}** | ${v.beforeMedianS} s → ${v.afterMedianS} s | ${v.beforeMedianSNoTimeout} s → ${v.afterMedianSNoTimeout} s (${v.verdictNoTimeout}) | ${v.beforeMedianToolCalls} → ${v.afterMedianToolCalls} (${v.toolCallsNote}) | ${v.beforeDone} / ${v.afterDone} |\n`
const aa = verdictA.filter((v) => v.aaMedianS != null)
if (aa.length) md += `\nNoise reference (A/A): a second "after" run of the same briefs (arm \`aa\`) took a median of ${aa.map((v) => `${v.aaMedianS} s (${v.model}) against ${v.afterMedianS} s`).join(', ')}; a before/after gap smaller than the A/A gap is within run-to-run variance.\n`
md += '\n| model | arm | done | first doctor clean | doctor calls | tool calls | reads (big) | card / map / impact / why / split | tokens in / out | cost | total time |\n|---|---|---:|---:|---:|---:|---:|---|---|---:|---:|\n'
for (const r of loop) {
  if (r.missing) { md += `| ${r.model} | ${r.arm} | missing | | | | | | | | |\n`; continue }
  const c = r.s43Calls
  md += `| ${r.model} | ${r.arm} | ${r.done}/${r.n} | ${r.firstDoctorOk}/${r.n} | ${r.doctorCalls} | ${r.toolCalls} | ${r.reads} (${r.bigReads}) | ${c.brain_card} / ${c.brain_map} / ${c.brain_impact} / ${c.brain_why} / ${c.brain_split} | ${k(r.inputTokens)} / ${k(r.outputTokens)} | $${r.cost.toFixed(2)} | ${r.s} s |\n`
}
md += '\n### Per brief\n\n| model | arm | brief | done | time | tool calls | reads (big) | doctor calls | first doctor clean | tokens in | cost | doctor errors at the end |\n|---|---|---|---|---:|---:|---:|---:|---|---:|---:|---|\n'
for (const r of loop) for (const x of r.rows ?? []) md += `| ${r.model} | ${r.arm} | ${x.id}${x.hungBefore ? ' (re-run)' : ''} | ${x.done ? 'yes' : x.doctorOk ? 'no (routes)' : 'no'} | ${x.s} s${x.timedOut ? ' (timeout)' : ''} | ${x.toolCalls} | ${x.reads} (${x.bigReads}) | ${x.doctorCalls} | ${x.firstDoctorOk === null ? 'never ran' : x.firstDoctorOk ? 'yes' : 'no'} | ${k(x.inputTokens)} | $${x.cost.toFixed(3)} | ${x.doctorErrors ? x.doctorErrors.join(' ') : ''} |\n`
const hung = loop.flatMap((r) => (r.rows ?? []).filter((x) => x.hungBefore).map((x) => `${r.model}/${r.arm}/${x.id} (${x.hungBefore.s} s, ${x.hungBefore.toolCalls} tool calls)`))
if (hung.length) md += `\nHung until the harness timeout and re-run once (the table uses the re-run; the hang is in \`summary.json\`, \`hungBefore\`): ${hung.join(', ')}.\n`
md += `\n## (b) Large app by components (bench/s43/briefs-202.json, 11 pages, 60+ components)\n\n`
md += '| model | verdict | doctor 0 | components (map total) | non-view components | routes | reads (big) | max lines read | card / map / split calls | tool calls | tokens in / out | cost | time | B1010 at the end | doctor again ×3 (exit 0) |\n|---|---|---|---:|---:|---:|---:|---:|---|---:|---|---:|---:|---:|---:|\n'
for (const x of large) {
  if (x.missing) { md += `| ${x.model} | MISSING | | | | | | | | | | | | |\n`; continue }
  md += `| ${x.model}${x.hungBefore ? ` (re-run; first run: ${x.hungBefore.s} s, timeout)` : ""} | **${x.verdict}** | ${x.doctorOk ? 'yes' : `no (${(x.doctorErrors ?? []).join(' ') || 'no check errors'})`} | ${x.components ?? '–'} | ${x.nonViewComponents ?? '–'} | ${x.routes ?? '–'} | ${x.reads} (${x.bigReads}) | ${x.maxReadLines} | ${x.s43Calls.brain_card} / ${x.s43Calls.brain_map} / ${x.s43Calls.brain_split} | ${x.toolCalls} | ${k(x.inputTokens)} / ${k(x.outputTokens)} | $${x.cost.toFixed(2)} | ${x.s} s${x.timedOut ? ' (timeout)' : ''} | ${x.b1010 ?? '–'} | ${(x.doctorRepeats ?? []).filter((d) => d.exit === 0).length}/${(x.doctorRepeats ?? []).length} |\n`
}
const flaky = large.flatMap((x) => (x.doctorRepeats ?? []).filter((d) => d.exit !== 0).map((d) => `${x.model}: ${d.leaks?.join(', ') || `exit ${d.exit}`}`))
if (flaky.length) md += `\nDoctor failures when run again on the unchanged final app: ${flaky.join('; ')}.\n`
md += `\nThe fresh \`brain new\` scaffold already holds ${SCAFFOLD.components} map components (${SCAFFOLD.nonViewComponents} outside views/) and ${SCAFFOLD.routes} routes; the counts above include them.\n`
const used = (t) => [...loop, ...large].reduce((a, r) => a + (r.s43Calls?.[t] ?? 0), 0)
md += `\n## Notes\n\n`
md += `- §43 tools the agents actually called over all runs: brain_card ${used('brain_card')}, brain_map ${used('brain_map')}, brain_why ${used('brain_why')}, brain_impact ${used('brain_impact')}, brain_split ${used('brain_split')}. SPEC.md does not mention them; agents learn of them only from the tool descriptions.\n`
md += `- Every timeout in this bench is a tool that never answered, not an agent that kept working: \`brain_eval\` with an awaited timer after clicking a submit whose handler calls \`navigate()\` (the CDP call has no deadline and blocks the MCP server), and once \`brain_navigate\` (aborted by Claude Code after 1800 s of silence). These tools exist in both arms; a hang inflates whichever arm it lands in.\n`
md += `- Big reads (over 200 lines) were 0 in every run, both arms: with the default scaffold and these briefs no agent ever opened a file that large, so "0 big reads" does not separate the arms.\n`
writeFileSync(join(OUT, 'summary.md'), md)
console.log(md)
