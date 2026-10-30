import { tpl, insert, attr, cls } from "__RT__/brain.js";
import { budgetEnter as $brainBudgetbudgetEnter, budgetLeave as $brainBudgetbudgetLeave, budgetOwner as $brainBudgetbudgetOwner, budgetStep as $brainBudgetbudgetStep } from "__RT__/brain-budget.js";
import { dev, fragment as $brainBudgetFragment } from "__RT__/brain-dev.js";
const $t0 = $brainBudgetFragment(tpl("<div data-b=\"26heot\" class=\"center\"><!></div>"), "26heot", 0);
function Center($p = {}) {
  const $brainBudgetOwner = $brainBudgetbudgetOwner();
  const sp = (n) => { const $$brainBudgetFrame1 = $brainBudgetbudgetEnter({ file: ".brain/kit/ui/center.brain", component: "Center", line: 5, column: 14 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame1); { return (n == null || n === '' ? null : String(n) === '0' ? '0' : 'var(--space-' + n + ')'); } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame1); } };
  const $r4 = $t0();
  const $e2 = $r4;
  const $e3 = $r4.firstChild;
  cls($e2, "text", () => ($p.text ?? false));
  attr($e2, "style:max-width", () => ($p.max ?? '60ch'));
  attr($e2, "style:padding-inline", () => sp(($p.gutter ?? '4')));
  insert(null, $p.children ? $p.children() : null, $e3);
  return $r4;
}
Center.h = "26heot";
Center.$hmr = { version: 1, wiring: "import { tpl, insert, attr, cls } from \"__RT__/brain.js\";\nimport { budgetEnter as $brainBudgetbudgetEnter, budgetLeave as $brainBudgetbudgetLeave, budgetOwner as $brainBudgetbudgetOwner, budgetStep as $brainBudgetbudgetStep } from \"__RT__/brain-budget.js\";\nimport { dev, fragment as $brainBudgetFragment } from \"__RT__/brain-dev.js\";\nconst $t0 = $brainBudgetFragment(tpl(\"__brain_static_template__\"), \"26heot\", 0);\nfunction Center($p = {}) {\n  const $brainBudgetOwner = $brainBudgetbudgetOwner();\n  const sp = (n) => { const $$brainBudgetFrame1 = $brainBudgetbudgetEnter({ file: \".brain/kit/ui/center.brain\", component: \"Center\", line: 5, column: 14 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame1); { return (n == null || n === '' ? null : String(n) === '0' ? '0' : 'var(--space-' + n + ')'); } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame1); } };\n  const $r4 = $t0();\n  const $e2 = $r4;\n  const $e3 = $r4.firstChild;\n  cls($e2, \"text\", () => ($p.text ?? false));\n  attr($e2, \"style:max-width\", () => ($p.max ?? '60ch'));\n  attr($e2, \"style:padding-inline\", () => sp(($p.gutter ?? '4')));\n  insert(null, $p.children ? $p.children() : null, $e3);\n  return $r4;\n}\nCenter.h = \"26heot\";\nexport default dev(Center);\n", templates: ["<div data-b=\"26heot\" class=\"center\"><!></div>"] };
export default dev(Center);
//# sourceMappingURL=data:application/json;base64,eyJ2ZXJzaW9uIjozLCJmaWxlIjoiY2VudGVyLmJyYWluLmpzIiwic291cmNlcyI6WyJjZW50ZXIuYnJhaW4iXSwic291cmNlc0NvbnRlbnQiOlsiPG1ldGEgcHJvcHM9XCJtYXg6c3RyPSc2MGNoJyBndXR0ZXI6c3RyPSc0JyB0ZXh0OmJvb2w9ZmFsc2VcIj5cbjxzY3JpcHQ+XG4gIC8vIGEgY2VudGVyZWQgY29sdW1uIHdpdGggYSBtYXhpbXVtIHdpZHRoIGFuZCBzaWRlIGd1dHRlcnMgKGEgdG9rZW4gc3RlcCk7IHRleHQ6IGFsc28gY2VudGVyIHRoZSB0ZXh0XG4gIC8vIHNwYWNpbmcgcHJvcHMgdGFrZSBhIHRva2VuIHN0ZXAgZnJvbSB0b2tlbnMuY3NzYnJhaW4gKDDigJM2IOKGkiAtLXNwYWNlLU4pLCBuZXZlciByYXcgQ1NTXG4gIGNvbnN0IHNwID0gKG4pID0+IChuID09IG51bGwgfHwgbiA9PT0gJycgPyBudWxsIDogU3RyaW5nKG4pID09PSAnMCcgPyAnMCcgOiAndmFyKC0tc3BhY2UtJyArIG4gKyAnKScpXG48L3NjcmlwdD5cbjxkaXYgY2xhc3M9XCJjZW50ZXJcIiBjbGFzczp0ZXh0PXt0ZXh0fSBzdHlsZTptYXgtd2lkdGg9e21heH0gc3R5bGU6cGFkZGluZy1pbmxpbmU9e3NwKGd1dHRlcil9PjxzbG90IC8+PC9kaXY+XG48c3R5bGU+XG4gIC5jZW50ZXIgeyBib3gtc2l6aW5nOiBjb250ZW50LWJveDsgbWFyZ2luLWlubGluZTogYXV0bzsgfVxuICAuY2VudGVyLnRleHQgeyB0ZXh0LWFsaWduOiBjZW50ZXI7IH1cbjwvc3R5bGU+XG4iXSwibmFtZXMiOltdLCJtYXBwaW5ncyI6Ijs7Ozs7O0FBSUE7QUFBQTtBQUFBO0FBQUE7QUFFQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7In0=

