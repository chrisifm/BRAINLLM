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
  let closed = false
  const signals = [['SIGINT', () => { close(); process.exit(130) }], ['SIGTERM', () => { close(); process.exit(143) }]]
  const close = () => {
    if (closed) return
    closed = true
    process.off('exit', close)
    for (const [signal, handler] of signals) process.off(signal, handler)
    proc.kill()
    try { rmSync(dir, { recursive: true, force: true }) } catch {}
  }
  let startupError
  proc.on('error', error => { startupError = new Error(`Chrome did not start: ${error.code || 'spawn failed'}`); close() })
  // Never leave an orphan Chrome when the measuring process dies early (crash, Ctrl-C, timeout).
  process.once('exit', close)
  for (const [signal, handler] of signals) process.once(signal, handler)
  try {
    for (let i = 0; i < 100 && !existsSync(portFile); i++) {
      if (startupError) throw startupError
      if (proc.exitCode !== null || proc.signalCode !== null) throw new Error('Chrome exited before debugging was ready')
      await sleep(50)
    }
    if (startupError) throw startupError
    if (!existsSync(portFile)) throw new Error('Chrome debugging startup deadline')
    const port = Number(readFileSync(portFile, 'utf8').split('\n')[0])
    if (!Number.isInteger(port) || port < 1 || port > 65535) throw new Error('Chrome returned an invalid debugging port')
    return { port, close }
  } catch (error) { close(); throw error }
}

export async function openPage(port, url) {
  // Chrome writes DevToolsActivePort a moment before its HTTP endpoint answers.
  const readyUntil = Date.now() + 5000
  for (;;) {
    try {
      const response = await fetch(`http://127.0.0.1:${port}/json/version`, { signal: AbortSignal.timeout(1000) })
      if (!response.ok) throw new Error('Chrome debugging endpoint is unavailable')
      break
    } catch (error) { if (Date.now() >= readyUntil) throw error; await sleep(50) }
  }
  // Enable domains before navigation so initial execution contexts and navigation events can be observed.
  // Creating a target is a mutation, so do it once after the read-only readiness probe.
  const response = await fetch(`http://127.0.0.1:${port}/json/new?about:blank`, { method: 'PUT', signal: AbortSignal.timeout(5000) })
  if (!response.ok) throw new Error('Chrome could not create the owned page')
  const t = await response.json()
  const ws = new WebSocket(t.webSocketDebuggerUrl)
  await new Promise((r, j) => {
    const timer = setTimeout(() => { ws.close(); j(new Error('Chrome debugging connection deadline')) }, 5000)
    ws.onopen = () => { clearTimeout(timer); r() }
    ws.onerror = () => { clearTimeout(timer); j(new Error('Chrome debugging connection failed')) }
  })
  let id = 0
  const pending = new Map(), listeners = []
  ws.onmessage = (e) => {
    const m = JSON.parse(e.data)
    if (m.id && pending.has(m.id)) { const { r, j, timer } = pending.get(m.id); pending.delete(m.id); clearTimeout(timer); m.error ? j(new Error(m.error.message)) : r(m.result) }
    else if (m.method) for (const l of listeners) l(m)
  }
  const disconnected = () => {
    for (const entry of pending.values()) { clearTimeout(entry.timer); entry.j(new Error('Chrome debugging connection closed')) }
    pending.clear()
  }
  ws.onclose = disconnected
  ws.onerror = disconnected
  const send = (method, params = {}, timeoutMs = 15000) => new Promise((r, j) => {
    if (!Number.isInteger(timeoutMs) || timeoutMs < 1 || timeoutMs > 120000) return j(new Error('Chrome debugging deadline must be 1..120000 ms'))
    if (ws.readyState !== WebSocket.OPEN || pending.size >= 64) return j(new Error('Chrome debugging connection unavailable or busy'))
    const k = ++id
    const timer = setTimeout(() => { pending.delete(k); j(new Error(`Chrome debugging command deadline: ${method}`)) }, timeoutMs)
    pending.set(k, { r, j, timer })
    try { ws.send(JSON.stringify({ id: k, method, params })) } catch (error) { clearTimeout(timer); pending.delete(k); j(error) }
  })
  const evaluate = async (expression, timeoutMs = 15000) => {
    const res = await send('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true }, timeoutMs)
    if (res.exceptionDetails) throw new Error(res.exceptionDetails.exception?.description ?? res.exceptionDetails.text)
    return res.result.value
  }
  try { await send('Page.enable'); await send('Runtime.enable'); await send('Page.navigate', { url }) }
  catch (error) { disconnected(); ws.close(); throw error }
  return { send, evaluate, on: (f) => listeners.push(f), close: () => { disconnected(); ws.close() } }
}
