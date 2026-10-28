import { tpl, insert, attr } from "__RT__/brain.js";
import { dev } from "__RT__/brain-dev.js";
const $t0 = tpl("<div data-b=\"q1gsil\" class=\"stack\"><!></div>");
function Stack($p = {}) {
  const sp = (n) => (n == null || n === '' ? null : String(n) === '0' ? '0' : 'var(--space-' + n + ')');
  const $r3 = $t0();
  const $e1 = $r3;
  const $e2 = $r3.firstChild;
  attr($e1, "style:gap", () => sp(($p.gap ?? '3')));
  attr($e1, "style:align-items", () => ($p.align ?? 'stretch'));
  insert(null, $p.children ? $p.children() : null, $e2);
  return $r3;
}
Stack.h = "q1gsil";
export default dev(Stack);
//# sourceMappingURL=data:application/json;base64,eyJ2ZXJzaW9uIjozLCJmaWxlIjoic3RhY2suYnJhaW4uanMiLCJzb3VyY2VzIjpbInN0YWNrLmJyYWluIl0sInNvdXJjZXNDb250ZW50IjpbIjxtZXRhIHByb3BzPVwiZ2FwOnN0cj0nMycgYWxpZ246c3RyPSdzdHJldGNoJ1wiPlxuPHNjcmlwdD5cbiAgLy8gYSB2ZXJ0aWNhbCBmbG93OiBjaGlsZHJlbiBvbmUgdW5kZXIgdGhlIG90aGVyIHdpdGggYSB0b2tlbiBnYXA7IGFsaWduOiBzdGFydCB8IGNlbnRlciB8IGVuZCB8IHN0cmV0Y2hcbiAgLy8gc3BhY2luZyBwcm9wcyB0YWtlIGEgdG9rZW4gc3RlcCBmcm9tIHRva2Vucy5jc3NicmFpbiAoMOKAkzYg4oaSIC0tc3BhY2UtTiksIG5ldmVyIHJhdyBDU1NcbiAgY29uc3Qgc3AgPSAobikgPT4gKG4gPT0gbnVsbCB8fCBuID09PSAnJyA/IG51bGwgOiBTdHJpbmcobikgPT09ICcwJyA/ICcwJyA6ICd2YXIoLS1zcGFjZS0nICsgbiArICcpJylcbjwvc2NyaXB0PlxuPGRpdiBjbGFzcz1cInN0YWNrXCIgc3R5bGU6Z2FwPXtzcChnYXApfSBzdHlsZTphbGlnbi1pdGVtcz17YWxpZ259PjxzbG90IC8+PC9kaXY+XG48c3R5bGU+XG4gIC5zdGFjayB7IGRpc3BsYXk6IGZsZXg7IGZsZXgtZGlyZWN0aW9uOiBjb2x1bW47IG1pbi13aWR0aDogMDsgfVxuPC9zdHlsZT5cbiJdLCJuYW1lcyI6W10sIm1hcHBpbmdzIjoiOzs7O0FBSUE7QUFBQTtBQUFBO0FBQUE7QUFFQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTsifQ==

