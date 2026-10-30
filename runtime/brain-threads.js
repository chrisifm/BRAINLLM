// Thin adapters over platform APIs. Experimental worklets are capabilities, never assumed browser support.
const G = globalThis
const fail = fix => Object.assign(new Error(fix), {code:'B0917',fix})
function local(url) {
  if (!G.location) throw fail('Thread module loading requires a browser context')
  const value = new URL(url,G.location.href)
  if (value.origin !== G.location.origin || value.username || value.password || !/^https?:$/.test(value.protocol)) throw fail('Use a same-origin versioned module URL without credentials')
  return value.href
}
export function threadCapabilities() {
  return { dedicated:typeof G.Worker==='function', shared:typeof G.SharedWorker==='function',
    service:!!G.navigator?.serviceWorker, messageChannel:typeof G.MessageChannel==='function',
    transferableArrayBuffer:typeof G.ArrayBuffer==='function', sharedMemory:!!G.crossOriginIsolated&&typeof G.SharedArrayBuffer==='function'&&!!G.Atomics,
    offscreenCanvas:typeof G.OffscreenCanvas==='function', wasm:!!G.WebAssembly,
    audioWorklet:typeof G.AudioWorkletNode==='function', paintWorklet:!!G.CSS?.paintWorklet?.addModule,
    layoutWorklet:!!G.CSS?.layoutWorklet?.addModule, animationWorklet:!!G.CSS?.animationWorklet?.addModule,
    indexedDB:!!G.indexedDB, opfs:!!G.navigator?.storage?.getDirectory,
    backgroundSync:!!G.ServiceWorkerRegistration?.prototype&&'sync' in G.ServiceWorkerRegistration.prototype,
    push:typeof G.PushManager==='function' }
}
export async function loadWorklet(kind,url,options={}) {
  const target = kind==='audio' ? options.context?.audioWorklet : kind==='paint' ? G.CSS?.paintWorklet : kind==='layout' ? G.CSS?.layoutWorklet : kind==='animation' ? G.CSS?.animationWorklet : null
  if (!target?.addModule) {
    if (typeof options.fallback==='function') return options.fallback()
    throw fail('Requested worklet is unavailable; supply an explicit rendering/audio fallback')
  }
  // addModule has no portable cancellation. Bound outstanding requests and avoid spawning an AudioContext implicitly.
  if (loading>=4) throw fail('Too many worklet loads; await existing modules')
  loading++
  try { await target.addModule(local(url),{credentials:'omit'}); return {available:true,kind} }
  catch { throw fail('Worklet module could not load; check its same-origin asset URL and production CSP') }
  finally { loading-- }
}
let loading=0
export function messageChannel() {
  if (!G.MessageChannel) throw fail('MessageChannel is unavailable in this context')
  const c=new G.MessageChannel()
  return {port1:c.port1,port2:c.port2,close(){c.port1.close();c.port2.close()}}
}
export function offscreen(canvas) {
  if (!canvas?.transferControlToOffscreen) throw fail('OffscreenCanvas transfer is unavailable; keep the canvas on the main thread')
  try { return canvas.transferControlToOffscreen() }
  catch { throw fail('Canvas was already initialized or transferred; transfer it before acquiring a rendering context') }
}
export function sharedMemory(bytes) {
  if (!threadCapabilities().sharedMemory) throw fail('Shared memory needs cross-origin isolation and browser support; use transferable ArrayBuffers otherwise')
  if (!Number.isSafeInteger(bytes)||bytes<1||bytes>8388608) throw fail('Shared memory buffers must contain between 1 byte and 8 MiB')
  return new G.SharedArrayBuffer(bytes)
}
