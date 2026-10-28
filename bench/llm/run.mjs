// LLM test (#11): 20 views generated with only `brain spec` as context → % that pass `brain check` on the first try.
// Usage: node bench/llm/run.mjs [--spec-file spec.md] [--briefs briefs-100.json] [--model claude-haiku-5-5|qwen] [--mode spec|mcp] [--fix 3] [--conc 5] [--only 03-login]
// --mode mcp: besides the spec, the model receives the app map (`brain map --json` if it exists; otherwise a QA
//   substitute with files, components, props/emits and exports), as it would see it through MCP. --model qwen calls ollama directly.
// Output: results/<model>/<id>/try<N>.brain + results/<model>.json + table on stdout.
import { spawn, execFileSync } from 'node:child_process'
import { readFileSync, writeFileSync, mkdirSync, rmSync, existsSync, readdirSync, copyFileSync } from 'node:fs'
import { join, dirname } from 'node:path'
import { fileURLToPath } from 'node:url'
import { tmpdir } from 'node:os'

const here = dirname(fileURLToPath(import.meta.url))
const root = join(here, '../..')
const arg = (k, d) => { const i = process.argv.indexOf(`--${k}`); return i > 0 ? process.argv[i + 1] : d }
const MODEL = arg('model', 'claude-haiku-5-5')
const FIX = Number(arg('fix', 3))
const CONC = Number(arg('conc', 5))
const ONLY = arg('only', null)
const MODE = arg('mode', 'spec')
const STAGE = arg('stage', '')
const TAG = [MODEL, MODE === 'spec' ? '' : MODE, STAGE].filter(Boolean).join('-')
const BRAIN_LIVE = arg('brain', join(root, 'target/debug/brain'))
const WORK = process.env.QA_LLM_WORK ?? join(tmpdir(), 'brainllm-llm')
// Frozen: the binary is copied at start and the whole run (spec, scaffold, check) uses that copy, so a
// rebuild by another role mid-run does not mix two checker versions.
const BIN_SHA = execFileSync('shasum', ['-a', '256', BRAIN_LIVE], { encoding: 'utf8' }).slice(0, 12)
const BRAIN = join(WORK, 'bin', `brain-${BIN_SHA}`)
mkdirSync(join(WORK, 'bin'), { recursive: true }); if (!existsSync(BRAIN)) copyFileSync(BRAIN_LIVE, BRAIN)
// --spec-file <path>: use that text instead of the served spec (ablation: measure what each section contributes).
const SPEC_FILE = arg('spec-file', null)
const SPEC = SPEC_FILE ? readFileSync(SPEC_FILE, 'utf8') : execFileSync(BRAIN, ['spec'], { encoding: 'utf8' })
if (!SPEC.includes('## Reference component') || !SPEC.includes('## SEO')) { console.error(`ABORT: the served spec is truncated (${SPEC.length} B, missing "## SEO" or "## Reference component")`); process.exit(2) }
const SPEC_SHA = execFileSync('shasum', ['-a', '256'], { input: SPEC, encoding: 'utf8' }).slice(0, 12)
// Primitives that SPEC describes and the checker does not implement yet: a B0111/B0204 on them is a toolchain gap, not an LLM error.
const AHEAD = /`(form|resource|api|provide|use|key|store|bus|sanitize|mount|click|type|expect|test|text|attr|hasClass|count)` is not declared|bind:value` needs the name of a `sig`|bind:value` only goes on HTML elements, not on components|`bind:api` does not exist/

const SYS = `${SPEC}\n\nYou write BRAINLLM files. Answer with ONLY the complete content of the requested file, no prose, no code fences.`
const BRIEFS = arg('briefs', 'briefs.json')
const briefs = JSON.parse(readFileSync(join(here, BRIEFS), 'utf8')).filter((b) => !ONLY || b.id === ONLY)

// Local qwen: direct to the ollama API (local-assistant.sh caps requests at 120 s and a 4k context, too small for
// spec + previous file + diagnostics). Timeout 15 min, 16k context, temperature 0.2.
const QWEN_MODEL = process.env.QWEN_MODEL || 'qwen2.5-coder:7b'
async function qwen(prompt) {
  const ac = new AbortController(); const t = setTimeout(() => ac.abort(), 15 * 60 * 1000)
  try {
    // stream:true so headers arrive at once (undici aborts after 300 s waiting for headers on long generations)
    const r = await fetch('http://localhost:11434/api/chat', { method: 'POST', signal: ac.signal, body: JSON.stringify({
      model: QWEN_MODEL, stream: true, options: { num_ctx: 16384, temperature: 0.2 },
      messages: [{ role: 'system', content: SYS }, { role: 'user', content: prompt }] }) })
    let text = '', buf = '', j = {}
    for await (const chunk of r.body) {
      buf += Buffer.from(chunk).toString('utf8'); let nl
      while ((nl = buf.indexOf('\n')) >= 0) { const line = buf.slice(0, nl).trim(); buf = buf.slice(nl + 1); if (!line) continue
        j = JSON.parse(line); if (j.error) throw new Error(`qwen: ${j.error}`); text += j.message?.content || '' }
    }
    if (!text.trim()) throw new Error(`qwen: empty (${r.status})`)
    return { text, input: j.prompt_eval_count || 0, output: j.eval_count || 0, cost: 0 }
  } finally { clearTimeout(t) }
}
const generate = (prompt) => (MODEL === 'qwen' ? qwen(prompt) : claude(prompt))

