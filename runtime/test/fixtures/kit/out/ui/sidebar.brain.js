import { tpl, insert, attr, cls } from "__RT__/brain.js";
import { dev } from "__RT__/brain-dev.js";
const $t0 = tpl("<div data-b=\"iy2nr6\" class=\"sidebar\"><div class=\"side\"><!></div> <div class=\"main\"><!></div></div>");
function Sidebar($p = {}) {
  const sp = (n) => (n == null || n === '' ? null : String(n) === '0' ? '0' : 'var(--space-' + n + ')');
  const $r6 = $t0();
  const $e1 = $r6;
  const $e2 = $r6.firstChild;
  const $e3 = $e2.firstChild;
  const $e4 = $e2.nextSibling.nextSibling;
  const $e5 = $e4.firstChild;
  cls($e1, "right", () => ($p.side ?? 'left') === 'right');
  attr($e1, "style:gap", () => sp(($p.gap ?? '4')));
  attr($e2, "style:flex-basis", () => ($p.width ?? '16rem'));
  insert(null, $p["slot:side"] ? $p["slot:side"]() : null, $e3);
  attr($e4, "style:min-width", () => ($p.min ?? '50%'));
  insert(null, $p.children ? $p.children() : null, $e5);
  return $r6;
}
Sidebar.h = "iy2nr6";
export default dev(Sidebar);
//# sourceMappingURL=data:application/json;base64,eyJ2ZXJzaW9uIjozLCJmaWxlIjoic2lkZWJhci5icmFpbi5qcyIsInNvdXJjZXMiOlsic2lkZWJhci5icmFpbiJdLCJzb3VyY2VzQ29udGVudCI6WyI8bWV0YSBwcm9wcz1cInNpZGU6c3RyPSdsZWZ0JyB3aWR0aDpzdHI9JzE2cmVtJyBnYXA6c3RyPSc0JyBtaW46c3RyPSc1MCUnXCI+XG48c2NyaXB0PlxuICAvLyBhIHNpZGViYXIgbmV4dCB0byB0aGUgbWFpbiBjb250ZW50IHRoYXQgc3RhY2tzIGFib3ZlIGl0IHdoZW4gdGhlIG1haW4gY29sdW1uIHdvdWxkIGdldCBuYXJyb3dlciB0aGFuIGBtaW5gXG4gIC8vIChubyBtZWRpYSBxdWVyeSkuIENvbnRlbnQgZ29lcyBpbiA8ZGl2IHNsb3Q9XCJzaWRlXCI+IGFuZCB0aGUgZGVmYXVsdCBzbG90LlxuICAvLyBzcGFjaW5nIHByb3BzIHRha2UgYSB0b2tlbiBzdGVwIGZyb20gdG9rZW5zLmNzc2JyYWluICgw4oCTNiDihpIgLS1zcGFjZS1OKSwgbmV2ZXIgcmF3IENTU1xuICBjb25zdCBzcCA9IChuKSA9PiAobiA9PSBudWxsIHx8IG4gPT09ICcnID8gbnVsbCA6IFN0cmluZyhuKSA9PT0gJzAnID8gJzAnIDogJ3ZhcigtLXNwYWNlLScgKyBuICsgJyknKVxuPC9zY3JpcHQ+XG48ZGl2IGNsYXNzPVwic2lkZWJhclwiIGNsYXNzOnJpZ2h0PXtzaWRlID09PSAncmlnaHQnfSBzdHlsZTpnYXA9e3NwKGdhcCl9PlxuICA8ZGl2IGNsYXNzPVwic2lkZVwiIHN0eWxlOmZsZXgtYmFzaXM9e3dpZHRofT48c2xvdCBuYW1lPVwic2lkZVwiIC8+PC9kaXY+XG4gIDxkaXYgY2xhc3M9XCJtYWluXCIgc3R5bGU6bWluLXdpZHRoPXttaW59PjxzbG90IC8+PC9kaXY+XG48L2Rpdj5cbjxzdHlsZT5cbiAgLnNpZGViYXIgeyBkaXNwbGF5OiBmbGV4OyBmbGV4LXdyYXA6IHdyYXA7IH1cbiAgLnNpZGViYXIucmlnaHQgeyBmbGV4LWRpcmVjdGlvbjogcm93LXJldmVyc2U7IH1cbiAgLnNpZGUgeyBmbGV4LWdyb3c6IDE7IH1cbiAgLm1haW4geyBmbGV4LWJhc2lzOiAwOyBmbGV4LWdyb3c6IDk5OTsgfVxuPC9zdHlsZT5cbiJdLCJuYW1lcyI6W10sIm1hcHBpbmdzIjoiOzs7O0FBS0E7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFFQTtBQUFBO0FBQ0E7QUFBQTtBQUNBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTsifQ==

