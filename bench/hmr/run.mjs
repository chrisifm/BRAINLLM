// HMR end to end (INTERNALS §28): from the file write to the change applied in Chrome, per edit kind.
// Clock: the `brain:hmr` event of the dev client (receivedAt / appliedAt, epoch ms, same base as Date.now()).
//   total  = appliedAt − writtenAt   (includes the OS file-event latency)
//   server = receivedAt − writtenAt  (file event + compile + WebSocket send)
//   client = appliedAt − receivedAt  (import of the new module / stylesheet + remount in the page)
//   compile = the ms the dev server logs per update · tool = compile + client (the §28 budget: ≤ 20 ms, OS file event excluded)
// Kinds: style (only <style>), code (markup/script, state must survive), contract (props change: importers re-checked).
// Usage: node bench/hmr/run.mjs [--reps 20] [--port 5194]   → bench/hmr/results/hmr.json
import { spawn, execFileSync } from 'node:child_process'
import { readFileSync, writeFileSync, rmSync, mkdirSync } from 'node:fs'
import { join } from 'node:path'
import { tmpdir } from 'node:os'
import { launch, openPage } from '../lib/cdp.mjs'

const ROOT = new URL('../..', import.meta.url).pathname
const BRAIN = process.env.BRAIN ?? join(ROOT, 'target/debug/brain')
const argv = process.argv.slice(2)
const arg = (k, d) => { const i = argv.indexOf(`--${k}`); return i >= 0 ? argv[i + 1] : d }
const REPS = Number(arg('reps', 20)), PORT = Number(arg('port', 5194))
const sleep = (ms) => new Promise((r) => setTimeout(r, ms))

const base = process.env.QA_HMR_DIR ?? join(tmpdir(), 'brainllm-hmr-e2e')
rmSync(base, { recursive: true, force: true }); mkdirSync(base, { recursive: true })
execFileSync(BRAIN, ['new', 'app'], { cwd: base, stdio: 'ignore' })
const APP = join(base, 'app'), COUNTER = join(APP, 'comp/counter.brain')
writeFileSync(join(APP, 'views/home.brain'), `<meta seo title="HMR" description="HMR latency probe page." canonical="/">
<script>
  import Counter from '../comp/counter.brain'
</script>
<main>
  <h1>HMR probe</h1>
  <Counter />
</main>
`)
const original = readFileSync(COUNTER, 'utf8')
const brainSha = execFileSync('shasum', ['-a', '256', BRAIN]).toString().slice(0, 12)

const dev = spawn(BRAIN, ['run', '--port', String(PORT)], { cwd: APP, stdio: ['ignore', 'pipe', 'pipe'] })
// The dev server logs `[hmr] <path> <ms> ms (…)` per update: the compile time inside the tool, without the OS file event.
const compiles = []
const onLog = (d) => { for (const m of String(d).matchAll(/\[hmr\] \S+ ([\d.]+) ms/g)) compiles.push(Number(m[1])) }
dev.stdout.on('data', onLog); dev.stderr.on('data', onLog)
const chrome = await launch()
process.on('exit', () => { dev.kill(); chrome.close() })
for (let i = 0; i < 100; i++) { try { await fetch(`http://127.0.0.1:${PORT}/`); break } catch { await sleep(100) } }
const page = await openPage(chrome.port, `http://127.0.0.1:${PORT}/`)
await sleep(1000)
const arm = () => page.evaluate(`window.__qa = []; window.__qa_armed || addEventListener('brain:hmr', (e) => __qa.push(e.detail)); window.__qa_armed = true; true`)
await arm()
const origin0 = await page.evaluate('performance.timeOrigin')
const label = `document.querySelector('button')?.textContent ?? ''`
for (let i = 0; i < 5; i++) await page.evaluate(`document.querySelector('button').click(); true`)

async function waitEvent(timeout = 5000) {
  for (let t = 0; t < timeout; t += 2) {
    const ev = await page.evaluate('window.__qa && __qa.length ? __qa.shift() : null').catch(() => 'gone')
    if (ev === 'gone') return { kind: 'reload-detected' }
    if (ev) return ev
    await sleep(2)
  }
  return null
}

