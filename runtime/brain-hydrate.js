// BRAINLLM hydration by adoption (#285, with COMPILER #143). An island resumes on the server's own nodes instead of
// replacing them, so internal scroll, a playing <video>, focus and CSS animations survive. No imports: the core and
// brain-island hand it what it needs (a static import of the core would make a hash cycle).
// How: while the server renders (stamp), every template clone gets `data-hk="<n>"` in render order and every dynamic
// region is wrapped as <!--[--> … <!--]--> (its anchor comment becomes `]`). While hydrating (adopter), the same render
// order claims the stamped node instead of cloning, and each region reuses what the server put there.
// Codegen (COMPILER) navigates with fc/nx instead of .firstChild/.nextSibling: on a fresh clone they are the same, on
// adopted server HTML they step over a whole region to its anchor.

// first child / next sibling, stepping over a server-rendered dynamic region to its `]` anchor
const sk = (y, d = 0) => {
  for (; y; y = y.nextSibling)
    if (y.data == '[') d++
    else if (y.data == ']' ? !--d : !d) return y
}
export const fc = e => sk(e.firstChild)
export const nx = n => sk(n.nextSibling)

// Server render (server.mjs, the build's prerender): `const end = stamp(_rt())` before mounting, `end()` after.
export function stamp(I, doc = globalThis.document) {
  let c = 0
  I.adopt({
    t(n) {
      const x = n.cloneNode(true)
      if (x.nodeType === 1) x.setAttribute('data-hk', String(++c))
      return x
    },
    p(end, a) {
      const p = end.parentNode
      if (!end.$m) (end.$m = 1), (end.data = ']'), p.insertBefore(doc.createComment('['), end)
      for (const n of a) p.insertBefore(n, end)
      return a
    },
  })
  return () => I.adopt(null)
}

// Hydration of the server's subtree `root` (brain-island): claims stamped nodes in render order; throws on any
// mismatch so the caller falls back to rendering fresh (a fragment template, another tag, a missing stamp).
export function adopter(root) {
  const els = [...root.querySelectorAll('[data-hk]')]
  if (root.getAttribute('data-hk')) els.unshift(root)
  const by = new Map(els.map(e => [+e.getAttribute('data-hk'), e]))
  if (!by.size) throw Error('adopt: nothing stamped')
  const base = Math.min(...by.keys())
  let c = 0
  return {
    t(n) {
      const e = by.get(base + c++)
      if (!e || n.nodeType !== 1 || e.tagName !== n.tagName) throw Error('adopt: the server rendered something else')
      return e
    },
    p(end, a) {
      // the region the server rendered before this anchor: from its matching `[` to `]`
      const ex = []
      if (end.data == ']')
        for (let y = end.previousSibling, d = 0; y; y = y.previousSibling) {
          if (y.data == ']') d++
          else if (y.data == '[' && !d--) break
          ex.push(y)
        }
      ex.reverse()
      // texts reuse the server's text node; adopted elements are already there; anything else goes in
      const out = a.map((n, i) => (n.nodeType === 3 && ex[i] && ex[i].nodeType === 3 ? (ex[i].data !== n.data && (ex[i].data = n.data), ex[i]) : n))
      const keep = new Set(out)
      for (const y of ex) if (!keep.has(y)) y.remove()
      const p = end.parentNode
      for (const n of out) if (n.parentNode !== p) p.insertBefore(n, end) // never move a node already in place
      return out
    },
  }
}
