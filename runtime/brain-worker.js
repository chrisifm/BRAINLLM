// BRAINLLM Web Worker bridge (INTERNALS §35.5). The compiler turns `*.worker.jsbrain` into a worker chunk that ends in
// `serve({ parseCsv, default: $default })`, and every importer into `const w = worker(new URL('…', import.meta.url))`
// plus `const parseCsv = (...a) => w.call('parseCsv', a)`. Messages use structured clone: { id, name, args } →
// { id, ok } | { id, err: { message, name, stack } }. The worker starts on the first call; a remote error is rethrown
// in the page as an Error with the same message and name (its stack in `remoteStack`).
const TRANSFER = Symbol('brain-transfer')
export function transfer(value, list) {
  if (!Array.isArray(list) || list.length > 32 || new Set(list).size !== list.length) throw new TypeError('transfer: provide at most 32 distinct transferable objects')
  return { [TRANSFER]:true, value, list }
}

// Worker side. `name` may be a path into the map ('default.mean'); the result may be a promise.
export function serve(map, scope = globalThis) {
  if ('onconnect' in scope) {
    scope.onconnect = event => {
      for (const port of event.ports || []) { serve(map,port); port.start() }
    }
    return
  }
  scope.onmessage = async e => {
    const { id, name, args } = e.data || {}
    try {
      if (!Number.isSafeInteger(id) || id < 1 || typeof name !== 'string' || name.length > 256 || !Array.isArray(args)) throw new TypeError('worker: invalid RPC envelope')
      let f = map, self
      for (const k of name.split('.')) {
        if (!k || /^(?:__proto__|prototype|constructor)$/.test(k)) throw new TypeError('worker: unsafe export path')
        self = f
        f = f == null ? undefined : Object.getOwnPropertyDescriptor(f, k)?.value
      }
      if (typeof f !== 'function') throw new TypeError(`worker: \`${name}\` is not an exported function`)
      const ok = await f.apply(self, args || [])
      if (ok?.[TRANSFER]) scope.postMessage({id,ok:ok.value},ok.list)
      else scope.postMessage({ id, ok }) // throws DataCloneError for a result that cannot be cloned → reported below
    } catch (x) {
      scope.postMessage({ id, err: { message: (x && x.message) || String(x), name: (x && x.name) || 'Error', stack: x && x.stack } })
    }
  }
}

