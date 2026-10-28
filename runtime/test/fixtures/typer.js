// An island with a text input, a button and a submit, as the codegen would emit it; used by the hydration tests.
import { sig, tpl, insert, on, attr } from '../rt.js'
const T = tpl('<form><input aria-label="Task"><button type="button">Mark</button><p><!></p></form>')
export function Typer($p) {
  const text = sig(''), items = sig([]), clicks = sig(0)
  const r = T(), inp = r.firstChild, btn = inp.nextSibling, p = btn.nextSibling
  attr(inp, 'value', () => text.v)
  on(inp, 'input', e => (text.v = e.target.value))
  on(btn, 'click', () => clicks.v++)
  on(r, 'submit', e => {
    e.preventDefault()
    items.v = [...items.v, text.v]
    text.v = ''
  })
  insert(p, () => items.v.join(',') + '|' + clicks.v, p.firstChild)
  globalThis.__typer = { text, items, clicks }
  return r
}
