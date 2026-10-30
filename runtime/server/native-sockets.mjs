// Fixed loopback Upgrade transport. Rust owns WebSocket frames, authentication and per-delivery policy.
import {request} from 'node:http'
import {createHash, randomUUID} from 'node:crypto'

const PROTOCOL = 'brain.v1'
const LIMITS = Object.freeze({connections:32, headers:16384, credentials:16384, head:4096, upstreamHead:8192, queued:262144, bytes:64*1024*1024, handshake:5000, write:2000, idle:70000, lifetime:15*60*1000, close:1000})
const SINGLE = new Set(['host','origin','authorization','cookie','upgrade','sec-websocket-key','sec-websocket-version','sec-websocket-protocol'])
const installed = new WeakSet()
const connectionUpgrade = value => typeof value === 'string' && value.toLowerCase().split(/\s*,\s*/).includes('upgrade')
const unique = (headers, names) => {
  const seen = new Set()
  for (let i=0;i<headers.length;i+=2) {
    const name = headers[i].toLowerCase()
    if (names.has(name) && seen.has(name)) return false
    seen.add(name)
  }
  return true
}

export function installNativeSockets(server, {target, base='/', cookie, log}) {
  if (installed.has(server)) throw Error('The native socket bridge is already installed')
  if (typeof target !== 'string' || target.length>2048) throw Error('Native sockets require a fixed loopback HTTP backend')
  let origin
  try { origin = new URL(target) } catch { throw Error('Native sockets require a fixed loopback HTTP backend') }
  if (origin.protocol !== 'http:' || !['127.0.0.1','localhost','[::1]'].includes(origin.hostname) || origin.username || origin.password || origin.pathname !== '/' || origin.search || origin.hash) throw Error('Native sockets require a fixed loopback HTTP backend')
  if (origin.hostname === 'localhost') origin.hostname = '127.0.0.1'
  if (typeof base !== 'string' || base.length>2048 || !base.startsWith('/') || !base.endsWith('/') || /[\\?#\s]/.test(base) || base.includes('//') || new URL(base,'http://brain.invalid').pathname !== base) throw Error('Native sockets require a canonical application base path')
  if (typeof cookie !== 'function' || typeof log !== 'function') throw Error('Native socket bridge callbacks are required')
  const path = (base === '/' ? '' : base.slice(0,-1)) + '/_brain/socket'
  const owned = new Set()
  let stopped = false, window = 0, reports = 0
  // Fixed fields only: never log target URLs, credentials, frames, request headers or exception messages.
  const report = (code,rid) => {
    const current = Math.floor(Date.now()/60000)
    if (window !== current) { window=current; reports=0 }
    if (reports++ >= 16) return
    try { log('warn','native socket unavailable',{code,rid,fix:'Check native socket activation, the trusted Origin, authentication and backend availability; reconnect after correcting the cause'}) } catch {}
  }
  const reject = (socket,status,rid,code) => {
    report(code,rid)
    if (socket.destroyed) return
    socket.setTimeout(LIMITS.close,()=>socket.destroy())
    socket.end(`HTTP/1.1 ${status} Socket unavailable\r\nConnection: close\r\nCache-Control: no-store\r\nX-Request-Id: ${rid}\r\nContent-Length: 0\r\n\r\n`,()=>socket.destroy())
  }
  const upgrade = (incoming,browser,head) => {
    const ownedPath=typeof incoming.url === 'string' && incoming.url.split(/[?#]/,1)[0] === path
    // Other trusted listeners own their endpoints. If this is the only listener, reject unknown upgrades promptly
    // instead of retaining a raw socket with no owner. Queries on our exact endpoint always remain ours to reject.
    if (!ownedPath && server.listenerCount('upgrade')>1) return
    if (browser.destroyed) return
    const rid = randomUUID()
    // A socket error can precede admission, including while a rejected response is flushing.
    browser.on('error',()=>{})
    if (stopped || server.draining) return reject(browser,503,rid,'NATIVE_SOCKET_DRAINING')
    if (incoming.method !== 'GET') return reject(browser,405,rid,'NATIVE_SOCKET_METHOD')
    if (incoming.url !== path) return reject(browser,400,rid,'NATIVE_SOCKET_PATH')
    const headerBytes = incoming.rawHeaders.reduce((bytes,value)=>bytes+Buffer.byteLength(value)+2,0)
    if (headerBytes>LIMITS.headers || !unique(incoming.rawHeaders,SINGLE)) return reject(browser,431,rid,'NATIVE_SOCKET_HEADERS')
    if (head.length>LIMITS.head || incoming.headers['content-length'] && incoming.headers['content-length'] !== '0' || incoming.headers['transfer-encoding']) return reject(browser,400,rid,'NATIVE_SOCKET_BODY')
    let site
    try { site=new URL(incoming.headers.origin) } catch { return reject(browser,403,rid,'NATIVE_SOCKET_ORIGIN') }
    if (!['http:','https:'].includes(site.protocol) || site.origin !== incoming.headers.origin || site.origin.length>2048) return reject(browser,403,rid,'NATIVE_SOCKET_ORIGIN')
    const key = incoming.headers['sec-websocket-key']
    if (incoming.headers.upgrade?.toLowerCase() !== 'websocket' || !connectionUpgrade(incoming.headers.connection) || incoming.headers['sec-websocket-version'] !== '13' || incoming.headers['sec-websocket-protocol'] !== PROTOCOL || typeof key !== 'string' || !/^[A-Za-z0-9+/]{22}==$/.test(key) || Buffer.from(key,'base64').toString('base64') !== key) return reject(browser,400,rid,'NATIVE_SOCKET_HANDSHAKE')
    let credentials
    try { credentials=cookie(incoming) } catch { return reject(browser,400,rid,'NATIVE_SOCKET_CREDENTIALS') }
    const authorization = incoming.headers.authorization
    if (typeof credentials !== 'string' || authorization !== undefined && typeof authorization !== 'string' || Buffer.byteLength(credentials)+Buffer.byteLength(authorization || '')>LIMITS.credentials) return reject(browser,431,rid,'NATIVE_SOCKET_CREDENTIALS')
    if (owned.size>=LIMITS.connections) return reject(browser,503,rid,'NATIVE_SOCKET_CAPACITY')
    browser.pause()
    let outgoing, upstream, handshake, lifetime, closing, bytes=0, done=false, upgraded=false
    const writes=new Set()
    const finish = (code,status) => {
      if (done) return
      done=true
      clearTimeout(handshake); clearTimeout(lifetime); clearTimeout(closing); owned.delete(finish)
      for (const timer of writes) clearTimeout(timer)
      writes.clear()
      browser.setTimeout(0)
      if (upstream) { upstream.setTimeout(0); browser.unpipe(upstream); upstream.unpipe(browser); upstream.destroy() }
      outgoing?.destroy()
      if (status && !upgraded) reject(browser,status,rid,code)
      else { if (code) report(code,rid); browser.destroy() }
    }
    const relay = (source,destination) => {
      let blocked
      destination.on('drain',()=>{if(blocked){clearTimeout(blocked);writes.delete(blocked);blocked=undefined}})
      source.on('data',chunk=>{
        bytes+=chunk.length
        if (bytes>LIMITS.bytes || destination.writableLength+chunk.length>LIMITS.queued) finish('NATIVE_SOCKET_WRITE_BUDGET')
      })
      source.once('end',()=>{
        if (done) return
        closing ??= setTimeout(()=>finish('NATIVE_SOCKET_HALF_CLOSE'),LIMITS.close)
        closing.unref?.()
        destination.once('finish',()=>finish())
      })
      // Native pipe honors write(false)/drain. highWaterMark alone is a threshold, so the explicit queued cap above
      // also stops a future/custom stream chunk that exceeds the admitted write budget.
      source.pipe(destination)
      // Observe after pipe's synchronous write so an unresponsive destination cannot retain its bounded queue
      // until the much longer idle/lifetime limits. There is at most one stalled-write timer per direction.
      source.on('data',()=>{
        if (!done && destination.writableNeedDrain && !blocked) {
          blocked=setTimeout(()=>finish('NATIVE_SOCKET_WRITE_TIMEOUT'),LIMITS.write)
          blocked.unref?.();writes.add(blocked)
        }
      })
    }
    owned.add(finish)
    browser.once('error',()=>finish('NATIVE_SOCKET_CLIENT_ERROR'))
    browser.once('close',()=>finish())
    browser.setTimeout(LIMITS.idle,()=>finish('NATIVE_SOCKET_IDLE'))
    handshake=setTimeout(()=>finish('NATIVE_SOCKET_HANDSHAKE_TIMEOUT',504),LIMITS.handshake)
    handshake.unref?.()
    try {
      outgoing=request(new URL('/_brain/socket',origin),{
        method:'GET',agent:false,maxHeaderSize:8192,
        headers:{host:origin.host,origin:site.origin,connection:'Upgrade',upgrade:'websocket','sec-websocket-version':'13','sec-websocket-key':key,'sec-websocket-protocol':PROTOCOL,'x-request-id':rid,...(credentials ? {cookie:credentials} : {}),...(authorization !== undefined ? {authorization} : {})},
      })
      outgoing.once('error',()=>finish('NATIVE_SOCKET_UPSTREAM_UNAVAILABLE',502))
      outgoing.once('response',response=>{
        response.destroy()
        finish('NATIVE_SOCKET_UPSTREAM_REJECTED',[400,401,403,404,429,503].includes(response.statusCode) ? response.statusCode : 502)
      })
      outgoing.once('upgrade',(response,socket,buffered)=>{
        if (done) { socket.destroy(); return }
        upstream=socket
        socket.on('error',()=>finish('NATIVE_SOCKET_UPSTREAM_ERROR'))
        socket.once('close',()=>finish())
        const expected=createHash('sha1').update(key+'258EAFA5-E914-47DA-95CA-C5AB0DC85B11').digest('base64')
        if (response.statusCode !== 101 || !unique(response.rawHeaders,new Set(['upgrade','sec-websocket-accept','sec-websocket-protocol'])) || response.headers.upgrade?.toLowerCase() !== 'websocket' || !connectionUpgrade(response.headers.connection) || response.headers['sec-websocket-accept'] !== expected || response.headers['sec-websocket-protocol'] !== PROTOCOL || response.headers['sec-websocket-extensions'] || buffered.length>LIMITS.upstreamHead) return finish('NATIVE_SOCKET_BAD_UPSTREAM_HANDSHAKE',502)
        clearTimeout(handshake)
        lifetime=setTimeout(()=>finish('NATIVE_SOCKET_LIFETIME'),LIMITS.lifetime)
        lifetime.unref?.()
        socket.setTimeout(LIMITS.idle,()=>finish('NATIVE_SOCKET_IDLE'))
        upgraded=true
        browser.write(`HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: ${expected}\r\nSec-WebSocket-Protocol: ${PROTOCOL}\r\nCache-Control: no-store\r\nX-Request-Id: ${rid}\r\n\r\n`)
        bytes=buffered.length+head.length
        if (buffered.length) browser.write(buffered)
        if (head.length) socket.write(head)
        relay(browser,socket); relay(socket,browser); browser.resume()
      })
      outgoing.end()
    } catch { finish('NATIVE_SOCKET_UPSTREAM_UNAVAILABLE',502) }
  }
  const closeOwned = () => { for (const finish of [...owned]) finish() }
  const stop = () => { if (stopped) return; stopped=true; closeOwned() }
  installed.add(server)
  server.on('upgrade',upgrade)
  server.once('close',()=>{stop();server.removeListener('upgrade',upgrade)})
  const originalClose=server.close, originalAll=server.closeAllConnections
  server.close=function(...args){stop();return originalClose.apply(this,args)}
  if (originalAll) server.closeAllConnections=function(...args){closeOwned();return originalAll.apply(this,args)}
  return stop
}
