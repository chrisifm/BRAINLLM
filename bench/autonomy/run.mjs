// Autonomy benchmark (INTERNALS §25): a complete multi-view app from a one-paragraph brief, no human turns.
// The agent gets only the brain MCP server (mode `mcp`) or the MCP server plus the host's Read/Write/Edit
// (mode `mcp-host`, baseline until brain_write lands, #89). No Bash, no web, no user settings or hooks.
// Done = an independent `brain doctor --json` exits 0 after the agent stops (the agent's own claim does not count)
// AND the app still has at least the number of routes the brief asks for (`pages`): pruning the brief is not done.
//
// Usage: node bench/autonomy/run.mjs [--model claude-haiku-5-5|claude-sonnet-5-5] [--mode mcp|mcp-host]
//          [--only habits,events] [--max-turns 120] [--timeout 1800] [--tag name] [--briefs briefs.json]
//          [--brain target/release/brain] [--deny brain_card,brain_impact,brain_why,brain_split]
// --brain: the binary to freeze for the run (default target/debug/brain; relative paths from the repo root).
// --deny: brain MCP tools the agent does not get (passed to --disallowedTools); used for ablations such as the
//         §43 before/after bench (bench/s43). The tool list the agent really saw is recorded per row (`tools`).
// Output: bench/autonomy/results/<tag>/<id>/ (the app + transcript.jsonl) and bench/autonomy/results/<tag>.json
import { spawn, execFileSync } from 'node:child_process'
import { mkdirSync, writeFileSync, readFileSync, rmSync, existsSync, statSync } from 'node:fs'
import { join } from 'node:path'
import { createHash } from 'node:crypto'

const ROOT = new URL('../..', import.meta.url).pathname
const HERE = join(ROOT, 'bench/autonomy')
const arg = (k, d) => { const i = process.argv.indexOf(`--${k}`); return i > 0 ? process.argv[i + 1] : d }
// Every argument must be a known `--flag value` pair: a flag glued to its value (`"--deny a,b"` as one word, as an
// unsplit shell variable produces) would otherwise be ignored silently and change what the run measures.
const FLAGS = ['model', 'mode', 'only', 'max-turns', 'timeout', 'tag', 'briefs', 'brain', 'deny']
for (let i = 2; i < process.argv.length; i += 2) {
  const f = process.argv[i]
  if (!f.startsWith('--') || !FLAGS.includes(f.slice(2)) || process.argv[i + 1] === undefined) {
    console.error(`bad argument \`${f}\`: expected --<${FLAGS.join('|')}> <value>`)
    process.exit(2)
  }
}
const MODEL = arg('model', 'claude-haiku-5-5')
const MODE = arg('mode', 'mcp-host')
const ONLY = arg('only', '')
const MAX_TURNS = arg('max-turns', '120')
const TIMEOUT_S = Number(arg('timeout', '1800'))
const TAG = arg('tag', `${MODEL}-${MODE}`)
if (!['mcp', 'mcp-host'].includes(MODE) || !/^claude-[a-z0-9-]+$/.test(MODEL)) {
  console.error(`bad arguments: --model ${MODEL} --mode ${MODE} (mode must be mcp or mcp-host)`)
  process.exit(2)
}

// Freeze the binary for the whole run so a concurrent rebuild cannot change the tool mid-benchmark.
const BRAIN_ARG = arg('brain', 'target/debug/brain')
const BRAIN_SRC = BRAIN_ARG.startsWith('/') ? BRAIN_ARG : join(ROOT, BRAIN_ARG)
const DENY = arg('deny', '').split(',').filter(Boolean)
if (DENY.some((t) => !/^brain_[a-z_]+$/.test(t))) { console.error(`bad --deny ${DENY}: brain_* tool names`); process.exit(2) }
const OUT = join(HERE, 'results', TAG)
mkdirSync(OUT, { recursive: true })
const BRAIN = join(OUT, 'brain')
// macOS kills (SIGKILL, code signature) a binary copied over a previous copy: remove it first.
rmSync(BRAIN, { force: true }); execFileSync('cp', [BRAIN_SRC, BRAIN])
const sha = (f) => createHash('sha256').update(readFileSync(f)).digest('hex').slice(0, 12)
const BRAIN_SHA = sha(BRAIN)
const SPEC_SHA = createHash('sha256').update(execFileSync(BRAIN, ['spec'])).digest('hex').slice(0, 12)

const SYSTEM = `You are an autonomous software agent. Nobody will answer questions: decide everything yourself.
Every page the brief names is a route (one file in views/). If the framework blocks something, keep the requirement and report the blocker in your final line.
You build web apps with BRAINLLM through the "brain" MCP tools. Read brain_spec before writing code.
The app root is the current directory: create the app there with brain_new (dir "."), never in a subdirectory.
You are done only when brain_doctor reports no errors (exit 0). Iterate until then, then stop with a one-line summary.`