// Page side.
export function worker(url, options = {}) {
  let w = null, n = 0, inFlight = 0, idle = null
  const wait = new Map()
  const timeout = Math.max(10, Math.min(60000, Number(options.timeout) || 15000))
  const capacity = Math.max(1, Math.min(256, Number(options.capacity) || 64))
  const maxBytes = Math.max(1024, Math.min(67108864, Number(options.maxBytes) || 4194304))
  const maxInFlight = Math.max(maxBytes, Math.min(67108864, Number(options.maxInFlightBytes) || 8388608))
  function size(value) {
    const seen = new Set(), stack = [[value, 0]]
    let bytes = 0, nodes = 0
    const push = (value, depth) => {
      if (nodes + stack.length >= 4096) throw Object.assign(new Error('worker: payload structure exceeds its budget; transfer a bounded ArrayBuffer instead'), {code:'B0911'})
      stack.push([value, depth])
    }
    while (stack.length) {
      const [v, depth] = stack.pop()
      if (++nodes > 4096 || depth > 24) throw Object.assign(new Error('worker: payload structure exceeds its budget; transfer a bounded ArrayBuffer instead'), { code: 'B0911' })
      if (typeof v === 'string') bytes += v.length * 2
      else if (v && typeof v === 'object' && !seen.has(v)) {
        seen.add(v)
        if (ArrayBuffer.isView(v)) push(v.buffer, depth + 1)
        else if (v instanceof ArrayBuffer || (typeof SharedArrayBuffer === 'function' && v instanceof SharedArrayBuffer)) bytes += v.byteLength
        else if (v instanceof Map) for (const [a,b] of v) { push(a,depth+1); push(b,depth+1) }
        else if (v instanceof Set) for (const a of v) push(a,depth+1)
        else for (const k in v) {
          const d = Object.getOwnPropertyDescriptor(v,k)
          if (!d) continue
          if (!('value' in d)) throw new DOMException('Worker payload accessors are not supported', 'DataCloneError')
          bytes += k.length * 2; push(d.value,depth+1)
        }
      } else bytes += 8
      if (bytes > maxBytes) throw Object.assign(new Error('worker: payload byte budget exceeded; reduce the batch or configure a bounded larger budget'), { code: 'B0911' })
    }
    return bytes
  }
  const failAll = err => {
    clearTimeout(idle); idle = null
    const old = w
    w = null
    if (old) old.terminate()
    for (const [, p] of wait) { p.clean(); p.fail(err) }
    wait.clear()
  }
  const start = () => {
    if (globalThis.location) {
      const resolved = new URL(url,location.href)
      if (resolved.origin !== location.origin || resolved.username || resolved.password) throw new TypeError('worker: use a same-origin compiled module URL')
    }
    if (options.shared) {
      if (!globalThis.SharedWorker) throw new Error('SharedWorker is unavailable; use a dedicated worker or explicit tab coordination fallback')
      const shared = new SharedWorker(url, {type:'module',name:options.name}), port = shared.port
      const bridge = { postMessage:(m,t) => port.postMessage(m,t), terminate:() => port.close() }
      port.onmessage = e => bridge.onmessage?.(e)
      port.onmessageerror = e => bridge.onmessageerror?.(e)
      shared.onerror = e => bridge.onerror?.(e)
      port.start()
      w = bridge
    } else w = new Worker(url, { type: 'module' })
    const instance = w
    w.onmessage = e => {
      if (w !== instance) return
      const { id, ok, err } = e.data || {}
      const p = wait.get(id)
      if (!p) return
      wait.delete(id)
      p.clean()
      if (!wait.size) {
        idle = setTimeout(() => { if (w === instance && !wait.size) failAll(new Error('worker idle')) }, 30000)
        idle.unref?.()
      }
      if (err) p.fail(Object.assign(new Error(err.message), { name: err.name, remoteStack: err.stack }))
      else p.ok(ok)
    }
    w.onerror = e => {
      if (w !== instance) return
      if (e && e.preventDefault) e.preventDefault()
      failAll(new Error('worker failed: ' + ((e && e.message) || url)))
    }
    w.onmessageerror = () => { if (w === instance) failAll(new Error('worker: response could not be decoded')) }
  }
  return {
    call(name, args = [], o = {}) {
      if (o.signal?.aborted) return Promise.reject(Object.assign(new Error('worker call aborted'), { name: 'AbortError', kind: 'aborted' }))
      if (wait.size >= capacity) return Promise.reject(Object.assign(new Error('worker: pending call capacity exceeded; await existing work'), { code: 'B0911' }))
      if (typeof name !== 'string' || name.length > 256 || !Array.isArray(args)) return Promise.reject(new TypeError('worker: invalid call'))
      let bytes
      try { bytes = size(args) } catch (e) { return Promise.reject(e) }
      if (inFlight + bytes > maxInFlight) return Promise.reject(Object.assign(new Error('worker: in-flight byte capacity exceeded; await pending work'), { code:'B0911' }))
      try { if (!w) start() } catch (e) { return Promise.reject(e) }
      clearTimeout(idle); idle = null
      const id = ++n
      return new Promise((ok, fail) => {
        const abort = () => failAll(Object.assign(new Error('worker call aborted; shared pending calls cancelled'), { name: 'AbortError', kind: 'aborted' }))
        const timer = setTimeout(() => failAll(Object.assign(new Error('worker: deadline exceeded; move bounded work into smaller batches'), { code: 'B0910' })), timeout)
        inFlight += bytes
        const clean = () => { inFlight -= bytes; clearTimeout(timer); o.signal?.removeEventListener('abort', abort) }
        wait.set(id, { ok, fail, clean })
        o.signal?.addEventListener('abort', abort, { once: true })
        try {
          w.postMessage({ id, name, args }, o.transfer || [])
        } catch (x) {
          wait.delete(id)
          clean()
          fail(x) // DataCloneError: an argument cannot cross to the worker (functions, DOM nodes…)
        }
      })
    },
    terminate() {
      if (w) w.terminate()
      failAll(new Error('worker terminated'))
    },
  }
}

export function sharedWorker(url, options = {}) {
  if (typeof options.name !== 'string' || !/^[a-zA-Z0-9][a-zA-Z0-9_.-]{0,79}$/.test(options.name)) throw new TypeError('sharedWorker: provide a stable public name of at most 80 characters')
  return worker(url, { ...options, shared:true })
}
