// BRAINLLM Web Components (SPEC §6.2): define(tag, Comp, css) exports a component as a Custom Element.
// Imports ./brain.js relatively. Props = attributes from Comp.p (string) or JS properties (any value).
import { sig, mount, _ev } from './brain.js'

const sheets = new Map()
// Events that do not cross the shadow root (composed: false): also delegated on each shadow root.
const LOCAL = ['change', 'submit', 'reset', 'select', 'toggle', 'invalid']

export function define(tag, C, css = '') {
  const P = C.p || []
  class E extends HTMLElement {
    static observedAttributes = P
    static formAssociated = !!C.form
    constructor() {
      super()
      this.s = {}
      for (const k of P) this.s[k] = sig(this.getAttribute(k))
      this.i = C.form ? this.attachInternals() : null
      this.x = null
    }
    attributeChangedCallback(k, _, v) {
      this.s[k].v = v
    }
    connectedCallback() {
      if (this.x) return
      const r = this.shadowRoot || this.attachShadow({ mode: 'open' }) // reuses Declarative Shadow DOM
      let sh = sheets.get(tag)
      if (!sh) sheets.set(tag, (sh = new CSSStyleSheet())), sh.replaceSync(css)
      r.adoptedStyleSheets = [sh]
      if (!r.$w) {
        r.$w = 1
        for (const t of LOCAL) r.addEventListener(t, _ev)
      }
      r.textContent = '' // v0: DSD content is re-rendered, same as hydrate
      const el = this
      const $p = new Proxy(
        {},
        {
          get: (_, k) =>
            k in el.s
              ? el.s[k].v
              : k === '$internals'
                ? el.i
                : typeof k === 'string' && k.startsWith('on:')
                  ? d => el.dispatchEvent(new CustomEvent(k.slice(3), { detail: d, bubbles: true, composed: true }))
                  : undefined,
        }
      )
      this.x = mount(C, r, $p)
    }
    disconnectedCallback() {
      // in a microtask: moving the element (disconnect and reconnect) does not destroy it
      queueMicrotask(() => {
        if (!this.isConnected && this.x) this.x(), (this.x = null)
      })
    }
  }
  for (const k of P)
    Object.defineProperty(E.prototype, k, {
      get() {
        return this.s[k].v
      },
      set(v) {
        this.s[k].v = v
      },
    })
  customElements.define(tag, E)
  return E
}