// App map for mcp mode. Prefers the real one; the substitute lists what SPEC promises ("routes, components,
// props, channels, tests") by reading the scaffold files.
function appMap(app) {
  try { return execFileSync(BRAIN, ['map', '--json', app], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'] }) } catch {}
  const out = { source: 'qa-substitute (brain map does not exist yet)', files: [], tokens: [] }
  const walk = (d) => { for (const e of readdirSync(join(app, d), { withFileTypes: true })) { const r = d ? `${d}/${e.name}` : e.name; if (e.isDirectory()) walk(r); else out.files.push(r) } }
  walk('')
  out.components = out.files.filter((f) => f.endsWith('.brain')).map((f) => {
    const src = readFileSync(join(app, f), 'utf8'); const m = src.match(/<meta props="([^"]*)"/); const e = src.match(/emits="([^"]*)"/)
    return { file: f, name: f.split('/').pop().replace('.brain', '').replace(/(^|-)(\w)/g, (_, __, c) => c.toUpperCase()), props: m ? m[1] : '', emits: e ? e[1] : '' }
  })
  const tok = existsSync(join(app, 'tokens.cssbrain')) ? readFileSync(join(app, 'tokens.cssbrain'), 'utf8') : ''
  out.tokens = [...tok.matchAll(/(--[\w-]+)\s*:/g)].map((m) => m[1])
  return JSON.stringify(out)
}

function claude(prompt) {
  return new Promise((resolve, reject) => {
    const p = spawn('claude', ['-p', '--model', MODEL, '--output-format', 'json', '--system-prompt', SYS,
      '--tools', '', '--setting-sources', '', '--strict-mcp-config', '--no-session-persistence', prompt],
      { cwd: WORK, stdio: ['ignore', 'pipe', 'pipe'] })
    let out = ''; let err = ''
    p.stdout.on('data', (d) => (out += d)); p.stderr.on('data', (d) => (err += d))
    p.on('close', () => {
      try {
        const j = JSON.parse(out)
        if (j.is_error) return reject(new Error(j.result || 'claude error'))
        const u = j.usage || {}
        resolve({ text: j.result, input: (u.input_tokens || 0) + (u.cache_creation_input_tokens || 0) + (u.cache_read_input_tokens || 0), output: u.output_tokens || 0, cost: j.total_cost_usd || 0 })
      } catch (e) { reject(new Error(err || out || String(e))) }
    })
  })
}
const strip = (t) => t.replace(/^\s*```[a-z]*\n/, '').replace(/\n```\s*$/, '').trim() + '\n'
const target = (id) => (id === '05-product-detail' ? 'views/products/[id].brain' : `views/${id.replace(/^\d+-/, '')}.brain`)

function check(app, rel) {
  let raw
  try { raw = execFileSync(BRAIN, ['check', '--json', app], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] }) }
  catch (e) { raw = e.stdout || '' }
  let all; try { all = JSON.parse(raw || '[]') } catch { all = [{ code: 'CRASH', msg: raw.slice(0, 300) }] }
  const base = rel.split('/').pop()
  // Only level "error" counts as a failure (INTERNALS §21): "info" are normalizations the compiler applies itself and
  // "warning" does not block the build. Diags without level (older binaries) count as errors.
  return all.filter((d) => (!d.file || d.file.endsWith(base) || d.file.endsWith(rel)) && (d.level ?? 'error') === 'error')
}
const kind = (d) => (AHEAD.test(d.msg) ? 'ahead' : /^B0[45]/.test(d.code) ? 'a11y' : 'lang')

