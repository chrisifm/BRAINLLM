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

// The observer is built in its own scope and reaches `go` through a box emptied on stop. A callback closing over
// lazy()'s scope kept the owner, the props and the built component alive after unmount and disconnect() (measured:
// parent and child survived forced GCs for the whole 10 s `--heap` grace, B0601; Chrome still holds the observer).
function visible(el, go) {
  const box = { go }
  const io = new IntersectionObserver(es => es.some(e => e.isIntersecting) && box.go?.(), { rootMargin: '200px' })
  io.observe(el)
  return () => { box.go = null; io.disconnect() }
}

export function lazy(el, imp, mode, props = {}) {
  const owner = I.owner()
  let dead = false, stop = () => {}, retry = () => {}, clicked = null, built
  const go = () => {
    if (dead) return
    stop()
    el.setAttribute('aria-busy', 'true')
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
        clicked = null // a retry must not replay a click from before the failure
        if (dead) return
        el.setAttribute('aria-busy', 'false')
        retry()
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
      if (e.type === 'keydown') {
        if (e.key !== 'Enter' && e.key !== ' ') return
        e.preventDefault()
        clicked = new Event('click', { bubbles: true, cancelable: true }) // the key press is what the user asked for
      }
      if (e.type === 'click') (clicked = e), e.preventDefault(), e.stopPropagation()
      go()
    }
    retry = () => { for (const t of ACT) el.addEventListener(t, f, t === 'click') }
    retry()
    stop = () => {
      for (const t of ACT) el.removeEventListener(t, f, t === 'click')
    }
  } else if (G.document && document.visibilityState === 'hidden') {
    go() // a hidden tab runs no observer: without this the content is missing until the tab is shown
  } else if (G.IntersectionObserver) stop = visible(el, go)
  else go()
  if (owner)
    fx(() => () => {
      dead = true
      stop()
      if (built) I.rm(built)
    })
  return el
}
