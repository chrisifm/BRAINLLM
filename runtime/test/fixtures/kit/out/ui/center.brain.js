import { tpl, insert, attr, cls } from "__RT__/brain.js";
import { dev } from "__RT__/brain-dev.js";
const $t0 = tpl("<div data-b=\"vpp6up\" class=\"center\"><!></div>");
function Center($p = {}) {
  const sp = (n) => (n == null || n === '' ? null : String(n) === '0' ? '0' : 'var(--space-' + n + ')');
  const $r3 = $t0();
  const $e1 = $r3;
  const $e2 = $r3.firstChild;
  cls($e1, "text", () => ($p.text ?? false));
  attr($e1, "style:max-width", () => ($p.max ?? '60ch'));
  attr($e1, "style:padding-inline", () => sp(($p.gutter ?? '4')));
  insert(null, $p.children ? $p.children() : null, $e2);
  return $r3;
}
Center.h = "vpp6up";
export default dev(Center);
//# sourceMappingURL=data:application/json;base64,eyJ2ZXJzaW9uIjozLCJmaWxlIjoiY2VudGVyLmJyYWluLmpzIiwic291cmNlcyI6WyJjZW50ZXIuYnJhaW4iXSwic291cmNlc0NvbnRlbnQiOlsiPG1ldGEgcHJvcHM9XCJtYXg6c3RyPSc2MGNoJyBndXR0ZXI6c3RyPSc0JyB0ZXh0OmJvb2w9ZmFsc2VcIj5cbjxzY3JpcHQ+XG4gIC8vIGEgY2VudGVyZWQgY29sdW1uIHdpdGggYSBtYXhpbXVtIHdpZHRoIGFuZCBzaWRlIGd1dHRlcnMgKGEgdG9rZW4gc3RlcCk7IHRleHQ6IGFsc28gY2VudGVyIHRoZSB0ZXh0XG4gIC8vIHNwYWNpbmcgcHJvcHMgdGFrZSBhIHRva2VuIHN0ZXAgZnJvbSB0b2tlbnMuY3NzYnJhaW4gKDDigJM2IOKGkiAtLXNwYWNlLU4pLCBuZXZlciByYXcgQ1NTXG4gIGNvbnN0IHNwID0gKG4pID0+IChuID09IG51bGwgfHwgbiA9PT0gJycgPyBudWxsIDogU3RyaW5nKG4pID09PSAnMCcgPyAnMCcgOiAndmFyKC0tc3BhY2UtJyArIG4gKyAnKScpXG48L3NjcmlwdD5cbjxkaXYgY2xhc3M9XCJjZW50ZXJcIiBjbGFzczp0ZXh0PXt0ZXh0fSBzdHlsZTptYXgtd2lkdGg9e21heH0gc3R5bGU6cGFkZGluZy1pbmxpbmU9e3NwKGd1dHRlcil9PjxzbG90IC8+PC9kaXY+XG48c3R5bGU+XG4gIC5jZW50ZXIgeyBib3gtc2l6aW5nOiBjb250ZW50LWJveDsgbWFyZ2luLWlubGluZTogYXV0bzsgfVxuICAuY2VudGVyLnRleHQgeyB0ZXh0LWFsaWduOiBjZW50ZXI7IH1cbjwvc3R5bGU+XG4iXSwibmFtZXMiOltdLCJtYXBwaW5ncyI6Ijs7OztBQUlBO0FBQUE7QUFBQTtBQUFBO0FBRUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTsifQ==

