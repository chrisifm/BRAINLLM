// Opt-in anonymous read-only SSR warmup. No background work or outbound request exists when it is absent.
import { createServer, request } from 'node:http'
import { Worker } from 'node:worker_threads'
const MAX_BODY = 256 * 1024, MAX_TOTAL = 2 * 1024 * 1024
const configError = () => Object.assign(new Error('server.warmup requires up to eight exact public cached SSR paths, a valid site origin, timeoutMs from 100 to 10000, and onError continue or abort'), { code:'WARMUP_CONFIG_INVALID' })
export function safePath(path) {
  if (typeof path !== 'string' || Buffer.byteLength(path) > 256 || !path.startsWith('/') || /[\\?#\s\u0000-\u0020\u007f-\u009f]/.test(path) || path.includes('//') || /%2f|%5c/i.test(path)) return false
  let decoded
  try { decoded = decodeURIComponent(path) } catch { return false }
  if (/[\\?#\s\u0000-\u0020\u007f-\u009f]/.test(decoded) || decoded.includes('//') || decoded.split('/').some(part => part === '.' || part === '..')) return false
  return new URL(path, 'http://warmup.invalid').pathname === path
}
export function warmupSettings(value, routes, base, site, redirects = {}) {
  if (value === undefined) return null
  if (!value || typeof value !== 'object' || Array.isArray(value) || Object.keys(value).some(key => !['paths','timeoutMs','onError'].includes(key))) throw configError()
  const paths = value.paths, timeoutMs = value.timeoutMs ?? 8000, onError = value.onError ?? 'continue'
  if (!Array.isArray(paths) || paths.length < 1 || paths.length > 8 || new Set(paths).size !== paths.length || !Number.isInteger(timeoutMs) || timeoutMs < 100 || timeoutMs > 10000 || !['continue','abort'].includes(onError)) throw configError()
  let origin
  try {
    if (typeof site !== 'string' || /[\s\u0000-\u0020\u007f-\u009f]/.test(site)) throw configError()
    origin = new URL(site)
    if (!['http:','https:'].includes(origin.protocol) || origin.username || origin.password || origin.search || origin.hash || origin.pathname !== '/' || !origin.hostname || origin.hostname.length > 253 || origin.host.length > 260 || /(?:^|\.)(?:example\.(?:com|org|net)|example|invalid|test)$/.test(origin.hostname)) throw configError()
  } catch { throw configError() }
  for (const path of paths) {
    if (!safePath(path) || base !== '/' && !path.startsWith(base)) throw configError()
    const local = base === '/' ? path : '/' + path.slice(base.length)
    if (/^\/(?:api|_brain|_r|_server|s|_health|health)(?:\/|$)/.test(local)) throw configError()
    const route = routes.find(route => {
      const canonical = redirects.slash === 'always' && route.path !== '/' && !route.path.endsWith('/') && !/\.[^/]*$/.test(route.path) ? route.path + '/' : route.path
      return canonical === local
    })
    if (typeof route?.server !== 'string' || !route.server || !Number.isSafeInteger(route.cache) || route.cache <= 0 || /[:*\[\]]/.test(route.path) || route.forms || route.guards?.length || route.middleware?.length) throw configError()
  }
  return Object.freeze({paths:Object.freeze([...paths]), timeoutMs, onError, host:origin.host})
}
const monotonic = () => process.hrtime.bigint()
const delay = ms => new Promise(resolve => setTimeout(resolve, ms))

// Called only by an explicitly warming production instance, before it binds its public listener.
export async function runWarmup({ server, settings, warm, prepare, log, started = monotonic() }) {
  const deadline = started + BigInt(settings.timeoutMs) * 1000000n
  const reserve = Math.min(1000, Math.max(25, Math.floor(settings.timeoutMs / 8)))
  const requestDeadline = deadline - BigInt(reserve) * 1000000n
  const state = { pending:new Set(), controllers:new Set(), deadline:requestDeadline }
  const watchdog = new Worker(new URL('./warmup-watchdog.mjs', import.meta.url), {workerData:{deadline:String(deadline)},env:{},execArgv:[]})
  const owned = new Set(), peers = new Map(), sockets = new Set(), handlers = new Set()
  let total = 0, failed = 0, stopped = false, timer, privateServer, modules
  const abort = () => {
    stopped = true
    for (const controller of state.controllers) controller.abort()
    for (const outgoing of owned) outgoing.destroy()
    for (const socket of sockets) socket.destroy()
  }
  const track = (promise, set) => {
    set.add(promise)
    promise.then(() => set.delete(promise), () => set.delete(promise))
    return promise
  }
  const secondsLeft = until => Math.max(0, Number(until - monotonic()) / 1e6)
  let failure
  try {
    await new Promise((resolve, reject) => { watchdog.once('message', message => message === 'armed' ? resolve() : reject(configError())); watchdog.once('error', reject); watchdog.once('exit', () => reject(configError())) })
    timer = setTimeout(abort, secondsLeft(requestDeadline))
    const controller = new AbortController()
    state.controllers.add(controller)
    modules = await track(Promise.resolve().then(() => warm(controller.signal)), state.pending)
    if (stopped) throw Object.assign(new Error('Startup warmup deadline'), {code:'WARMUP_TIMEOUT'})
    const dispatch = server.listeners('request')[0]
    privateServer = createServer((incoming, response) => {
      const peer = peers.get(incoming.socket.remotePort)
      if (!peer || peer.path !== incoming.url || incoming.method !== 'GET' || incoming.headers.authorization || incoming.headers.cookie || handlers.size >= 2) { response.writeHead(403,{connection:'close'}); response.end(); return }
      prepare(incoming, peer.controller, state)
      const handling = Promise.resolve().then(() => dispatch.call(server, incoming, response))
      track(handling, handlers).catch(() => { if (!response.headersSent) response.writeHead(500); response.end() })
    })
    privateServer.on('connection', socket => { sockets.add(socket); socket.once('close', () => sockets.delete(socket)) })
    privateServer.maxConnections = 2
    privateServer.maxRequestsPerSocket = 1
    privateServer.maxHeadersCount = 16
    privateServer.headersTimeout = settings.timeoutMs
    privateServer.requestTimeout = settings.timeoutMs
    await new Promise((resolve, reject) => { privateServer.once('error', reject); privateServer.listen(0, '127.0.0.1', resolve) })
    const port = privateServer.address().port
    const one = path => new Promise(resolve => {
      if (stopped) return resolve()
      const controller = new AbortController()
      state.controllers.add(controller)
      let outgoing, settled = false, localPort
      const finish = ok => {
        if (settled) return
        settled = true
        if (!ok) { failed++; controller.abort() }
        owned.delete(outgoing); peers.delete(localPort); state.controllers.delete(controller)
        resolve()
      }
      outgoing = request({host:'127.0.0.1',port,path,method:'GET',agent:false,headers:{host:settings.host,accept:'text/html','accept-encoding':'identity',connection:'close'}}, response => {
        let bytes = 0
        const rejected = response.statusCode !== 200 || response.headers['set-cookie'] || /(?:private|no-store|no-cache)/i.test(response.headers['cache-control'] || '') || response.headers['content-encoding'] || Number(response.headers['content-length']) > MAX_BODY
        if (rejected) { response.destroy(); outgoing.destroy(); return finish(false) }
        response.on('data', chunk => {
          bytes += chunk.length; total += chunk.length
          if (bytes > MAX_BODY || total > MAX_TOTAL) { response.destroy(); outgoing.destroy(); finish(false) }
        })
        response.once('end', () => finish(true))
        response.once('error', () => finish(false))
        response.once('aborted', () => finish(false))
      })
      owned.add(outgoing)
      outgoing.once('socket', socket => socket.once('connect', () => { localPort = socket.localPort; peers.set(localPort, {path,controller}) }))
      outgoing.once('error', () => finish(false))
      outgoing.end()
    })
    let next = 0
    await Promise.all(Array.from({length:Math.min(2,settings.paths.length)}, async () => { while (!stopped && next < settings.paths.length) await one(settings.paths[next++]) }))
    if (stopped) throw Object.assign(new Error('Startup warmup deadline'), {code:'WARMUP_TIMEOUT'})
    if (failed && settings.onError === 'abort') throw Object.assign(new Error('Required startup warmup failed'), {code:'WARMUP_FAILED'})
  } catch (error) { failure = error }
  finally {
    abort()
    if (privateServer) {
      privateServer.closeAllConnections()
      await new Promise(resolve => { if (!privateServer.listening) return resolve(); privateServer.close(resolve) })
    }
    // A load/import that ignores cancellation is not allowed to run behind a ready process.
    // The independent watchdog terminates this owned instance if cleanup cannot finish within the absolute budget.
    while (owned.size || sockets.size || handlers.size || state.pending.size) await delay(1)
    clearTimeout(timer)
    await watchdog.terminate()
  }
  if (monotonic() >= deadline) throw Object.assign(new Error('Startup warmup cleanup exceeded its absolute deadline'),{code:'WARMUP_DEADLINE'})
  log?.(failed || failure ? 'warn' : 'info', 'startup data warmup', {event:'startup-data-warmup',requested:settings.paths.length,failed,bytes:total,code:failure?.code || (failed ? 'WARMUP_PARTIAL' : 'WARMUP_READY'),private:true,valuesCaptured:false})
  if (failure && settings.onError === 'abort') throw failure
  return {...modules,dataPaths:settings.paths.length,dataFailed:failed}
}
