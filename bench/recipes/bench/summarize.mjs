// Summary of the recipes benchmark (task #83): recipes off vs on, per model, from the autonomy harness output
// (bench/autonomy/results/recipes-<off|on>-<haiku|sonnet>.json and the transcripts next to it).
// Extra metrics read from the transcripts:
//   firstDoctorOk   the agent's first brain_doctor call already exited clean (a true first pass)
//   firstCheckErrs  error-level diagnostics in the first brain_check/brain_doctor answer (doctor: check errors +
//                   failed tests + a failed build)
//   writeErrs       error-level diagnostics returned by brain_write/brain_edit over the whole run
//   doctorCalls     how many times the agent ran brain_doctor
//   routesOk        the routes the brief names exist (briefs-src.json `routes`, `a|b` = either path)
// Output: bench/recipes/results/summary.json and summary.md (home directory written as ~).
import { existsSync, readFileSync, writeFileSync } from 'node:fs'
import { homedir } from 'node:os'
import { join } from 'node:path'

const ROOT = new URL('../../..', import.meta.url).pathname
const RES = join(ROOT, 'bench/autonomy/results')
const OUT = join(ROOT, 'bench/recipes/results')
const src = JSON.parse(readFileSync(join(ROOT, 'bench/recipes/bench/briefs-src.json'), 'utf8'))
const tilde = (s) => JSON.parse(JSON.stringify(s).split(homedir()).join('~'))

const text = (c) => (typeof c.content === 'string' ? c.content : (c.content ?? []).map((x) => x.text ?? '').join(''))
const errs = (diags) => (diags ?? []).filter((d) => (d.level ?? 'error') === 'error').length

function transcript(file) {
  const out = { firstDoctorOk: null, firstCheckErrs: null, writeErrs: 0, writes: 0, doctorCalls: 0 }
  if (!existsSync(file)) return out
  const names = {}
  for (const line of readFileSync(file, 'utf8').split('\n')) {
    let e
    try { e = JSON.parse(line) } catch { continue }
    if (e.type === 'assistant') for (const c of e.message?.content ?? []) if (c.type === 'tool_use') names[c.id] = c.name.split('__').pop()
    if (e.type !== 'user') continue
    for (const c of e.message?.content ?? []) {
      if (c.type !== 'tool_result') continue
      const n = names[c.tool_use_id]
      let j
      try { j = JSON.parse(text(c)) } catch { continue }
      if (n === 'brain_write' || n === 'brain_edit') {
        out.writes++
        out.writeErrs += errs(j.diags)
      } else if (n === 'brain_check') {
        if (out.firstCheckErrs === null) out.firstCheckErrs = errs(Array.isArray(j) ? j : j.diags)
      } else if (n === 'brain_doctor') {
        out.doctorCalls++
        const e2 = (j.check?.errors ?? errs(j.check?.diags)) + (j.test?.failed ?? 0) + (j.build && j.build.ok === false ? 1 : 0)
        if (out.firstCheckErrs === null) out.firstCheckErrs = e2
        if (out.firstDoctorOk === null) out.firstDoctorOk = j.ok === true
      }
    }
  }
  return out
}

const routesOk = (app, routes) => routes.every((r) => r.split('|').some((p) => existsSync(join(app, p))))

