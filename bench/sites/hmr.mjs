// #21 — HMR latency measured in Chrome with the client's own clock (`brain:hmr` event, SPEC §8 / TOOLCHAIN).
// latency = appliedAt − write time · server = receivedAt − write time · client = appliedAt − receivedAt (ms, epoch).
// CSS: edit only <style>, assert no reload (performance.timeOrigin unchanged).
// JS: click the counter to 5, edit <script>, assert kind 'js' and the counter still shows 5 (needs codegen #7).
// Usage: node bench/sites/hmr.mjs [--app <dir>] [--reps 10] [--port 5191]
import { spawn, execFileSync } from 'node:child_process'
import { readFileSync, writeFileSync, existsSync, rmSync, mkdirSync } from 'node:fs'
import { join } from 'node:path'
import { tmpdir } from 'node:os'
import { launch, openPage } from '../lib/cdp.mjs'

const ROOT = new URL('../..', import.meta.url).pathname
const BRAIN = process.env.BRAIN ?? join(ROOT, 'target/debug/brain')
const argv = process.argv.slice(2)
const arg = (k, d) => { const i = argv.indexOf(`--${k}`); return i >= 0 ? argv[i + 1] : d }
const REPS = Number(arg('reps', 10)), PORT = Number(arg('port', 5191))
const sleep = (ms) => new Promise((r) => setTimeout(r, ms))

let APP = arg('app')
if (!APP) { // fresh scaffold: views/home.brain mounts Counter
  const base = process.env.QA_HMR_DIR ?? join(tmpdir(), 'brainllm-hmr'); mkdirSync(base, { recursive: true })
  rmSync(join(base, 'app'), { recursive: true, force: true })
  execFileSync(BRAIN, ['new', 'app'], { cwd: base, stdio: 'ignore' }); APP = join(base, 'app')
}
const VIEW = join(APP, 'views/home.brain'), COUNTER = join(APP, 'comp/counter.brain')
const original = { view: readFileSync(VIEW, 'utf8'), counter: readFileSync(COUNTER, 'utf8') }

const dev = spawn(BRAIN, ['dev', '--port', String(PORT)], { cwd: APP, stdio: ['ignore', 'pipe', 'pipe'] })
const chrome = await launch()
const cleanup = () => { writeFileSync(VIEW, original.view); writeFileSync(COUNTER, original.counter); dev.kill(); chrome.close() }
process.on('exit', cleanup)
for (let i = 0; i < 100; i++) { try { await fetch(`http://127.0.0.1:${PORT}/`); break } catch { await sleep(100) } }

const page = await openPage(chrome.port, `http://127.0.0.1:${PORT}/`)
await sleep(800)
await page.evaluate(`window.__qa = []; addEventListener('brain:hmr', (e) => __qa.push(e.detail)); true`)
const origin0 = await page.evaluate('performance.timeOrigin')

async function waitEvent(n, timeout = 5000) {
  for (let t = 0; t < timeout; t += 10) {
    const evs = await page.evaluate('window.__qa ? __qa.length : -1').catch(() => -1)
    if (evs >= n) return page.evaluate(`__qa[${n - 1}]`)
    if (evs === -1) return { kind: 'reload-detected' }
    await sleep(10)
  }
  return null
}

const stats = (xs) => { const s = [...xs].sort((a, b) => a - b); return { n: s.length, median: +s[s.length >> 1].toFixed(1), p95: +s[Math.min(s.length - 1, Math.floor(s.length * 0.95))].toFixed(1), min: +s[0].toFixed(1), max: +s.at(-1).toFixed(1) } }

// CSS-only edits: alternate a padding value inside <style>.
const css = []
for (let i = 0; i < REPS; i++) {
  const next = original.view.replace(/<style>([\s\S]*?)<\/style>/, (m, body) => `<style>${body}  .qa-hmr-${i} { padding: ${i + 1}px; }\n</style>`)
  const writtenAt = Date.now()
  writeFileSync(VIEW, next)
  const ev = await waitEvent(i + 1)
  if (!ev || ev.kind !== 'css') { css.push({ i, error: ev ? `kind ${ev.kind}` : 'timeout' }); break }
  css.push({ i, latency: ev.appliedAt - writtenAt, server: ev.receivedAt - writtenAt, client: ev.appliedAt - ev.receivedAt })
}
const reloaded = (await page.evaluate('performance.timeOrigin').catch(() => null)) !== origin0
writeFileSync(VIEW, original.view)
await sleep(300)

// JS edit with state: click to 5, change the counter's <script>, expect kind 'js' and 5 preserved.
let js = { status: 'skipped' }
try {
  await page.evaluate(`window.__qa = []; true`)
  const read = `document.querySelector('button')?.textContent ?? ''`
  for (let i = 0; i < 5; i++) await page.evaluate(`document.querySelector('button').click(); true`)
  await sleep(50)
  const before = await page.evaluate(read)
  const writtenAt = Date.now()
  writeFileSync(COUNTER, original.counter.replace('n * 2', 'n * 3')) // semantic edit: comments are stripped and emit nothing
  const ev = await waitEvent(1)
  await sleep(100)
  const after = await page.evaluate(read).catch(() => null)
  js = { status: ev?.kind === 'js' && /Clicks: 5\b/.test(before) && /Clicks: 5\b/.test(after ?? '') && /\b15\b/.test(after ?? '') ? 'ok' : 'fail', kind: ev?.kind ?? 'timeout',
    before, after, latency: ev?.appliedAt ? ev.appliedAt - writtenAt : null }
} catch (e) { js = { status: 'error', error: e.message } }

const ok = css.filter((r) => r.latency !== undefined)
const result = { at: new Date().toISOString(), app: APP, reps: REPS,
  css: ok.length ? { latency: stats(ok.map((r) => r.latency)), server: stats(ok.map((r) => r.server)), client: stats(ok.map((r) => r.client)), reloaded, errors: css.filter((r) => r.error) } : { errors: css },
  js }
const out = join(ROOT, 'bench/sites/results'); mkdirSync(out, { recursive: true })
writeFileSync(join(out, 'hmr.json'), JSON.stringify(result, null, 2))
console.log(JSON.stringify(result, null, 2))
page.close(); process.exit(0)
