// #21 final — joins the generation runs (bench/autonomy/results/final21-<model>.json) and the measurements
// (bench/sites/final/results/<model>-<site>.json) into results/summary.json and results/summary.md.
// Usage: node bench/sites/final/summarize.mjs [--tag final21b]   (generation reports: bench/autonomy/results/<tag>-<model>.json)
import { readFileSync, writeFileSync, existsSync } from 'node:fs'
import { join } from 'node:path'
import { homedir } from 'node:os'

const ROOT = new URL('../../..', import.meta.url).pathname
const HERE = join(ROOT, 'bench/sites/final')
const read = (p) => existsSync(p) ? JSON.parse(readFileSync(p, 'utf8')) : null
const TAG = (() => { const i = process.argv.indexOf('--tag'); return i >= 0 ? process.argv[i + 1] : 'final21' })()
const MODELS = { sonnet: 'claude-sonnet-5-5', haiku: 'claude-haiku-5-5' }
const kb = (n) => n == null ? '—' : `${(n / 1024).toFixed(1)} KB`
const ms = (s) => s ? `${s.median} / ${s.p95}` : '—'

// Shipped bytes count each distinct file once; a file Chrome requested twice (a preload it discarded) is reported in
// duplicateRequests and still counts in `transferred`, which is what went over the wire.
const dedupe = (x) => {
  const seen = new Map(); for (const r of x.resources ?? []) if (!r.missing && !seen.has(r.path)) seen.set(r.path, r)
  const rows = [...seen.values()]
  const by = (k) => { const y = rows.filter((r) => r.kind === k); return { files: y.length, raw: y.reduce((a, r) => a + r.raw, 0), gz: y.reduce((a, r) => a + r.gz, 0), br: y.reduce((a, r) => a + r.br, 0) } }
  const t = (k) => rows.filter((r) => r.kind !== 'other').reduce((a, r) => a + r[k], 0)
  return { html: by('html'), js: by('js'), css: by('css'), other: by('other'), totalHtmlJsCss: { raw: t('raw'), gz: t('gz'), br: t('br') },
    requests: x.requests, duplicateRequests: (x.resources ?? []).length - rows.length - (x.resources ?? []).filter((r) => r.missing).length,
    duplicated: [...new Set((x.resources ?? []).filter((r, i, a) => a.findIndex((q) => q.path === r.path) !== i).map((r) => r.path))], transferred: x.transferred }
}

const sites = []
for (const [label, model] of Object.entries(MODELS)) {
  const gen = read(join(ROOT, `bench/autonomy/results/${TAG}-${label}.json`))
  for (const site of ['landing', 'panel']) {
    const g = gen?.rows.find((r) => r.id === site) ?? null
    const m = read(join(HERE, 'results', `${label}-${site}.json`))
    const main = m?.routes?.includes('/customers') ? '/customers' : '/'
    const b = m?.bytes?.[main]
    sites.push({
      site, label, model, primary: label === 'sonnet',
      generation: g && { done: g.done, doctorExit: g.doctor?.exit, routesVsBrief: `${g.views}/${g.pages}`, wallS: +(g.ms / 1000).toFixed(1), toolCalls: g.toolCalls,
        calls: g.calls, turns: g.turns, inputTokens: g.inputTokens, outputTokens: g.outputTokens, costUsd: g.cost, stuckLoops: g.stuckLoops,
        humanInterventions: g.humanInterventions, timedOut: g.timedOut, brainSha: gen.brainSha, specSha: gen.specSha },
      quality: m && { doctorExit: m.doctor?.exit, tests: `${m.test?.passed}/${m.test?.tests}`, testExit: m.test?.exit,
        buildAsGeneratedExit: m.buildAsGenerated?.exit, buildAsGeneratedErrors: m.buildAsGenerated?.errors?.length ?? 0, patch: m.patch ?? null,
        distOk: m.dist?.ok, dist: m.dist && { sourceExtLeaked: m.dist.sourceExtLeaked, sourceMaps: m.dist.sourceMaps, missingGz: m.dist.missingGz, brSiblings: m.dist.brSiblings, snippets: m.dist.snippets },
        contentTypeOk: m.contentType?.ok, contentTypeChecked: m.contentType?.checked, encodings: m.contentType?.encodings,
        lighthouse: m.lighthouse, brainSeo: m.brainSeo?.pages,
        smoke: m.smoke && { passed: m.smoke.passed, total: m.smoke.total, failed: m.smoke.checks.filter((c) => !c.pass).map((c) => ({ name: c.name, evidence: c.evidence })), consoleErrors: m.smoke.consoleErrors?.length ?? null } },
      performance: m && { buildColdMs: m.buildCold, buildWarmMs: m.buildWarm, mainRoute: main,
        bytes: m.bytes && Object.fromEntries(Object.entries(m.bytes).map(([r, x]) => [r, dedupe(x)])),
        firstRouteJs: b ? dedupe(b).js : null,
        hmr: m.hmr && { component: m.hmr.component, style: m.hmr.style, script: m.hmr.script, statePreserved: m.hmr.statePreserved, reloaded: m.hmr.reloaded, error: m.hmr.error } },
      binary: m && { brainSha: m.brainSha, specSha: m.specSha },
    })
  }
}
const summary = { at: new Date().toISOString(), sites }
writeFileSync(join(HERE, 'results/summary.json'), JSON.stringify(summary, null, 1).split(homedir()).join('~'))

