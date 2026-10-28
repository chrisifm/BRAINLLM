// BRAINLLM lazy components (INTERNALS §22): `<Chart lazy />` (visible), `lazy="idle"`, `lazy="interaction"`.
// The codegen renders a placeholder element in the template (its fallback content, or an empty sized box) and emits
// `lazy(el, () => import('/_b/<h>'), mode, $props)`. When the trigger fires, the chunk is imported and the component
// is built under the owner that called lazy (context, disposal and the error catcher work as if it were inline);
// its nodes replace the placeholder. If the owner is disposed first, the trigger is cancelled; afterwards, the
// component is disposed with it. An import failure is reported (reportError → brain-err) and the placeholder stays.
// `el.$l` is the load promise (prefetch: `el.$l || el.$go()`).
import { _rt, fx } from './brain.js'

const G = globalThis
const I = _rt()
const ACT = ['pointerover', 'pointerdown', 'focusin', 'touchstart', 'click', 'keydown']
// A chunk deleted by a newer deploy: reload the page once per build (sessionStorage guard), never in a loop (§35.7).
function reloadOnce() {
  try {
    const b = document.querySelector('meta[name=brain-build]'), k = 'brain:reload:' + (b ? b.getAttribute('content') : '')
    if (sessionStorage.getItem(k)) return false
    sessionStorage.setItem(k, 1)
    location.reload()
    return true
  } catch {
    return false
  }
}

export function lazy(el, imp, mode, props = {}) {
  const owner = I.owner()
  let dead = false, stop = () => {}, clicked = null, built
  const go = () => {
    if (dead) return
    stop()
    return (el.$l ||= Promise.resolve()
      .then(imp)
      .then(m => {
        if (dead) return
        const C = (m && m.default) || m
        built = I.root(() => I.nodes(C(props)), owner) // adopted by owner: disposed with it
        el.replaceWith(...built)
        // the click that triggered the load landed on the placeholder: replay it on the real component
        const t = clicked && built.find(n => n.nodeType === 1)
        if (t) t.dispatchEvent(new clicked.constructor('click', clicked))
        return built
      })
      .catch(e => {
        el.$l = null
        if (!reloadOnce()) G.reportError ? reportError(e) : queueMicrotask(() => { throw e })
      }))
  }
  el.$go = go
  if (mode === 'idle') {
    if (G.requestIdleCallback) {
      const h = requestIdleCallback(go, { timeout: 2000 })
      stop = () => cancelIdleCallback(h)
    } else {
      const h = setTimeout(go, 1)
      stop = () => clearTimeout(h)
    }
  } else if (mode === 'interaction') {
    const f = e => {
      if (e.type === 'click') (clicked = e), e.preventDefault(), e.stopPropagation()
      go()
    }
    for (const t of ACT) el.addEventListener(t, f, t === 'click')
    stop = () => {
      for (const t of ACT) el.removeEventListener(t, f, t === 'click')
    }
  } else if (G.IntersectionObserver) {
    const io = new IntersectionObserver(es => es.some(e => e.isIntersecting) && go(), { rootMargin: '200px' })
    io.observe(el)
    stop = () => io.disconnect()
  } else go()
  if (owner)
    fx(() => () => {
      dead = true
      stop()
      if (built) I.rm(built)
    })
  return el
}
