// BRAINLLM islands (SPEC §6.1). A prerendered island is its root element (`data-b`) followed by
// `<script type="b/state">{"c": chunk, "x": export, "p": props}</script>`. hydrate() listens on the document:
// intent (pointerdown, focusin, keydown) only preloads the chunk and lets the native event act on the prerendered DOM,
// so typing and the press that starts a click are never lost. The first real event (click, submit, input, change)
// mounts the island in place, carrying over what the user already did there (values, checked, focus and caret), and
// replays that event once on the live node.
// No import of the core: brain.js loads this module with import() and passes { mount, _rt } (a static import back
// would make a cycle that content-hashed chunks cannot name).
import { adopter } from './brain-hydrate.js' // no imports of its own: no hash cycle

let mount, I
const load = (el, s) => el.$m || (el.$m = import(new URL(JSON.parse(s.textContent).c, document.baseURI).href).catch(_cf))
const at = (n, path) => {
  for (const i of path) n = n && n.childNodes[i]
  return n
}
const pathOf = (n, root, a = []) => {
  for (; n && n !== root; n = n.parentNode) a.unshift([].indexOf.call(n.parentNode.childNodes, n))
  return n ? a : null
}
async function resume(el, s) {
  const { c, x = 'default', p = {} } = JSON.parse(s.textContent)
  const m = await load(el, s)
  // #285: a server render stamped for adoption (data-hk) resumes on its own nodes: nothing is replaced, so internal
  // scroll, a playing video, focus and animations survive. Any mismatch falls back to rendering fresh below.
  if (el.getAttribute('data-hk') && I.adopt) {
    let d = null
    try {
      I.adopt(adopter(el))
      const ns = I.root(z => ((d = z), I.nodes(m[x](p))), null)
      if (ns[0] !== el) throw Error('adopt: another root')
      I.adopt(null)
      I.dev(null, c)
      el.$d = d // the router disposes islands it swaps out
      s.remove()
      return el
    } catch {
      I.adopt(null)
      if (d) d()
    }
  }
  const f = document.createDocumentFragment()
  const d = mount(m[x], f, p)
  I.dev(null, c) // dev: hydrate event for brain-inspect
  // dev: the prerendered HTML and the first client render must match (B0214, §31): else the page flashes on resume
  if (globalThis.__brainDev && f.firstChild) {
    const norm = h => h.replace(/<!--[\s\S]*?-->/g, '').replace(/>\s+</g, '><').replace(/\s+(data-b|nonce)="[^"]*"/g, '').trim()
    const a0 = norm(el.outerHTML), b0 = norm(f.firstChild.outerHTML)
    if (a0 !== b0) {
      let i = 0
      while (a0[i] === b0[i]) i++
      console.warn(Object.assign(Error(`B0214: ${c} renders differently on the client than in the prerendered HTML near: server "${a0.slice(Math.max(0, i - 20), i + 40)}" · client "${b0.slice(Math.max(0, i - 20), i + 40)}"`), { code: 'B0214' }))
    }
  }
  const n = f.firstChild, q = 'input,select,textarea', a = el.querySelectorAll(q), b = n.querySelectorAll(q)
  for (let i = 0; i < a.length && i < b.length; i++)
    /^(checkbox|radio)$/.test(a[i].type) ? (b[i].checked = a[i].checked) : a[i].value !== b[i].value && (b[i].value = a[i].value)
  const act = document.activeElement, fp = act && el.contains(act) && pathOf(act, el)
  n.$d = d // the router disposes islands it swaps out
  el.replaceWith(f)
  s.remove()
  const g = fp && at(n, fp)
  if (g && g.focus) {
    g.focus({ preventScroll: true })
    try {
      g.setSelectionRange(act.selectionStart, act.selectionEnd, act.selectionDirection)
    } catch {}
  }
  return n
}
// A chunk that cannot be imported (deleted by a newer deploy) reloads the page once per build, never in a loop.
export function _cf(e) {
  const b = document.querySelector('meta[name=brain-build]'), k = 'brain:reload:' + (b ? b.getAttribute('content') : '')
  try {
    if (!sessionStorage.getItem(k)) sessionStorage.setItem(k, 1), location.reload()
  } catch {}
  throw e
}
export function hydrate(r = document, rt) {
  ;({ mount } = rt)
  I = rt._rt()
  if (r.$h) return // idempotent: the router re-runs page scripts on every navigation
  r.$h = 1
  for (const t of ['click', 'submit', 'input', 'change', 'pointerdown', 'focusin', 'keydown'])
    r.addEventListener(
      t,
      e => {
        const path = []
        let el = e.target, s
        for (; el && el.nodeType === 1; el = el.parentNode) {
          s = el.nextSibling
          if (s && s.type === 'b/state' && el.hasAttribute('data-b')) break
          path.unshift([].indexOf.call(el.parentNode.childNodes, el))
        }
        if (!el || el.nodeType !== 1) return
        if ('pointerdown focusin keydown'.includes(t)) return void load(el, s) // intent: preload only, the native event goes on
        e.preventDefault()
        e.stopPropagation()
        ;(el.$b || (el.$b = resume(el, s))).then(
          n => (n = at(n, path)) && n.dispatchEvent(new e.constructor(t, e)),
          x => I.oops(x, null, 'island') // reported once to the catcher, never an unhandled rejection
        )
      },
      true
    )
}
