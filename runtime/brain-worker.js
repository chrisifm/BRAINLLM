// BRAINLLM Web Worker bridge (INTERNALS §35.5). The compiler turns `*.worker.jsbrain` into a worker chunk that ends in
// `serve({ parseCsv, default: $default })`, and every importer into `const w = worker(new URL('…', import.meta.url))`
// plus `const parseCsv = (...a) => w.call('parseCsv', a)`. Messages use structured clone: { id, name, args } →
// { id, ok } | { id, err: { message, name, stack } }. The worker starts on the first call; a remote error is rethrown
// in the page as an Error with the same message and name (its stack in `remoteStack`).

// Worker side. `name` may be a path into the map ('default.mean'); the result may be a promise.
export function serve(map, scope = globalThis) {
  scope.onmessage = async e => {
    const { id, name, args } = e.data || {}
    try {
      let f = map, self
      for (const k of String(name).split('.')) (self = f), (f = f == null ? undefined : f[k])
      if (typeof f !== 'function') throw new TypeError(`worker: \`${name}\` is not an exported function`)
      const ok = await f.apply(self, args || [])
      scope.postMessage({ id, ok }) // throws DataCloneError for a result that cannot be cloned → reported below
    } catch (x) {
      scope.postMessage({ id, err: { message: (x && x.message) || String(x), name: (x && x.name) || 'Error', stack: x && x.stack } })
    }
  }
}

// Page side.
export function worker(url) {
  let w = null, n = 0
  const wait = new Map()
  const failAll = err => {
    for (const [, p] of wait) p[1](err)
    wait.clear()
    w = null // the next call starts a fresh worker
  }
  const start = () => {
    w = new Worker(url, { type: 'module' })
    w.onmessage = e => {
      const { id, ok, err } = e.data || {}
      const p = wait.get(id)
      if (!p) return
      wait.delete(id)
      if (err) p[1](Object.assign(new Error(err.message), { name: err.name, remoteStack: err.stack }))
      else p[0](ok)
    }
    w.onerror = e => {
      if (e && e.preventDefault) e.preventDefault()
      failAll(new Error('worker failed: ' + ((e && e.message) || url)))
    }
  }
  return {
    call(name, args) {
      if (!w) start()
      const id = ++n
      return new Promise((ok, fail) => {
        wait.set(id, [ok, fail])
        try {
          w.postMessage({ id, name, args })
        } catch (x) {
          wait.delete(id)
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
