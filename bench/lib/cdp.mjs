// Minimal Chrome DevTools Protocol client (no deps): launches headless Chrome, opens a page, evaluates JS.
// Node ≥ 22 (global WebSocket). Chrome path: $BRAIN_CHROME or the default macOS install.
import { spawn } from 'node:child_process'
import { mkdtempSync, readFileSync, existsSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

const CHROME = process.env.BRAIN_CHROME ?? '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome'
const sleep = (ms) => new Promise((r) => setTimeout(r, ms))

export async function launch() {
  const dir = mkdtempSync(join(tmpdir(), 'qa-cdp-'))
  const proc = spawn(CHROME, ['--headless=new', '--remote-debugging-port=0', `--user-data-dir=${dir}`,
    // A fresh profile makes Chrome create "Chrome Safe Storage" in the macOS keychain, which fails from a
    // terminal session and pops a system alert on every run: use a mock keychain and a basic password store.
    '--use-mock-keychain', '--password-store=basic',
    '--no-first-run', '--no-default-browser-check', 'about:blank'], { stdio: 'ignore' })
  const portFile = join(dir, 'DevToolsActivePort')
  for (let i = 0; i < 100 && !existsSync(portFile); i++) await sleep(50)
  const port = Number(readFileSync(portFile, 'utf8').split('\n')[0])
  let closed = false
  const close = () => { if (closed) return; closed = true; proc.kill(); try { rmSync(dir, { recursive: true, force: true }) } catch {} }
  // Never leave an orphan Chrome when the measuring process dies early (crash, Ctrl-C, timeout).
  process.once('exit', close)
  for (const sig of ['SIGINT', 'SIGTERM']) process.once(sig, () => { close(); process.exit(130) })
  return { port, close }
}

export async function openPage(port, url) {
  const t = await (await fetch(`http://127.0.0.1:${port}/json/new?${encodeURI(url)}`, { method: 'PUT' })).json()
  const ws = new WebSocket(t.webSocketDebuggerUrl)
  await new Promise((r, j) => { ws.onopen = r; ws.onerror = j })
  let id = 0
  const pending = new Map(), listeners = []
  ws.onmessage = (e) => {
    const m = JSON.parse(e.data)
    if (m.id && pending.has(m.id)) { const { r, j } = pending.get(m.id); pending.delete(m.id); m.error ? j(new Error(m.error.message)) : r(m.result) }
    else if (m.method) for (const l of listeners) l(m)
  }
  const send = (method, params = {}) => new Promise((r, j) => { const k = ++id; pending.set(k, { r, j }); ws.send(JSON.stringify({ id: k, method, params })) })
  const evaluate = async (expression) => {
    const res = await send('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true })
    if (res.exceptionDetails) throw new Error(res.exceptionDetails.exception?.description ?? res.exceptionDetails.text)
    return res.result.value
  }
  await send('Page.enable'); await send('Runtime.enable')
  return { send, evaluate, on: (f) => listeners.push(f), close: () => ws.close() }
}
