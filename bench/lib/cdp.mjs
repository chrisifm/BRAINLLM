// Minimal Chrome DevTools Protocol client (no deps): launches headless Chrome, opens a page, evaluates JS.
// Node ≥ 22 (global WebSocket). Chrome path: $BRAIN_CHROME or the default macOS install.
import { spawn } from 'node:child_process'
import { mkdtempSync, openSync, readSync, closeSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

const CHROME = process.env.BRAIN_CHROME ?? '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome'
const sleep = (ms) => new Promise((r) => setTimeout(r, ms))
const browserPath = /^\/devtools\/browser(?:\/[a-f\d]{8}-[a-f\d]{4}-[a-f\d]{4}-[a-f\d]{4}-[a-f\d]{12})?$/i

// Chromium creates this file before its port/browser address is necessarily complete.
// Bound the read and recognize both supported address forms. The endpoint probe below resolves
// the ambiguity between a complete bare path and that same prefix of a still-being-written UUID.
export function readDebuggingAddress(file) {
  let fd
  try {
    fd = openSync(file, 'r')
    const bytes = Buffer.alloc(256), size = readSync(fd, bytes, 0, bytes.length, 0)
    if (size === bytes.length) return null
    const lines = bytes.subarray(0, size).toString('utf8').split(/\r?\n/)
    if (!(lines.length === 2 || (lines.length === 3 && lines[2] === '')) || !/^\d{1,5}$/.test(lines[0]) || !browserPath.test(lines[1])) return null
    const port = Number(lines[0])
    return port >= 1 && port <= 65535 ? { port, path: lines[1] } : null
  } catch (error) {
    if (error.code === 'ENOENT') return null
    throw error
  } finally { if (fd !== undefined) closeSync(fd) }
}

export async function waitDebuggingAddress(file, { timeoutMs = 5000, check = () => {} } = {}) {
  if (!Number.isInteger(timeoutMs) || timeoutMs < 1 || timeoutMs > 30000) throw Error('Chrome debugging startup deadline must be 1..30000 ms')
  const deadline = performance.now() + timeoutMs
  for (;;) {
    check()
    const address = readDebuggingAddress(file)
    const left = deadline - performance.now()
    if (left <= 0) throw new Error('Chrome debugging startup deadline')
    if (address) {
      try {
        const response = await fetch(`http://127.0.0.1:${address.port}/json/version`, { redirect: 'error', signal: AbortSignal.timeout(Math.max(1, Math.min(1000, Math.ceil(left)))) })
        if (!response.ok) { await response.body?.cancel(); throw Error('Chrome debugging endpoint unavailable') }
        const chunks = []; let bytes = 0
        for await (const chunk of response.body) {
          bytes += chunk.length
          if (bytes > 16384) throw Error('Chrome debugging discovery exceeded its size limit')
          chunks.push(chunk)
        }
        const info = JSON.parse(Buffer.concat(chunks, bytes).toString('utf8'))
        const target = new URL(info.webSocketDebuggerUrl)
        if (target.protocol !== 'ws:' || !['127.0.0.1', 'localhost', '[::1]'].includes(target.hostname) || Number(target.port || 80) !== address.port || target.username || target.password || target.search || target.hash || !browserPath.test(target.pathname)) throw Error('Chrome debugging discovery returned a different endpoint')
        check()
        if (performance.now() <= deadline) return { port: address.port, path: target.pathname }
      } catch { /* The same startup budget covers a not-yet-ready discovery endpoint. */ }
    }
    const remaining = deadline - performance.now()
    if (remaining <= 0) throw new Error('Chrome debugging startup deadline')
    await sleep(Math.min(50, remaining))
  }
}

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
    const { port } = await waitDebuggingAddress(portFile, { check: () => {
      if (startupError) throw startupError
      if (proc.exitCode !== null || proc.signalCode !== null) throw new Error('Chrome exited before debugging was ready')
    } })
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