const briefs = JSON.parse(readFileSync(join(HERE, arg('briefs', 'briefs.json')), 'utf8'))
  .filter((b) => !ONLY || ONLY.split(',').includes(b.id))

function agent(dir, brief) {
  const mcp = join(dir, '..', `${brief.id}.mcp.json`)
  writeFileSync(mcp, JSON.stringify({ mcpServers: { brain: { command: BRAIN, args: ['mcp'] } } }))
  const tools = MODE === 'mcp' ? '' : 'Read,Write,Edit'
  const allowed = MODE === 'mcp' ? 'mcp__brain' : 'mcp__brain,Read,Write,Edit'
  const args = ['-p', '--model', MODEL, '--output-format', 'stream-json', '--verbose', '--system-prompt', SYSTEM,
    '--tools', tools, '--allowedTools', allowed,
    ...(DENY.length ? ['--disallowedTools', DENY.map((t) => `mcp__brain__${t}`).join(',')] : []),
    '--setting-sources', '', '--strict-mcp-config', '--mcp-config', mcp,
    '--max-turns', MAX_TURNS, '--no-session-persistence', `Brief: ${brief.brief}`]
  return new Promise((resolve) => {
    const t0 = Date.now()
    const p = spawn('claude', args, { cwd: dir, stdio: ['ignore', 'pipe', 'pipe'] })
    const kill = setTimeout(() => p.kill('SIGTERM'), TIMEOUT_S * 1000)
    let out = '', err = ''
    p.stdout.on('data', (d) => (out += d))
    p.stderr.on('data', (d) => (err += d))
    p.on('close', (code) => {
      clearTimeout(kill)
      writeFileSync(join(dir, '..', `${brief.id}.transcript.jsonl`), out)
      resolve({ code, ms: Date.now() - t0, out, err: err.slice(0, 500), timedOut: Date.now() - t0 >= TIMEOUT_S * 1000 })
    })
  })
}

const hasStuck = (j) => j && typeof j === 'object' && (j.stuck === true || Object.values(j).some((v) => typeof v === 'object' && hasStuck(v)))

// A file read counts as big when the file it returned has more than BIG_LINES lines (or is paged: over 60 KB).
const BIG_LINES = 200
const resultText = (c) => (typeof c.content === 'string' ? c.content : (c.content ?? []).map((x) => x.text ?? '').join(''))

export function metrics(out) {
  const ev = out.split('\n').filter(Boolean).map((l) => { try { return JSON.parse(l) } catch { return null } }).filter(Boolean)
  const calls = {}, stuck = [], names = {}, reads = []
  let asked = false, tools = null, doctorCalls = 0, firstDoctorOk = null
  for (const e of ev) {
    if (e.type === 'system' && e.subtype === 'init' && Array.isArray(e.tools)) tools = e.tools.map((t) => t.split('__').pop())
    if (e.type === 'assistant') for (const c of e.message?.content ?? []) if (c.type === 'tool_use') {
      calls[c.name] = (calls[c.name] ?? 0) + 1
      names[c.id] = { name: c.name.split('__').pop(), input: c.input }
    }
    if (e.type === 'user') for (const c of e.message?.content ?? []) if (c.type === 'tool_result') {
      const n = names[c.tool_use_id]
      if (!n) continue
      let j
      try { j = JSON.parse(resultText(c)) } catch { j = null }
      if (n.name === 'brain_read') {
        // lines of the file the read returned; a paged read (nextOffset) is a file over 60 KB
        const src = typeof j?.source === 'string' ? j.source : null
        const lines = src === null ? null : src.split('\n').length - (src.endsWith('\n') ? 1 : 0)
        const paged = j?.nextOffset != null
        reads.push({ path: n.input?.path ?? null, lines, bytes: j?.size ?? null, paged, big: paged || (lines ?? 0) > BIG_LINES, error: src === null })
      } else if (n.name === 'brain_doctor' && j) {
        doctorCalls++
        if (firstDoctorOk === null) firstDoctorOk = j.ok === true
      }
    }
    if (e.type === 'user') for (const c of e.message?.content ?? []) {
      // Only a real tool answer counts: parse the result as JSON and look for a `stuck: true` field
      // (docs that mention `"stuck": true` in prose must not count).
      const texts = typeof c.content === 'string' ? [c.content] : (c.content ?? []).map((x) => x.text ?? '')
      for (const t of texts) { let j; try { j = JSON.parse(t) } catch { continue } if (hasStuck(j)) stuck.push(t.slice(0, 200)) }
    }
  }
  const last = ev.findLast((e) => e.type === 'result') ?? {}
  // A final message that asks the user something is a human intervention the agent needed.
  if (typeof last.result === 'string' && /\?\s*$/.test(last.result.trim())) asked = true
  const u = last.usage ?? {}
  return {
    toolCalls: Object.values(calls).reduce((a, b) => a + b, 0), calls, turns: last.num_turns, cost: last.total_cost_usd,
    inputTokens: (u.input_tokens ?? 0) + (u.cache_creation_input_tokens ?? 0) + (u.cache_read_input_tokens ?? 0),
    outputTokens: u.output_tokens ?? 0, stuckLoops: stuck.length, humanInterventions: asked ? 1 : 0,
    stopReason: last.subtype, final: typeof last.result === 'string' ? last.result.slice(0, 300) : null,
    reads: reads.length, bigReads: reads.filter((r) => r.big).length, maxReadLines: Math.max(0, ...reads.map((r) => r.lines ?? 0)),
    readList: reads, doctorCalls, firstDoctorOk, tools,
  }
}

