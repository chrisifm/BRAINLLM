import { tpl, insert, attr } from "__RT__/brain.js";
import { dev } from "__RT__/brain-dev.js";
const $t0 = tpl("<div data-b=\"1g6nib\" class=\"container\"><!></div>");
function Container($p = {}) {
  const sp = (n) => (n == null || n === '' ? null : String(n) === '0' ? '0' : 'var(--space-' + n + ')');
  const W = { sm: '40rem', md: '56rem', lg: '72rem', xl: '90rem', full: 'none' };
  const $r3 = $t0();
  const $e1 = $r3;
  const $e2 = $r3.firstChild;
  attr($e1, "style:max-width", () => W[($p.size ?? 'lg')] || W.lg);
  attr($e1, "style:padding-inline", () => sp(($p.gutter ?? '4')));
  insert(null, $p.children ? $p.children() : null, $e2);
  return $r3;
}
Container.h = "1g6nib";
export default dev(Container);
//# sourceMappingURL=data:application/json;base64,eyJ2ZXJzaW9uIjozLCJmaWxlIjoiY29udGFpbmVyLmJyYWluLmpzIiwic291cmNlcyI6WyJjb250YWluZXIuYnJhaW4iXSwic291cmNlc0NvbnRlbnQiOlsiPG1ldGEgcHJvcHM9XCJzaXplOnN0cj0nbGcnIGd1dHRlcjpzdHI9JzQnXCI+XG48c2NyaXB0PlxuICAvLyB0aGUgcGFnZSdzIGNvbnRlbnQgd2lkdGg6IHNtIDQwcmVtIMK3IG1kIDU2cmVtIMK3IGxnIDcycmVtIMK3IHhsIDkwcmVtIMK3IGZ1bGw7IGd1dHRlcnMgYXJlIGEgdG9rZW4gc3RlcFxuICAvLyBzcGFjaW5nIHByb3BzIHRha2UgYSB0b2tlbiBzdGVwIGZyb20gdG9rZW5zLmNzc2JyYWluICgw4oCTNiDihpIgLS1zcGFjZS1OKSwgbmV2ZXIgcmF3IENTU1xuICBjb25zdCBzcCA9IChuKSA9PiAobiA9PSBudWxsIHx8IG4gPT09ICcnID8gbnVsbCA6IFN0cmluZyhuKSA9PT0gJzAnID8gJzAnIDogJ3ZhcigtLXNwYWNlLScgKyBuICsgJyknKVxuICBjb25zdCBXID0geyBzbTogJzQwcmVtJywgbWQ6ICc1NnJlbScsIGxnOiAnNzJyZW0nLCB4bDogJzkwcmVtJywgZnVsbDogJ25vbmUnIH1cbjwvc2NyaXB0PlxuPGRpdiBjbGFzcz1cImNvbnRhaW5lclwiIHN0eWxlOm1heC13aWR0aD17V1tzaXplXSB8fCBXLmxnfSBzdHlsZTpwYWRkaW5nLWlubGluZT17c3AoZ3V0dGVyKX0+PHNsb3QgLz48L2Rpdj5cbjxzdHlsZT5cbiAgLmNvbnRhaW5lciB7IHdpZHRoOiAxMDAlOyBib3gtc2l6aW5nOiBib3JkZXItYm94OyBtYXJnaW4taW5saW5lOiBhdXRvOyB9XG48L3N0eWxlPlxuIl0sIm5hbWVzIjpbXSwibWFwcGluZ3MiOiI7Ozs7QUFJQTtBQUNBO0FBQUE7QUFBQTtBQUFBO0FBRUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7In0=