async function one(b) {
  const app = join(WORK, 'apps', `${TAG}-${b.id}`)
  rmSync(app, { recursive: true, force: true }); mkdirSync(dirname(app), { recursive: true })
  execFileSync(BRAIN, ['new', app], { stdio: 'ignore' })
  const rel = target(b.id); const file = join(app, rel); mkdirSync(dirname(file), { recursive: true })
  const outDir = join(here, 'results', TAG, b.id); mkdirSync(outDir, { recursive: true })
  const ctx = MODE === 'mcp' ? `Project map (brain map --json):\n${appMap(app)}\n\n` : ''
  let prompt = `${ctx}Write the file ${rel}.\n\n${b.brief}`
  const tries = []
  for (let t = 0; t <= FIX; t++) {
    const t0 = Date.now(); const r = await generate(prompt); const ms = Date.now() - t0
    const src = strip(r.text); writeFileSync(file, src); writeFileSync(join(outDir, `try${t}.brain`), src)
    const diags = check(app, rel)
    tries.push({ t, ms, input: r.input, output: r.output, cost: r.cost, diags: diags.map((d) => ({ ...d, kind: kind(d) })) })
    if (diags.length === 0) break
    prompt = `${ctx}Write the file ${rel}.\n\n${b.brief}\n\nYour previous version:\n${src}\n\`brain check --json\` returned:\n${JSON.stringify(diags)}\nApply each fix and return the complete corrected file.`
  }
  const first = tries[0].diags
  return { id: b.id, file: rel, firstPass: first.length === 0, firstPassLang: first.every((d) => d.kind !== 'lang'),
    firstOnlyAhead: first.length > 0 && first.every((d) => d.kind === 'ahead'),
    iterations: tries.length, finalPass: tries.at(-1).diags.length === 0, tries }
}

async function pool(items, n, fn) {
  const out = []; let i = 0
  await Promise.all(Array.from({ length: n }, async () => { while (i < items.length) { const k = i++; try { out[k] = await fn(items[k]) } catch (e) { out[k] = { id: items[k].id, infraFail: String(e).slice(0, 300) } ; console.error(items[k].id, e.message) } } }))
  return out
}

mkdirSync(WORK, { recursive: true })
const scaffoldSha = () => { const d = join(WORK, 'sha-probe'); rmSync(d, { recursive: true, force: true }); execFileSync(BRAIN, ['new', d], { stdio: 'ignore' }); return execFileSync('shasum', ['-a', '256'], { input: readFileSync(join(d, 'tokens.cssbrain')), encoding: 'utf8' }).slice(0, 12) }
const TOK0 = scaffoldSha()
const rows = await pool(briefs, CONC, one)
// Guard: if the spec or the scaffold change mid-run (other roles edit live), the run is not comparable.
const SPEC_END = execFileSync('shasum', ['-a', '256'], { input: SPEC_FILE ? readFileSync(SPEC_FILE, 'utf8') : execFileSync(BRAIN, ['spec'], { encoding: 'utf8' }), encoding: 'utf8' }).slice(0, 12)
const TOK1 = scaffoldSha()
if (SPEC_END !== SPEC_SHA || TOK1 !== TOK0) console.error(`WARNING: the spec (${SPEC_SHA}→${SPEC_END}) or the scaffold (${TOK0}→${TOK1}) changed during the run: result NOT comparable`)
const ok = rows.filter((r) => !r.infraFail)
const pct = (k) => `${ok.filter((r) => r[k]).length}/${ok.length}`
const codes = {}; for (const r of ok) for (const d of r.tries[0].diags) codes[`${d.code}:${d.kind}`] = (codes[`${d.code}:${d.kind}`] || 0) + 1
const sum = { model: MODEL, mode: MODE, briefs: BRIEFS, specSha: SPEC_SHA, binSha: BIN_SHA, specShaEnd: SPEC_END, tokensSha: TOK0, tokensShaEnd: TOK1, stable: SPEC_END === SPEC_SHA && TOK1 === TOK0, specBytes: SPEC.length, fix: FIX, at: new Date().toISOString(), n: ok.length, infraFail: rows.length - ok.length,
  firstPass: pct('firstPass'), firstPassLang: pct('firstPassLang'), firstOnlyAhead: pct('firstOnlyAhead'), finalPass: pct('finalPass'),
  meanIterations: +(ok.reduce((a, r) => a + r.iterations, 0) / ok.length).toFixed(2),
  costUSD: +ok.reduce((a, r) => a + r.tries.reduce((x, t) => x + t.cost, 0), 0).toFixed(4), firstTryCodes: codes }
mkdirSync(join(here, 'results'), { recursive: true })
writeFileSync(join(here, 'results', `${TAG}${ONLY ? '-' + ONLY : ''}.json`), JSON.stringify({ summary: sum, rows }, null, 1))
console.log(JSON.stringify(sum, null, 1))
console.log('| view | 1st try | iterations | final | 1st-try codes |\n|---|---|---:|---|---|')
for (const r of rows) console.log(r.infraFail ? `| ${r.id} | infra | | | ${r.infraFail.slice(0, 80)} |` : `| ${r.id} | ${r.firstPass ? '✅' : '❌'} | ${r.iterations} | ${r.finalPass ? '✅' : '❌'} | ${r.tries[0].diags.map((d) => d.code + ':' + d.kind).join(' ')} |`)
