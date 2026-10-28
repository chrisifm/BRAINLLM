import { tpl, attr, cls } from "__RT__/brain.js";
import { dev } from "__RT__/brain-dev.js";
const $t0 = tpl("<span data-b=\"2sqwfa\" class=\"spacer\" aria-hidden=\"true\"></span>");
function Spacer($p = {}) {
  const sp = (n) => (n == null || n === '' ? null : String(n) === '0' ? '0' : 'var(--space-' + n + ')');
  const $r2 = $t0();
  const $e1 = $r2;
  cls($e1, "grow", () => !$p.size);
  attr($e1, "style:flex-basis", () => sp($p.size));
  return $r2;
}
Spacer.h = "2sqwfa";
export default dev(Spacer);
//# sourceMappingURL=data:application/json;base64,eyJ2ZXJzaW9uIjozLCJmaWxlIjoic3BhY2VyLmJyYWluLmpzIiwic291cmNlcyI6WyJzcGFjZXIuYnJhaW4iXSwic291cmNlc0NvbnRlbnQiOlsiPG1ldGEgcHJvcHM9XCJzaXplOnN0cj9cIj5cbjxzY3JpcHQ+XG4gIC8vIGluc2lkZSBhIFN0YWNrIG9yIFJvdzogd2l0aCBubyBzaXplIGl0IHRha2VzIHRoZSByZW1haW5pbmcgc3BhY2UgKHB1c2hlcyB0aGUgbmV4dCBpdGVtcyB0byB0aGUgZW5kKTsgd2l0aCBhXG4gIC8vIHRva2VuIHN0ZXAgaXQgaXMgYSBmaXhlZCBnYXAgb2YgdGhhdCBzaXplXG4gIC8vIHNwYWNpbmcgcHJvcHMgdGFrZSBhIHRva2VuIHN0ZXAgZnJvbSB0b2tlbnMuY3NzYnJhaW4gKDDigJM2IOKGkiAtLXNwYWNlLU4pLCBuZXZlciByYXcgQ1NTXG4gIGNvbnN0IHNwID0gKG4pID0+IChuID09IG51bGwgfHwgbiA9PT0gJycgPyBudWxsIDogU3RyaW5nKG4pID09PSAnMCcgPyAnMCcgOiAndmFyKC0tc3BhY2UtJyArIG4gKyAnKScpXG48L3NjcmlwdD5cbjxzcGFuIGNsYXNzPVwic3BhY2VyXCIgY2xhc3M6Z3Jvdz17IXNpemV9IHN0eWxlOmZsZXgtYmFzaXM9e3NwKHNpemUpfSBhcmlhLWhpZGRlbj1cInRydWVcIj48L3NwYW4+XG48c3R5bGU+XG4gIC5zcGFjZXIgeyBkaXNwbGF5OiBibG9jazsgZmxleC1zaHJpbms6IDA7IH1cbiAgLnNwYWNlci5ncm93IHsgZmxleC1ncm93OiAxOyB9XG48L3N0eWxlPlxuIl0sIm5hbWVzIjpbXSwibWFwcGluZ3MiOiI7Ozs7QUFLQTtBQUFBO0FBQUE7QUFFQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7In0=

