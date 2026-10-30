// Explicit foreign-DOM ownership. The adapter, SDK and any React runtime belong to the app.
import { _rt, fx, untrack } from './brain.js'

const I = _rt()

// Call during component setup. Resolve the bound element only after insertion.
// mount must synchronously return a cleanup function or { update?, destroy }.
export function interop(element, adapter, props = {}, options = {}) {
  const owner = I.owner()
  if (!owner) throw Error('interop: call during component setup')
  let alive = true, instance = null, pending = props, ready = false
  const report = (error, phase) => {
    if (typeof options.onError === 'function') options.onError(error, phase)
    else I.oops(error, owner, 'interop:' + phase)
  }
  const destroy = () => {
    if (!alive) return
    alive = false
    const old = instance
    instance = null
    if (old) try { old.destroy() } catch (error) { report(error, 'destroy') }
  }
  const controller = {
    update(next) {
      if (!alive) throw Error('interop: boundary has been destroyed')
      pending = next
      if (!ready) return
      if (!instance || typeof instance.update !== 'function') throw Error('interop: adapter does not support updates')
      try { untrack(() => instance.update(next)) } catch (error) { report(error, 'update') }
    },
    destroy,
  }
  // No SDK execution during SSR or prerender. Disposal still invalidates this controller.
  fx(() => destroy)
  if (globalThis.__brainSSR || globalThis.__brainRoute) return controller
  queueMicrotask(() => {
    if (!alive) return
    try {
      const el = typeof element === 'function' ? untrack(element) : element
      if (!el || el.nodeType !== 1) throw Error('interop: expected a bound DOM element')
      if (!adapter || typeof adapter.mount !== 'function') throw Error('interop: adapter.mount is required')
      const result = untrack(() => adapter.mount(el, pending))
      if (typeof result === 'function') instance = { destroy: result }
      else if (result && typeof result.destroy === 'function' && typeof result.then !== 'function') instance = result
      else throw Error('interop: mount must return cleanup or { update, destroy }; asynchronous mounts are unsupported')
      ready = true
    } catch (error) { report(error, 'mount') }
  })
  return controller
}
