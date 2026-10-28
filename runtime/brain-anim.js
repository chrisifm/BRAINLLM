// BRAINLLM transitions (INTERNALS §41.6): `transition:fade|slide|scale` on an element inside {#if}/{#each} compiles to
// `transition(el, 'fade', { ms, in, out })` right after the element is created. The enter animation plays once the
// element is in the document; the exit one is left on the element as `el.$out` (→ Promise), which the core's removal
// (when/list) waits for before taking the node out. Web Animations API; nothing with prefers-reduced-motion, without
// `el.animate` or while the server renders.
const G = globalThis
const K = {
  fade: [{ opacity: 0 }, { opacity: 1 }],
  scale: [{ opacity: 0, transform: 'scale(.95)' }, { opacity: 1, transform: 'none' }],
  slide: [{ opacity: 0, transform: 'translateY(-8px)' }, { opacity: 1, transform: 'none' }],
}
const still = () => !!(G.matchMedia && G.matchMedia('(prefers-reduced-motion: reduce)').matches)

export function transition(el, kind = 'fade', o = {}) {
  const k = K[kind]
  if (!k) throw Error(`transition: unknown kind "${kind}" (fade, slide, scale)`)
  if (!el.animate || G.__brainSSR || G.__brainRoute || still()) return
  const opts = { duration: o.ms ?? 200, easing: 'cubic-bezier(.22,1,.36,1)' }
  if (o.in !== false) queueMicrotask(() => el.isConnected && el.animate(k, opts))
  if (o.out !== false) el.$out = () => el.animate([...k].reverse(), { ...opts, fill: 'forwards' }).finished.catch(() => {})
}