// Markdown: one table per site, Sonnet (primary) and Haiku side by side.
const row = (name, f) => `| ${name} | ${['sonnet', 'haiku'].map((l) => { const s = sites.find((x) => x.label === l && x.site === cur); try { return f(s) ?? '—' } catch { return '—' } }).join(' | ')} |`
let cur
const md = [`# #21 final — results summary`, '', `Generated ${summary.at.slice(0, 10)} by \`summarize.mjs\`. Primary pair: Sonnet; Haiku is the second data point.`, '']
for (cur of ['landing', 'panel']) {
  md.push(`## ${cur === 'landing' ? 'Landing — "Nubo"' : 'Panel — "Nubo Admin"'}`, '', '| Metric | Sonnet 5.5 (primary) | Haiku 5.5 |', '|---|---|---|')
  md.push('| **Generation** | | |',
    row('done (doctor 0 and routes ≥ brief)', (s) => `${s.generation.done ? 'yes' : 'no'} (routes ${s.generation.routesVsBrief})`),
    row('wall time', (s) => `${s.generation.wallS} s`),
    row('tool calls / turns', (s) => `${s.generation.toolCalls} / ${s.generation.turns}`),
    row('tokens in / out', (s) => `${s.generation.inputTokens.toLocaleString('en')} / ${s.generation.outputTokens.toLocaleString('en')}`),
    row('cost', (s) => `$${s.generation.costUsd.toFixed(4)}`),
    row('stuck loops · human interventions', (s) => `${s.generation.stuckLoops} · ${s.generation.humanInterventions}`),
    '| **Quality** | | |',
    row('`brain doctor` exit', (s) => s.quality.doctorExit),
    row('`brain test` passed', (s) => `${s.quality.tests} (exit ${s.quality.testExit})`),
    row('`brain build` on the app as generated', (s) => s.quality.buildAsGeneratedExit === 0 ? 'exit 0' : `**exit ${s.quality.buildAsGeneratedExit}** (${s.quality.buildAsGeneratedErrors} errors; measured after: ${s.quality.patch?.rewrite?.join(', ')})`),
    row('dist rules (INTERNALS §10)', (s) => `${s.quality.distOk ? 'ok' : 'fail'}: ${s.quality.dist.sourceExtLeaked.length} source ext · ${s.quality.dist.sourceMaps.length} maps · ${s.quality.dist.missingGz.length} text files without .gz (${s.quality.dist.missingGz.map((x) => x.file).join(', ') || '—'}) · .br ${s.quality.dist.brSiblings}`),
    row('Content-Type via `brain start`', (s) => `${s.quality.contentTypeOk ? 'ok' : 'fail'} (${s.quality.contentTypeChecked} URLs, ${Object.entries(s.quality.encodings).map(([k, v]) => `${k} ${v}`).join(', ')})`),
    row('Lighthouse a11y / SEO per page', (s) => Object.entries(s.quality.lighthouse).map(([r, x]) => `\`${r}\` ${x.a11y}/${x.seo}${x.failing.length ? ` (${x.failing.join(', ')})` : ''}`).join('<br>')),
    row('`brain seo audit` score', (s) => s.quality.brainSeo.map((p) => `\`${p.route}\` ${p.score}`).join(', ')),
    row('functional smoke', (s) => `${s.quality.smoke.passed}/${s.quality.smoke.total}; console errors ${s.quality.smoke.consoleErrors}`),
    row('smoke failures', (s) => s.quality.smoke.failed.map((f) => f.name).join('<br>') || '—'),
    '| **Performance** | | |',
    row('build cold, median / p95 (10 runs)', (s) => `${ms(s.performance.buildColdMs)} ms`),
    row('build warm, median / p95 (10 runs)', (s) => `${ms(s.performance.buildWarmMs)} ms`),
    row('main route HTML+JS+CSS raw / gz / br', (s) => { const t = s.performance.bytes[s.performance.mainRoute].totalHtmlJsCss; return `${kb(t.raw)} / ${kb(t.gz)} / ${kb(t.br)}` }),
    row('…HTML · JS · CSS (gz)', (s) => { const x = s.performance.bytes[s.performance.mainRoute]; return `${kb(x.html.gz)} · ${kb(x.js.gz)} (${x.js.files} files) · ${kb(x.css.gz)}` }),
    row('first-route JS raw / gz / br', (s) => `${kb(s.performance.firstRouteJs.raw)} / ${kb(s.performance.firstRouteJs.gz)} / ${kb(s.performance.firstRouteJs.br)}`),
    row('distinct files loaded · CSS fetched twice (SRI preload discarded)', (s) => { const x = s.performance.bytes[s.performance.mainRoute]; return `${x.html.files + x.js.files + x.css.files + x.other.files} · ${x.duplicated.filter((p) => p.endsWith('.css')).length}` }),
    row('HMR component', (s) => `\`${s.performance.hmr.component}\``),
    row('HMR style, tool median / p95 (20)', (s) => `${ms(s.performance.hmr.style.tool)} ms (${s.performance.hmr.style.applied.join(',')}, errors ${s.performance.hmr.style.errors})`),
    row('HMR script, tool median / p95 (20)', (s) => `${ms(s.performance.hmr.script.tool)} ms (${s.performance.hmr.script.applied.join(',')}, errors ${s.performance.hmr.script.errors})`),
    row('HMR incl. OS file event, median', (s) => `style ${s.performance.hmr.style.total.median} · script ${s.performance.hmr.script.total.median} ms`),
    row('HMR state preserved · page reloaded', (s) => `${s.performance.hmr.statePreserved} · ${s.performance.hmr.reloaded}`),
    '')
  const other = Object.keys(sites.find((s) => s.label === 'sonnet' && s.site === cur)?.performance?.bytes ?? {}).filter((r) => r !== '/' && r !== '/customers')
  if (other.length) md.push(`Other routes of the Sonnet ${cur} (gz HTML+JS+CSS): ${other.map((r) => `\`${r}\` ${kb(sites.find((s) => s.label === 'sonnet' && s.site === cur).performance.bytes[r].totalHtmlJsCss.gz)}`).join(', ')}.`, '')
}
const s0 = sites.find((s) => s.binary)
md.push(`Binary: \`brain\` release sha256 \`${s0?.binary?.brainSha}\` (measurement) · generation binary (frozen by the harness, run tag \`${TAG}\`) \`${sites[0].generation?.brainSha}\` · spec sha256 \`${s0?.binary?.specSha}\`.`)
writeFileSync(join(HERE, 'results/summary.md'), md.join('\n').split(homedir()).join('~') + '\n')
console.log(md.join('\n'))
