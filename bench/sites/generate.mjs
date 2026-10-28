// #21 — An LLM builds a site from docs only (llms.txt + `brain spec`), file by file, then iterates on `brain check`
// until 0 diagnostics or --max-iter. Same harness for every engine, so iterations and times are comparable.
// The harness plays the MCP role (brain_new → write files → brain_check); see PROTOCOL.md §2.
// Usage: node bench/sites/generate.mjs --engine claude|qwen --site landing|panel [--max-iter 6] [--run <name>]
import { execFileSync } from 'node:child_process'
import { readFileSync, writeFileSync, mkdirSync, rmSync, existsSync } from 'node:fs'
import { join, dirname } from 'node:path'
import { engines, modelOf, extractCode } from '../lib/llm.mjs'
import { check as qaCheck } from '../phase0/check.mjs'
import { lint as a11yLint } from '../a11y-seo/a11y.mjs'

const ROOT = new URL('../..', import.meta.url).pathname
const BRAIN = process.env.BRAIN ?? (existsSync(join(ROOT, 'target/release/brain')) ? join(ROOT, 'target/release/brain') : join(ROOT, 'target/debug/brain'))
const argv = process.argv.slice(2)
const arg = (k, d) => { const i = argv.indexOf(`--${k}`); return i >= 0 ? argv[i + 1] : d }
const ENGINE = arg('engine', 'claude'), SITE = arg('site', 'landing'), MAX = Number(arg('max-iter', 6))
const RUN = arg('run', `${ENGINE}-${SITE}-${new Date().toISOString().replace(/[:.]/g, '-')}`)
const generate = engines[ENGINE]
if (!generate) throw Error(`unknown engine: ${ENGINE}`)
const BRIEF = JSON.parse(readFileSync(new URL(`./briefs/${SITE}.json`, import.meta.url), 'utf8'))

const RUNS = join(ROOT, 'bench/sites/runs'), APP = join(RUNS, RUN)
mkdirSync(RUNS, { recursive: true })
rmSync(APP, { recursive: true, force: true })
const t0 = Date.now()
execFileSync(BRAIN, ['new', RUN], { cwd: RUNS, stdio: 'ignore' }) // brain_new
// The scaffold's sample files are not part of the site: the LLM writes only the brief's file plan.
for (const f of ['views/home.brain', 'comp/counter.brain', 'ui/badge.brain']) rmSync(join(APP, f), { force: true })

// --docs spec (default): llms.txt + `brain spec` · --docs specmd: llms.txt + SPEC.md read from disk.
const DOCS_MODE = arg('docs', 'spec')
const DOCS = `${readFileSync(join(ROOT, 'llms.txt'), 'utf8')}\n\n${DOCS_MODE === 'specmd' ? readFileSync(join(ROOT, 'SPEC.md'), 'utf8') : execFileSync(BRAIN, ['spec']).toString()}`
const TOKENS = readFileSync(join(APP, existsSync(join(APP, 'tokens.cssbrain')) ? 'tokens.cssbrain' : 'tokens.brain.css'), 'utf8')
const SYS = 'You are a developer who uses BRAINLLM. You know only what the documentation you receive says. ' +
  'Answer only with the content of the requested file inside a single ```html block, with no explanation.'
const plan = BRIEF.files.map((f) => `- ${f.path}: ${f.role}`).join('\n')

const brainCheck = () => {
  let out
  try { out = execFileSync(BRAIN, ['check', '--json', '.'], { cwd: APP }).toString() } catch (e) { out = e.stdout.toString() }
  return JSON.parse(out.trim() || '[]')
}
const qa = (f) => {
  const src = readFileSync(join(APP, f), 'utf8')
  return [...qaCheck(src, f).errors, ...a11yLint(src, f).diags]
}

const log = { docs: DOCS_MODE, docsBytes: Buffer.byteLength(DOCS), engine: ENGINE, model: modelOf(ENGINE), site: SITE, brain: BRAIN, startedAt: new Date(t0).toISOString(), iterations: [] }
let modelMs = 0, checkMs = 0, calls = 0, inputTokens = 0
async function write(path, prompt) {
  const r = await generate(SYS, prompt)
  modelMs += r.ms; calls++; inputTokens += r.input ?? 0
  mkdirSync(dirname(join(APP, path)), { recursive: true })
  writeFileSync(join(APP, path), extractCode(r.text))
}

// Provided files (plain JS fixtures) are copied, not generated.
for (const f of BRIEF.files.filter((x) => x.provided)) {
  mkdirSync(dirname(join(APP, f.path)), { recursive: true })
  writeFileSync(join(APP, f.path), readFileSync(join(ROOT, 'examples', f.path), 'utf8'))
}
// Iteration 1: write every file of the plan, in order (later files see the earlier ones).
for (const f of BRIEF.files.filter((x) => !x.provided)) {
  const done = BRIEF.files.filter((x) => x !== f && existsSync(join(APP, x.path)))
    .map((x) => `--- ${x.path}\n${readFileSync(join(APP, x.path), 'utf8')}`).join('\n')
  await write(f.path, `DOCUMENTATION:\n${DOCS}\n\nAVAILABLE TOKENS (tokens.brain.css):\n${TOKENS}\n\nSITE: ${BRIEF.brief}\n\n` +
    `FILE PLAN:\n${plan}\n\n${done ? `FILES ALREADY WRITTEN:\n${done}\n\n` : ''}Now write ONLY the file ${f.path}.`)
}

for (let it = 1; it <= MAX; it++) {
  const c0 = Date.now(); const diags = brainCheck(); checkMs += Date.now() - c0
  const qaDiags = Object.fromEntries(BRIEF.files.filter((f) => f.path.endsWith('.brain')).map((f) => [f.path, existsSync(join(APP, f.path)) ? qa(f.path) : [{ code: 'MISSING' }]]))
  const qaCount = Object.values(qaDiags).reduce((a, d) => a + d.length, 0)
  log.iterations.push({ it, brainCheck: diags.length, codes: diags.map((d) => d.code), diags: diags.map(({ code, file, line, msg }) => ({ code, file, line, msg })), qaCheck: qaCount, elapsedMs: Date.now() - t0 })
  console.log(`iter ${it}: brain check ${diags.length} · qa ${qaCount}`)
  if (diags.length === 0) break
  if (it === MAX) break
  const byFile = Map.groupBy(diags, (d) => d.file)
  for (const [file, ds] of byFile) {
    if (!existsSync(join(APP, file))) continue
    await write(file, `DOCUMENTATION:\n${DOCS}\n\nThe file ${file} has these \`brain check --json\` errors:\n${JSON.stringify(ds, null, 2)}\n\n` +
      `CURRENT FILE:\n\`\`\`html\n${readFileSync(join(APP, file), 'utf8')}\`\`\`\n\nFix it by applying each \`fix\` and return the complete file.`)
  }
}
const last = log.iterations.at(-1)
Object.assign(log, { ok: last.brainCheck === 0, iterationsToClean: last.brainCheck === 0 ? last.it : null,
  totalMs: Date.now() - t0, modelMs, checkMs, calls, inputTokens, app: APP })
writeFileSync(join(APP, 'qa-run.json'), JSON.stringify(log, null, 2))
writeFileSync(join(RUNS, `${RUN}.json`), JSON.stringify(log, null, 2))
console.log(JSON.stringify({ run: RUN, ok: log.ok, iterationsToClean: log.iterationsToClean, totalMs: log.totalMs, calls }))
