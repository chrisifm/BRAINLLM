import { tpl as $tpl, insert, attr } from "__RT__/brain.js";
import { dev } from "__RT__/brain-dev.js";
const $t0 = $tpl("<div data-b=\"2ttf1o\" class=\"grid\"><!></div>");
function Grid($p = {}) {
  const sp = (n) => (n == null || n === '' ? null : String(n) === '0' ? '0' : 'var(--space-' + n + ')');
  const tpl = (c, m) => (c ? 'repeat(' + c + ', minmax(0, 1fr))' : 'repeat(auto-fit, minmax(min(' + m + ', 100%), 1fr))');
  const $r3 = $t0();
  const $e1 = $r3;
  const $e2 = $r3.firstChild;
  attr($e1, "style:gap", () => sp(($p.gap ?? '4')));
  attr($e1, "style:grid-template-columns", () => tpl($p.cols, ($p.min ?? '16rem')));
  insert(null, $p.children ? $p.children() : null, $e2);
  return $r3;
}
Grid.h = "2ttf1o";
export default dev(Grid);
//# sourceMappingURL=data:application/json;base64,eyJ2ZXJzaW9uIjozLCJmaWxlIjoiZ3JpZC5icmFpbi5qcyIsInNvdXJjZXMiOlsiZ3JpZC5icmFpbiJdLCJzb3VyY2VzQ29udGVudCI6WyI8bWV0YSBwcm9wcz1cImdhcDpzdHI9JzQnIGNvbHM6bnVtPyBtaW46c3RyPScxNnJlbSdcIj5cbjxzY3JpcHQ+XG4gIC8vIGVxdWFsIGNvbHVtbnM6IGBjb2xzYCBmaXhlZCwgZWxzZSBhcyBtYW55IGFzIGZpdCB3aXRoIGEgbWluaW11bSB3aWR0aCAoYXV0by1maXQsIG9uZSBjb2x1bW4gb24gcGhvbmVzKVxuICAvLyBzcGFjaW5nIHByb3BzIHRha2UgYSB0b2tlbiBzdGVwIGZyb20gdG9rZW5zLmNzc2JyYWluICgw4oCTNiDihpIgLS1zcGFjZS1OKSwgbmV2ZXIgcmF3IENTU1xuICBjb25zdCBzcCA9IChuKSA9PiAobiA9PSBudWxsIHx8IG4gPT09ICcnID8gbnVsbCA6IFN0cmluZyhuKSA9PT0gJzAnID8gJzAnIDogJ3ZhcigtLXNwYWNlLScgKyBuICsgJyknKVxuICBjb25zdCB0cGwgPSAoYywgbSkgPT4gKGMgPyAncmVwZWF0KCcgKyBjICsgJywgbWlubWF4KDAsIDFmcikpJyA6ICdyZXBlYXQoYXV0by1maXQsIG1pbm1heChtaW4oJyArIG0gKyAnLCAxMDAlKSwgMWZyKSknKVxuPC9zY3JpcHQ+XG48ZGl2IGNsYXNzPVwiZ3JpZFwiIHN0eWxlOmdhcD17c3AoZ2FwKX0gc3R5bGU6Z3JpZC10ZW1wbGF0ZS1jb2x1bW5zPXt0cGwoY29scywgbWluKX0+PHNsb3QgLz48L2Rpdj5cbjxzdHlsZT5cbiAgLmdyaWQgeyBkaXNwbGF5OiBncmlkOyBtaW4td2lkdGg6IDA7IH1cbjwvc3R5bGU+XG4iXSwibmFtZXMiOltdLCJtYXBwaW5ncyI6Ijs7OztBQUlBO0FBQ0E7QUFBQTtBQUFBO0FBQUE7QUFFQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTsifQ==

