// Phase 0 — token cost and generation error rate of naming styles (semantic / abbreviated / cryptic).
// Method (decided by the LEAD 2026-10-13, no API key): `claude -p --output-format json` and read `usage`.
// Input tokens = input_tokens + cache_creation_input_tokens + cache_read_input_tokens.
// Token cost of a variant = total input(base + component) − total input(base), same prompt for all variants.
// Usage: node run.mjs [--gens 20] [--reps 3] [--conc 5] [--only tokens|gen] [--task 1|2]
import { spawn } from 'node:child_process'
import { readFileSync, writeFileSync, mkdirSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { tmpdir } from 'node:os'
import { fileURLToPath } from 'node:url'
import { check } from './check.mjs'

const HERE = dirname(fileURLToPath(import.meta.url))
const MODEL = 'claude-haiku-5-5'
const args = Object.fromEntries(process.argv.slice(2).join(' ').split('--').filter(Boolean).map((s) => { const [k, ...v] = s.trim().split(/\s+/); return [k, v.length > 1 ? v : v[0]] }))
const GENS = Number(args.gens ?? 20), REPS = Number(args.reps ?? 3), CONC = Number(args.conc ?? 5)
const VARIANTS = (args.variants ?? 'semantic,abbreviated,cryptic,apishort').split(',')
const ENGINE = args.engine ?? 'claude' // claude (Haiku 5.5 via claude -p) | qwen (local-assistant.sh code)
const read = (p) => readFileSync(join(HERE, p), 'utf8')
// apishort = framework API renamed to s/d/f (user names stay semantic); its spec excerpt uses the same names.
const specFor = (v) => read(v === 'apishort' ? 'spec-excerpt-apishort.md' : 'spec-excerpt.md')
const apiFor = (v) => (v === 'apishort' ? { sig: 's', drv: 'd', fx: 'f' } : { sig: 'sig', drv: 'drv', fx: 'fx' })
const DICT = read('variants/cryptic.dict.md')
const OUT = join(HERE, 'results')
mkdirSync(OUT, { recursive: true })
// Isolated cwd so no project CLAUDE.md, hooks or memory leak into the prompt.
const CWD = process.env.PHASE0_CWD ?? join(tmpdir(), 'brainllm-phase0')
mkdirSync(CWD, { recursive: true })

function claude(system, prompt) {
  return new Promise((resolve, reject) => {
    const p = spawn('claude', ['-p', '--model', MODEL, '--output-format', 'json', '--system-prompt', system,
      '--tools', '', '--setting-sources', '', '--strict-mcp-config', '--no-session-persistence', prompt],
      { cwd: CWD, stdio: ['ignore', 'pipe', 'pipe'] })
    let out = '', err = ''
    p.stdout.on('data', (d) => (out += d))
    p.stderr.on('data', (d) => (err += d))
    p.on('close', (code) => {
      try {
        const j = JSON.parse(out)
        if (j.is_error) return reject(new Error(j.result || 'claude error'))
        const u = j.usage
        resolve({
          input: u.input_tokens + u.cache_creation_input_tokens + u.cache_read_input_tokens,
          output: u.output_tokens, thinking: u.output_tokens_details?.thinking_tokens ?? 0,
          cost: j.total_cost_usd, ms: j.duration_ms, text: j.result,
        })
      } catch (e) { reject(new Error(`exit ${code}: ${err || out.slice(0, 300)}`)) }
    })
  })
}

// Local qwen through a local assistant script (same prompt; the script fixes its own system prompt, ours goes first in the prompt).
// LOCAL_ASSISTANT overrides the command; by default it is looked up on PATH.
const LOCAL = process.env.LOCAL_ASSISTANT ?? 'local-assistant.sh'
function qwen(system, prompt) {
  return new Promise((resolve, reject) => {
    const t0 = Date.now()
    const p = spawn(LOCAL, ['code', `${system}\n\n${prompt}`], { stdio: ['ignore', 'pipe', 'pipe'] })
    let out = ''
    p.stdout.on('data', (d) => (out += d))
    p.on('close', () => {
      if (!out.trim() || out.startsWith('ERROR')) return reject(new Error(`qwen: ${out.slice(0, 200) || 'empty'}`))
      resolve({ input: null, output: null, thinking: 0, cost: 0, ms: Date.now() - t0, text: out })
    })
  })
}
const generate = (system, prompt) => (ENGINE === 'qwen' ? qwen(system, prompt) : claude(system, prompt))

async function pool(tasks, n) {
  const res = new Array(tasks.length); let i = 0
  await Promise.all(Array.from({ length: n }, async () => { while (i < tasks.length) { const k = i++; res[k] = await tasks[k]() } }))
  return res
}

const stats = (xs) => {
  const s = [...xs].sort((a, b) => a - b), mean = xs.reduce((a, b) => a + b, 0) / xs.length
  return { n: xs.length, mean: +mean.toFixed(1), median: s[Math.floor(s.length / 2)], min: s[0], max: s.at(-1) }
}

async function tokens() {
  const SYS = 'Reply only OK.'
  const BASE = 'Read the following .brain component and reply only OK.\n\n'
  const prompts = { base: BASE }
  for (const v of VARIANTS) prompts[v] = BASE + read(`variants/${v}.brain`)
  prompts['spec'] = BASE + read('spec-excerpt.md')
  prompts['spec-apishort'] = BASE + read('spec-excerpt-apishort.md')
  prompts['cryptic+dict'] = BASE + DICT + '\n' + read('variants/cryptic.brain')
  const rows = {}
  for (const [k, p] of Object.entries(prompts)) {
    rows[k] = (await pool(Array.from({ length: REPS }, () => () => claude(SYS, p)), REPS)).map((r) => r.input)
  }
  const base = Math.min(...rows.base)
  const table = Object.fromEntries(Object.entries(rows).map(([k, xs]) => [k, {
    inputs: xs, delta: k === 'base' ? 0 : Math.min(...xs) - base,
    chars: k === 'base' ? 0 : prompts[k].length - BASE.length, deterministic: new Set(xs).size === 1,
  }]))
  writeFileSync(join(OUT, 'tokens.json'), JSON.stringify({ model: MODEL, reps: REPS, at: new Date().toISOString(), table }, null, 2))
  console.table(Object.fromEntries(Object.entries(table).map(([k, v]) => [k, { delta: v.delta, chars: v.chars, det: v.deterministic }])))
}

const TASKS = {}
TASKS[1] = `Task: write the component comp/shopping-list.brain: a shopping list with a product field and a quantity field,
an add button, mark as bought, delete, a counter of pending products and an all/pending/bought filter.
Follow EXACTLY the same naming style as the reference example.`
// Task 2 departs from the reference: new state, derived money values, async submit, {#if}, {#await}-free error path.
TASKS[2] = `Task: write the component comp/cart.brain: a cart with a prop \`products\` (list of {id, name, price}),
a quantity per product with + and − buttons, remove line, subtotal, a coupon field (the coupon 'BRAIN10' gives a 10% discount,
any other shows an error message), final total, a "Pay" button disabled when the cart is empty, which emits
the 'checkout' event with the lines and the total. Show an empty state when there are no lines.
Follow EXACTLY the same naming style as the reference example (new names too).`
const TASK_ID = Number(args.task ?? 1)
const TASK = TASKS[TASK_ID]

function genPrompt(v) {
  const dict = v === 'cryptic' ? `\n${DICT}\n` : ''
  return `${specFor(v)}\n\nReference example:\n${dict}\n\`\`\`html\n${read(`variants/${v}.brain`)}\`\`\`\n\n${TASK}`
}

function extract(text) {
  const m = text.match(/```(?:html|brain|svelte)?\s*\n([\s\S]*?)```/)
  return (m ? m[1] : text).trim() + '\n'
}

function styleScore(v, src) {
  const script = (src.match(/<script>([\s\S]*?)<\/script>/) || ['', ''])[1]
  const names = [...script.matchAll(/\b(?:let|const)\s+([\w$]+)/g)].map((m) => m[1])
  if (!names.length) return 0
  if (v === 'cryptic') return names.filter((n) => /^[tfp]\d{2}$/.test(n)).length / names.length
  const avg = names.reduce((a, n) => a + n.length, 0) / names.length
  return +avg.toFixed(2) // semantic/abbreviated: mean identifier length
}

async function generations() {
  const SYS = 'You generate BRAINLLM components (.brain). Reply only with the file inside a single ```html block, with no explanation.'
  const summary = {}
  for (const v of VARIANTS) {
    const TAG = `${ENGINE === 'qwen' ? 'qwen-' : ''}${TASK_ID === 1 ? 'gen' : `gen-t${TASK_ID}`}`
    const dir = join(OUT, TAG, v); mkdirSync(dir, { recursive: true })
    const prompt = genPrompt(v)
    const runs = await pool(Array.from({ length: GENS }, (_, i) => async () => {
      try {
        const r = await generate(SYS, prompt)
        const src = extract(r.text)
        const file = join(dir, `${String(i + 1).padStart(2, '0')}.brain`)
        writeFileSync(file, src)
        const c = check(src, file, apiFor(v))
        return { i: i + 1, ok: c.ok, errors: c.errors, warnings: c.warnings, input: r.input, output: r.output - r.thinking, thinking: r.thinking,
          cost: r.cost, ms: r.ms, bytes: Buffer.byteLength(src), style: styleScore(v, src) }
      } catch (e) { return { i: i + 1, ok: false, errors: [{ code: 'RUN', msg: e.message }] } }
    }), CONC)
    writeFileSync(join(OUT, `${TAG}-${v}.jsonl`), runs.map((r) => JSON.stringify(r)).join('\n') + '\n')
    const okRuns = runs.filter((r) => r.ms !== undefined)
    const infra = runs.filter((r) => r.ms === undefined).length
    const codes = {}
    for (const r of runs) for (const e of r.errors) codes[e.code] = (codes[e.code] || 0) + 1
    summary[v] = {
      n: runs.length, infraFail: infra, deadState: runs.filter((r) => r.warnings?.length).length, pass: runs.filter((r) => r.ok).length, errorRate: +(1 - runs.filter((r) => r.ok).length / Math.max(1, runs.length - infra)).toFixed(3), // infra failures excluded
      errorCodes: codes, promptInput: okRuns[0]?.input, outputTokens: ENGINE === 'qwen' ? null : stats(okRuns.map((r) => r.output)),
      outputBytes: stats(okRuns.map((r) => r.bytes)), ms: stats(okRuns.map((r) => r.ms)),
      style: stats(okRuns.map((r) => r.style)), costUSD: +okRuns.reduce((a, r) => a + r.cost, 0).toFixed(4),
    }
    console.log(v, JSON.stringify(summary[v]))
  }
  writeFileSync(join(OUT, `generations-${ENGINE}-t${TASK_ID}.json`), JSON.stringify({ model: ENGINE === 'qwen' ? 'qwen2.5-coder:7b' : MODEL, engine: ENGINE, task: TASK_ID, gens: GENS, at: new Date().toISOString(), summary }, null, 2))
}

if (args.only !== 'gen') await tokens()
if (args.only !== 'tokens') await generations()