const runs = []
for (const model of ['haiku', 'sonnet']) for (const variant of ['off', 'on']) {
  const tag = `recipes-${variant}-${model}`
  const f = join(RES, `${tag}.json`)
  if (!existsSync(f)) { runs.push({ model, variant, tag, missing: true }); continue }
  const r = JSON.parse(readFileSync(f, 'utf8'))
  // a brief whose agent hung on an MCP tool until the harness timeout (no result event, so no tokens or cost) is
  // re-run under `<tag>-r2`; the re-run replaces it and the hang is kept as `hungBefore`
  const r2f = join(RES, `${tag}-r2.json`)
  const r2 = existsSync(r2f) ? JSON.parse(readFileSync(r2f, 'utf8')) : null
  const rows = r.rows.map((orig) => {
    const again = r2?.rows.find((x) => x.id === orig.id)
    const row = again ?? orig
    const dir = again ? `${tag}-r2` : tag
    const b = src.find((x) => x.id === row.id)
    const t = transcript(join(RES, dir, `${row.id}.transcript.jsonl`))
    const ok = routesOk(join(RES, dir, row.id), b.routes)
    return {
      id: row.id, recipe: b.recipe, run: dir, hungBefore: again ? { s: Math.round(orig.ms / 1000), toolCalls: orig.toolCalls, doctorOk: orig.doctorOk } : undefined,
      done: row.done && ok, doctorOk: row.doctorOk, routesOk: ok, doctorErrors: row.doctor?.errors ?? null,
      toolCalls: row.toolCalls, inputTokens: row.inputTokens, outputTokens: row.outputTokens, cost: row.cost, s: Math.round(row.ms / 1000),
      timedOut: row.timedOut, stuckLoops: row.stuckLoops, ...t,
    }
  })
  const sum = (k) => rows.reduce((a, x) => a + (x[k] ?? 0), 0)
  runs.push({
    model, variant, tag, brainSha: r.brainSha, specSha: r.specSha, at: r.at, n: rows.length,
    done: rows.filter((x) => x.done).length, firstDoctorOk: rows.filter((x) => x.firstDoctorOk).length,
    toolCalls: sum('toolCalls'), inputTokens: sum('inputTokens'), outputTokens: sum('outputTokens'), cost: +sum('cost').toFixed(4), s: sum('s'),
    firstCheckErrs: sum('firstCheckErrs'), writeErrs: sum('writeErrs'), doctorCalls: sum('doctorCalls'),
    timedOut: rows.filter((x) => x.timedOut).length, hungBefore: rows.filter((x) => x.hungBefore).length, rows,
  })
}

const report = tilde({ at: new Date().toISOString(), harness: 'bench/autonomy/run.mjs --mode mcp', briefs: 'bench/recipes/bench/briefs-83.json (off) · briefs-83-on.json (on)', runs })
writeFileSync(join(OUT, 'summary.json'), JSON.stringify(report, null, 1) + '\n')

const k = (n) => (n >= 1e6 ? (n / 1e6).toFixed(2) + ' M' : n >= 1e3 ? Math.round(n / 1e3) + ' k' : String(n))
let md = `# Recipes benchmark: with vs without recipes (task #83)\n\nHarness: \`bench/autonomy/run.mjs --mode mcp\` (only the brain MCP server, no human turns). Six briefs that map to a recipe; "on" inlines the recipe's files in the brief (the harness cannot pre-seed files), "off" is the plain brief. Done = independent \`brain doctor\` exit 0 and the routes the brief names exist.\n\n`
md += '| model | recipes | done | first doctor clean | errors at first check | write/edit errors | doctor calls | tool calls | tokens in / out | cost | time |\n|---|---|---:|---:|---:|---:|---:|---:|---|---:|---:|\n'
for (const r of runs) {
  if (r.missing) { md += `| ${r.model} | ${r.variant} | missing | | | | | | | | |\n`; continue }
  md += `| ${r.model} | ${r.variant} | ${r.done}/${r.n} | ${r.firstDoctorOk}/${r.n} | ${r.firstCheckErrs} | ${r.writeErrs} | ${r.doctorCalls} | ${r.toolCalls} | ${k(r.inputTokens)} / ${k(r.outputTokens)} | $${r.cost.toFixed(2)} | ${r.s} s |\n`
}
const hung = runs.flatMap((r) => (r.rows ?? []).filter((x) => x.hungBefore || x.timedOut).map((x) => `${r.model}/${r.variant}/${x.id}`))
if (hung.length) md += `\nHung on an MCP tool until the 1500 s harness timeout (no result event, so no tokens or cost) and re-run once: ${hung.join(', ')}. The table uses the re-run; the hang is in \`summary.json\` (\`hungBefore\`, \`timedOut\`).\n`
md += '\n## Per brief\n\n| model | recipes | brief | done | first doctor clean | errors at first check | tool calls | tokens in | cost | time | doctor errors at the end |\n|---|---|---|---|---|---:|---:|---:|---:|---:|---|\n'
for (const r of runs) for (const x of r.rows ?? []) md += `| ${r.model} | ${r.variant} | ${x.id} | ${x.done ? 'yes' : x.doctorOk ? 'no (routes)' : 'no'} | ${x.firstDoctorOk === null ? 'never ran' : x.firstDoctorOk ? 'yes' : 'no'} | ${x.firstCheckErrs ?? '–'} | ${x.toolCalls} | ${k(x.inputTokens)} | $${(x.cost ?? 0).toFixed(3)} | ${x.s} s | ${x.doctorErrors ? x.doctorErrors.join(' ') : ''} |\n`
writeFileSync(join(OUT, 'summary.md'), md)
console.log(md)