function doctor(dir) {
  if (!existsSync(join(dir, 'app.brain'))) return { exit: -1, note: 'no app.brain in the app root' }
  try { execFileSync(BRAIN, ['doctor', '--json'], { cwd: dir, timeout: 300_000 }); return { exit: 0 } }
  catch (e) {
    let errors = null
    try { const j = JSON.parse(e.stdout.toString()); errors = (j.diags ?? j.errors ?? []).filter((d) => (d.level ?? 'error') === 'error').map((d) => d.code) } catch {}
    return { exit: e.status ?? -1, errors, raw: errors ? undefined : e.stdout?.toString().slice(0, 400) }
  }
}

const views = (dir) => { try { return execFileSync('fd', ['-e', 'brain', '.', 'views'], { cwd: dir }).toString().trim().split('\n').filter(Boolean).length } catch { return 0 } }

const rows = []
for (const b of briefs) {
  const dir = join(OUT, b.id)
  rmSync(dir, { recursive: true, force: true })
  mkdirSync(dir, { recursive: true })
  console.error(`→ ${b.id} (${MODEL}, ${MODE})`)
  const r = await agent(dir, b)
  const m = metrics(r.out)
  const d = doctor(dir)
  const v = views(dir)
  const row = { id: b.id, done: d.exit === 0 && v >= b.pages, doctorOk: d.exit === 0, doctor: d, views: v, pages: b.pages, ms: r.ms, timedOut: r.timedOut, exit: r.code, ...m }
  if (r.code !== 0 && !m.turns) {
    row.agentError = r.err
    console.error(`  agent failed to start (exit ${r.code}): ${r.err.slice(0, 200)}`)
  }
  rows.push(row)
  console.error(`  done=${row.done} reads=${row.reads} big=${row.bigReads} calls=${row.toolCalls} tokens=${row.inputTokens}/${row.outputTokens} stuck=${row.stuckLoops} ${(r.ms / 1000).toFixed(0)}s`)
}

const sum = (k) => rows.reduce((a, r) => a + (r[k] ?? 0), 0)
const report = {
  model: MODEL, mode: MODE, deny: DENY, brainSha: BRAIN_SHA, specSha: SPEC_SHA, at: new Date().toISOString(),
  done: rows.filter((r) => r.done).length, n: rows.length,
  totals: { toolCalls: sum('toolCalls'), reads: sum('reads'), bigReads: sum('bigReads'), inputTokens: sum('inputTokens'), outputTokens: sum('outputTokens'), stuckLoops: sum('stuckLoops'), humanInterventions: sum('humanInterventions'), cost: +sum('cost').toFixed(4) },
  rows,
}
writeFileSync(join(HERE, 'results', `${TAG}.json`), JSON.stringify(report, null, 1))
console.log(`| app | done (doctor=0 and routes ≥ brief) | routes/brief | tool calls | tokens in/out | stuck | human | time |\n|---|---|---:|---:|---|---:|---:|---:|`)
for (const r of rows) console.log(`| ${r.id} | ${r.done ? 'yes' : `no (${r.doctorOk ? 'routes pruned' : r.doctor.errors?.join(' ') ?? r.doctor.note ?? r.doctor.exit})`} | ${r.views}/${r.pages} | ${r.toolCalls} | ${r.inputTokens}/${r.outputTokens} | ${r.stuckLoops} | ${r.humanInterventions} | ${(r.ms / 1000).toFixed(0)} s |`)
console.log(`\n${report.done}/${report.n} done · ${MODEL} · ${MODE} · brain ${BRAIN_SHA} · spec ${SPEC_SHA}`)
