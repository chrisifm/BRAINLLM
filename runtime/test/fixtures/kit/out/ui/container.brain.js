import { tpl, insert, attr } from "__RT__/brain.js";
import { budgetEnter as $brainBudgetbudgetEnter, budgetLeave as $brainBudgetbudgetLeave, budgetOwner as $brainBudgetbudgetOwner, budgetStep as $brainBudgetbudgetStep } from "__RT__/brain-budget.js";
import { dev, fragment as $brainBudgetFragment } from "__RT__/brain-dev.js";
const $t0 = $brainBudgetFragment(tpl("<div data-b=\"66zf5p\" class=\"container\"><!></div>"), "66zf5p", 0);
function Container($p = {}) {
  const $brainBudgetOwner = $brainBudgetbudgetOwner();
  const sp = (n) => { const $$brainBudgetFrame1 = $brainBudgetbudgetEnter({ file: ".brain/kit/ui/container.brain", component: "Container", line: 5, column: 14 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame1); { return (n == null || n === '' ? null : String(n) === '0' ? '0' : 'var(--space-' + n + ')'); } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame1); } };
  const W = { sm: '40rem', md: '56rem', lg: '72rem', xl: '90rem', full: 'none' };
  const $r4 = $t0();
  const $e2 = $r4;
  const $e3 = $r4.firstChild;
  attr($e2, "style:max-width", () => W[($p.size ?? 'lg')] || W.lg);
  attr($e2, "style:padding-inline", () => sp(($p.gutter ?? '4')));
  insert(null, $p.children ? $p.children() : null, $e3);
  return $r4;
}
Container.h = "66zf5p";
Container.$hmr = { version: 1, wiring: "import { tpl, insert, attr } from \"__RT__/brain.js\";\nimport { budgetEnter as $brainBudgetbudgetEnter, budgetLeave as $brainBudgetbudgetLeave, budgetOwner as $brainBudgetbudgetOwner, budgetStep as $brainBudgetbudgetStep } from \"__RT__/brain-budget.js\";\nimport { dev, fragment as $brainBudgetFragment } from \"__RT__/brain-dev.js\";\nconst $t0 = $brainBudgetFragment(tpl(\"__brain_static_template__\"), \"66zf5p\", 0);\nfunction Container($p = {}) {\n  const $brainBudgetOwner = $brainBudgetbudgetOwner();\n  const sp = (n) => { const $$brainBudgetFrame1 = $brainBudgetbudgetEnter({ file: \".brain/kit/ui/container.brain\", component: \"Container\", line: 5, column: 14 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame1); { return (n == null || n === '' ? null : String(n) === '0' ? '0' : 'var(--space-' + n + ')'); } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame1); } };\n  const W = { sm: '40rem', md: '56rem', lg: '72rem', xl: '90rem', full: 'none' };\n  const $r4 = $t0();\n  const $e2 = $r4;\n  const $e3 = $r4.firstChild;\n  attr($e2, \"style:max-width\", () => W[($p.size ?? 'lg')] || W.lg);\n  attr($e2, \"style:padding-inline\", () => sp(($p.gutter ?? '4')));\n  insert(null, $p.children ? $p.children() : null, $e3);\n  return $r4;\n}\nContainer.h = \"66zf5p\";\nexport default dev(Container);\n", templates: ["<div data-b=\"66zf5p\" class=\"container\"><!></div>"] };
export default dev(Container);
//# sourceMappingURL=data:application/json;base64,eyJ2ZXJzaW9uIjozLCJmaWxlIjoiY29udGFpbmVyLmJyYWluLmpzIiwic291cmNlcyI6WyJjb250YWluZXIuYnJhaW4iXSwic291cmNlc0NvbnRlbnQiOlsiPG1ldGEgcHJvcHM9XCJzaXplOnN0cj0nbGcnIGd1dHRlcjpzdHI9JzQnXCI+XG48c2NyaXB0PlxuICAvLyB0aGUgcGFnZSdzIGNvbnRlbnQgd2lkdGg6IHNtIDQwcmVtIMK3IG1kIDU2cmVtIMK3IGxnIDcycmVtIMK3IHhsIDkwcmVtIMK3IGZ1bGw7IGd1dHRlcnMgYXJlIGEgdG9rZW4gc3RlcFxuICAvLyBzcGFjaW5nIHByb3BzIHRha2UgYSB0b2tlbiBzdGVwIGZyb20gdG9rZW5zLmNzc2JyYWluICgw4oCTNiDihpIgLS1zcGFjZS1OKSwgbmV2ZXIgcmF3IENTU1xuICBjb25zdCBzcCA9IChuKSA9PiAobiA9PSBudWxsIHx8IG4gPT09ICcnID8gbnVsbCA6IFN0cmluZyhuKSA9PT0gJzAnID8gJzAnIDogJ3ZhcigtLXNwYWNlLScgKyBuICsgJyknKVxuICBjb25zdCBXID0geyBzbTogJzQwcmVtJywgbWQ6ICc1NnJlbScsIGxnOiAnNzJyZW0nLCB4bDogJzkwcmVtJywgZnVsbDogJ25vbmUnIH1cbjwvc2NyaXB0PlxuPGRpdiBjbGFzcz1cImNvbnRhaW5lclwiIHN0eWxlOm1heC13aWR0aD17V1tzaXplXSB8fCBXLmxnfSBzdHlsZTpwYWRkaW5nLWlubGluZT17c3AoZ3V0dGVyKX0+PHNsb3QgLz48L2Rpdj5cbjxzdHlsZT5cbiAgLmNvbnRhaW5lciB7IHdpZHRoOiAxMDAlOyBib3gtc2l6aW5nOiBib3JkZXItYm94OyBtYXJnaW4taW5saW5lOiBhdXRvOyB9XG48L3N0eWxlPlxuIl0sIm5hbWVzIjpbXSwibWFwcGluZ3MiOiI7Ozs7OztBQUlBO0FBQ0E7QUFBQTtBQUFBO0FBQUE7QUFFQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBOyJ9

