// Counter exactly as the codegen would emit it (SPEC §6), used by the hydration test.
import { sig, tpl, insert, on } from '../rt.js'
const T = tpl('<button>n=<!><span>+</span></button>')
export function Counter($p) {
  const n = sig($p.start ?? 0)
  const r = T()
  on(r, 'click', () => {
    n.v++
  })
  insert(r, () => n.v, r.childNodes[1])
  return r
}
Counter.h = 'k3x9'
