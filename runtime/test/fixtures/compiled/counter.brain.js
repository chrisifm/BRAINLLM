import { sig, drv, fx, tpl, insert, cls, on } from "__RT__/brain.js";
const $t0 = tpl("<button data-b=\"rs62nv\"><!>: <!> × 2 = <!></button>");
function Counter($p = {}) {
  const n = sig(($p.start ?? 0));
  const dbl = drv(() => n.v * 2);
  fx(() => console.log(n.v));
  const $r5 = $t0();
  const $e1 = $r5;
  const $e2 = $r5.firstChild;
  const $e3 = $e2.nextSibling.nextSibling;
  const $e4 = $e3.nextSibling.nextSibling;
  on($e1, "click", () => { n.v++; });
  cls($e1, "hot", () => n.v > 10);
  insert(null, () => ($p.label ?? 'Clicks'), $e2);
  insert(null, () => n.v, $e3);
  insert(null, () => dbl.v, $e4);
  return $r5;
}
Counter.h = "rs62nv";
export default Counter;