const edits = {
  style: (i) => original.replace('var(--space-2) var(--space-4)', i % 2 ? 'var(--space-2) var(--space-4)' : 'var(--space-3) var(--space-4)'),
  code: (i) => original.replace('n * 2', i % 2 ? 'n * 2' : 'n * 3').replace('× 2', i % 2 ? '× 2' : '× 3'),
  // A contract change that alters the output (a prop default); an unused new prop emits identical JS and no update.
  contract: (i) => original.replace("label:str='Clicks'", i % 2 ? "label:str='Clicks'" : "label:str='Taps'"),
}
const stats = (xs) => {
  if (!xs.length) return null
  const s = [...xs].sort((a, b) => a - b), q = (p) => +s[Math.min(s.length - 1, Math.ceil(p * s.length) - 1)].toFixed(2)
  return { n: s.length, median: q(0.5), p95: q(0.95), min: +s[0].toFixed(2), max: +s.at(-1).toFixed(2) }
}

const result = { at: new Date().toISOString(), brainSha, reps: REPS, kinds: {} }
for (const [kind, edit] of Object.entries(edits)) {
  writeFileSync(COUNTER, original); await sleep(400); await arm()
  const rows = []
  for (let i = 0; i < REPS; i++) {
    await sleep(120) // let the previous update settle so edits are not coalesced
    const c0 = compiles.length
    const writtenAt = Date.now()
    writeFileSync(COUNTER, edit(i))
    const ev = await waitEvent()
    for (let t = 0; t < 100 && compiles.length === c0; t += 5) await sleep(5) // the log line may trail the WebSocket message
    const compile = compiles.length > c0 ? compiles.at(-1) : null
    if (!ev || ev.kind === 'reload-detected') { rows.push({ i, error: ev ? ev.kind : 'timeout' }); continue }
    rows.push({ i, kind: ev.kind, total: ev.appliedAt - writtenAt, server: ev.receivedAt - writtenAt, client: ev.appliedAt - ev.receivedAt, compile, tool: compile === null ? null : compile + (ev.appliedAt - ev.receivedAt) })
  }
  const ok = rows.filter((r) => r.total !== undefined)
  const kinds = [...new Set(ok.map((r) => r.kind))]
  result.kinds[kind] = { applied: kinds, total: stats(ok.map((r) => r.total)), server: stats(ok.map((r) => r.server)), client: stats(ok.map((r) => r.client)), compile: stats(ok.filter((r) => r.compile !== null).map((r) => r.compile)), tool: stats(ok.filter((r) => r.tool !== null).map((r) => r.tool)), errors: rows.filter((r) => r.error).length }
}
writeFileSync(COUNTER, original); await sleep(300)
const after = await page.evaluate(label).catch(() => null)
result.statePreserved = /Clicks: 5\b/.test(after ?? '')
result.reloaded = (await page.evaluate('performance.timeOrigin').catch(() => null)) !== origin0
result.chrome = await page.evaluate('navigator.userAgent').catch(() => null)

const out = join(ROOT, 'bench/hmr/results'); mkdirSync(out, { recursive: true })
writeFileSync(join(out, 'hmr.json'), JSON.stringify(result, null, 2))
console.log('| edit | applied as | tool = compile + client, median / p95 (ms) | compile | client | total incl. OS file event | errors |\n|---|---|---|---|---|---|---:|')
for (const [k, v] of Object.entries(result.kinds)) {
  const f = (s) => s ? `${s.median} / ${s.p95}` : '—'
  console.log(`| ${k} | ${v.applied.join(', ')} | ${f(v.tool)} | ${f(v.compile)} | ${f(v.client)} | ${f(v.total)} | ${v.errors} |`)
}
console.log(`\nstate preserved after all edits: ${result.statePreserved} · page reloaded: ${result.reloaded} · brain ${brainSha}`)
page.close(); process.exit(0)
