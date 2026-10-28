// CDP over one browser WebSocket with flattened sessions (no deps, Node ≥ 22). Extends bench/lib/cdp.mjs with what
// the cache bench needs: a persistent profile (repeat visits share disk cache, Cache API and service workers), the
// Network/Emulation domains on the page AND on every service-worker target (throttling and offline must reach the
// worker's own fetches), and a per-page request ledger.
import { spawn } from 'node:child_process'
import { readFileSync, existsSync, rmSync, mkdirSync } from 'node:fs'
import { join } from 'node:path'

const CHROME = process.env.BRAIN_CHROME ?? '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome'
export const sleep = (ms) => new Promise((r) => setTimeout(r, ms))

// Slow 4G as requested: 150 ms RTT, 1.6 Mbps down, 750 Kbps up; CPU 4x.
export const SLOW_4G = { offline: false, latency: 150, downloadThroughput: (1.6 * 1024 * 1024) / 8, uploadThroughput: (750 * 1024) / 8 }
export const OFFLINE = { offline: true, latency: 0, downloadThroughput: -1, uploadThroughput: -1 }
export const CPU_RATE = 4

export async function launchBrowser(profileDir) {
  rmSync(profileDir, { recursive: true, force: true })
  mkdirSync(profileDir, { recursive: true })
  const proc = spawn(CHROME, ['--headless=new', '--remote-debugging-port=0', `--user-data-dir=${profileDir}`, '--no-first-run', '--use-mock-keychain', '--password-store=basic',
    '--no-default-browser-check', '--disable-background-networking', '--disable-component-update', '--disable-sync', 'about:blank'], { stdio: 'ignore' })
  const portFile = join(profileDir, 'DevToolsActivePort')
  for (let i = 0; i < 200 && !existsSync(portFile); i++) await sleep(50)
  const [port, path] = readFileSync(portFile, 'utf8').split('\n')
  const ws = new WebSocket(`ws://127.0.0.1:${port}${path}`)
  await new Promise((r, j) => { ws.onopen = r; ws.onerror = j })
  let id = 0
  const pending = new Map(), listeners = new Set()
  ws.onmessage = (e) => {
    const m = JSON.parse(e.data)
    if (m.id && pending.has(m.id)) { const { r, j } = pending.get(m.id); pending.delete(m.id); m.error ? j(new Error(m.error.message)) : r(m.result) }
    else if (m.method) for (const l of listeners) l(m)
  }
  const send = (method, params = {}, sessionId) => new Promise((r, j) => {
    const k = ++id
    pending.set(k, { r, j })
    ws.send(JSON.stringify(sessionId ? { id: k, method, params, sessionId } : { id: k, method, params }))
  })
  const on = (f) => { listeners.add(f); return () => listeners.delete(f) }

  // service-worker targets: attach, apply the current network conditions, record their network traffic
  let conditions = null
  const swSessions = new Map() // targetId → sessionId
  const swLedger = []
  on(async (m) => {
    if ((m.method === 'Target.targetCreated' || m.method === 'Target.targetInfoChanged') && m.params.targetInfo.type === 'service_worker') {
      const t = m.params.targetInfo
      if (swSessions.has(t.targetId)) return
      swSessions.set(t.targetId, null)
      try {
        const { sessionId } = await send('Target.attachToTarget', { targetId: t.targetId, flatten: true })
        swSessions.set(t.targetId, sessionId)
        await send('Network.enable', {}, sessionId)
        await send('Network.setCacheDisabled', { cacheDisabled: false }, sessionId)
        if (conditions) await send('Network.emulateNetworkConditions', conditions, sessionId)
        await send('Runtime.runIfWaitingForDebugger', {}, sessionId).catch(() => {})
      } catch {}
    }
    if (m.method === 'Target.targetDestroyed') swSessions.delete(m.params.targetId)
    if (m.sessionId && [...swSessions.values()].includes(m.sessionId) && m.method === 'Network.responseReceived') {
      const r = m.params.response
      swLedger.push({ t: Date.now(), url: r.url, status: r.status, fromDisk: !!r.fromDiskCache, fromSW: !!r.fromServiceWorker })
    }
  })
  await send('Target.setDiscoverTargets', { discover: true })

  const setConditions = async (c, pageSession) => {
    conditions = c
    const all = [pageSession, ...swSessions.values()].filter(Boolean)
    for (const s of all) await send('Network.emulateNetworkConditions', c, s).catch(() => {})
  }

  async function newPage({ mobile = true } = {}) {
    const { targetId } = await send('Target.createTarget', { url: 'about:blank' })
    const { sessionId } = await send('Target.attachToTarget', { targetId, flatten: true })
    const s = (method, params) => send(method, params, sessionId)
    const reqs = new Map(), navs = []
    const off = on((m) => {
      if (m.sessionId !== sessionId) return
      const p = m.params
      if (m.method === 'Network.requestWillBeSent') reqs.set(p.requestId, { t: Date.now(), url: p.request.url, type: p.type, status: 0, build: null, fromSW: false, fromDisk: false, fromMemory: false, fromPrefetch: false, encoded: 0, failed: false })
      if (m.method === 'Network.requestServedFromCache' && reqs.has(p.requestId)) reqs.get(p.requestId).fromMemory = true
      if (m.method === 'Network.responseReceived' && reqs.has(p.requestId)) {
        const r = reqs.get(p.requestId)
        const hh = Object.fromEntries(Object.entries(p.response.headers || {}).map(([k, v]) => [k.toLowerCase(), v]))
        Object.assign(r, { build: hh['x-brain-build'] ?? null, status: p.response.status, fromSW: !!p.response.fromServiceWorker, fromDisk: !!p.response.fromDiskCache || r.fromMemory, fromPrefetch: !!p.response.fromPrefetchCache })
      }
      if (m.method === 'Network.loadingFinished' && reqs.has(p.requestId)) reqs.get(p.requestId).encoded = p.encodedDataLength
      if (m.method === 'Page.frameNavigated' && !p.frame.parentId) navs.push({ t: Date.now(), kind: 'document', url: p.frame.url })
      if (m.method === 'Page.navigatedWithinDocument') navs.push({ t: Date.now(), kind: 'same-document', url: p.url })
      if (m.method === 'Page.loadEventFired') navs.push({ t: Date.now(), kind: 'load' })
      if (m.method === 'Network.loadingFailed' && reqs.has(p.requestId)) reqs.get(p.requestId).failed = p.errorText
    })
    await s('Page.enable'); await s('Runtime.enable'); await s('Network.enable')
    if (mobile) await s('Emulation.setDeviceMetricsOverride', { width: 412, height: 915, deviceScaleFactor: 2.625, mobile: true })
    await s('Emulation.setCPUThrottlingRate', { rate: CPU_RATE })
    await s('Page.addScriptToEvaluateOnNewDocument', {
      source: "self.__lcp=null;try{new PerformanceObserver(l=>{for(const e of l.getEntries())self.__lcp=e.startTime}).observe({type:'largest-contentful-paint',buffered:true})}catch(e){}",
    })
    const evaluate = async (expression, timeout = 20000) => {
      const res = await Promise.race([
        s('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true }),
        sleep(timeout).then(() => ({ exceptionDetails: { text: 'evaluate timeout' } })),
      ])
      if (res.exceptionDetails) throw new Error(res.exceptionDetails.exception?.description ?? res.exceptionDetails.text)
      return res.result.value
    }
    const goto = async (url, timeout = 30000) => {
      const loaded = new Promise((r) => { const o = on((m) => { if (m.sessionId === sessionId && m.method === 'Page.loadEventFired') { o(); r(true) } }); setTimeout(() => { o(); r(false) }, timeout) })
      const nav = await s('Page.navigate', { url })
      const ok = await loaded
      return { ok, errorText: nav.errorText || null }
    }
    return {
      sessionId, send: s, evaluate, goto,
      ledger: () => [...reqs.values()],
      navs: () => navs,
      resetLedger: () => reqs.clear(),
      close: async () => { off(); await send('Target.closeTarget', { targetId }).catch(() => {}) },
    }
  }

  return {
    send, on, newPage, setConditions, swLedger,
    close: async () => { try { ws.close() } catch {} proc.kill('SIGTERM'); await sleep(300); try { process.kill(proc.pid, 0); proc.kill('SIGKILL') } catch {} },
  }
}
