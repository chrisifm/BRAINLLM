import { tpl, attr, cls } from "__RT__/brain.js";
import { budgetEnter as $brainBudgetbudgetEnter, budgetLeave as $brainBudgetbudgetLeave, budgetOwner as $brainBudgetbudgetOwner, budgetStep as $brainBudgetbudgetStep } from "__RT__/brain-budget.js";
import { dev, fragment as $brainBudgetFragment } from "__RT__/brain-dev.js";
const $t0 = $brainBudgetFragment(tpl("<span data-b=\"3rwmlg\" class=\"spacer\" aria-hidden=\"true\"></span>"), "3rwmlg", 0);
function Spacer($p = {}) {
  const $brainBudgetOwner = $brainBudgetbudgetOwner();
  const sp = (n) => { const $$brainBudgetFrame1 = $brainBudgetbudgetEnter({ file: ".brain/kit/ui/spacer.brain", component: "Spacer", line: 6, column: 14 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame1); { return (n == null || n === '' ? null : String(n) === '0' ? '0' : 'var(--space-' + n + ')'); } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame1); } };
  const $r3 = $t0();
  const $e2 = $r3;
  cls($e2, "grow", () => !$p.size);
  attr($e2, "style:flex-basis", () => sp($p.size));
  return $r3;
}
Spacer.h = "3rwmlg";
Spacer.$hmr = { version: 1, wiring: "import { tpl, attr, cls } from \"__RT__/brain.js\";\nimport { budgetEnter as $brainBudgetbudgetEnter, budgetLeave as $brainBudgetbudgetLeave, budgetOwner as $brainBudgetbudgetOwner, budgetStep as $brainBudgetbudgetStep } from \"__RT__/brain-budget.js\";\nimport { dev, fragment as $brainBudgetFragment } from \"__RT__/brain-dev.js\";\nconst $t0 = $brainBudgetFragment(tpl(\"__brain_static_template__\"), \"3rwmlg\", 0);\nfunction Spacer($p = {}) {\n  const $brainBudgetOwner = $brainBudgetbudgetOwner();\n  const sp = (n) => { const $$brainBudgetFrame1 = $brainBudgetbudgetEnter({ file: \".brain/kit/ui/spacer.brain\", component: \"Spacer\", line: 6, column: 14 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame1); { return (n == null || n === '' ? null : String(n) === '0' ? '0' : 'var(--space-' + n + ')'); } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame1); } };\n  const $r3 = $t0();\n  const $e2 = $r3;\n  cls($e2, \"grow\", () => !$p.size);\n  attr($e2, \"style:flex-basis\", () => sp($p.size));\n  return $r3;\n}\nSpacer.h = \"3rwmlg\";\nexport default dev(Spacer);\n", templates: ["<span data-b=\"3rwmlg\" class=\"spacer\" aria-hidden=\"true\"></span>"] };
export default dev(Spacer);
//# sourceMappingURL=data:application/json;base64,eyJ2ZXJzaW9uIjozLCJmaWxlIjoic3BhY2VyLmJyYWluLmpzIiwic291cmNlcyI6WyJzcGFjZXIuYnJhaW4iXSwic291cmNlc0NvbnRlbnQiOlsiPG1ldGEgcHJvcHM9XCJzaXplOnN0cj9cIj5cbjxzY3JpcHQ+XG4gIC8vIGluc2lkZSBhIFN0YWNrIG9yIFJvdzogd2l0aCBubyBzaXplIGl0IHRha2VzIHRoZSByZW1haW5pbmcgc3BhY2UgKHB1c2hlcyB0aGUgbmV4dCBpdGVtcyB0byB0aGUgZW5kKTsgd2l0aCBhXG4gIC8vIHRva2VuIHN0ZXAgaXQgaXMgYSBmaXhlZCBnYXAgb2YgdGhhdCBzaXplXG4gIC8vIHNwYWNpbmcgcHJvcHMgdGFrZSBhIHRva2VuIHN0ZXAgZnJvbSB0b2tlbnMuY3NzYnJhaW4gKDDigJM2IOKGkiAtLXNwYWNlLU4pLCBuZXZlciByYXcgQ1NTXG4gIGNvbnN0IHNwID0gKG4pID0+IChuID09IG51bGwgfHwgbiA9PT0gJycgPyBudWxsIDogU3RyaW5nKG4pID09PSAnMCcgPyAnMCcgOiAndmFyKC0tc3BhY2UtJyArIG4gKyAnKScpXG48L3NjcmlwdD5cbjxzcGFuIGNsYXNzPVwic3BhY2VyXCIgY2xhc3M6Z3Jvdz17IXNpemV9IHN0eWxlOmZsZXgtYmFzaXM9e3NwKHNpemUpfSBhcmlhLWhpZGRlbj1cInRydWVcIj48L3NwYW4+XG48c3R5bGU+XG4gIC5zcGFjZXIgeyBkaXNwbGF5OiBibG9jazsgZmxleC1zaHJpbms6IDA7IH1cbiAgLnNwYWNlci5ncm93IHsgZmxleC1ncm93OiAxOyB9XG48L3N0eWxlPlxuIl0sIm5hbWVzIjpbXSwibWFwcGluZ3MiOiI7Ozs7OztBQUtBO0FBQUE7QUFBQTtBQUVBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBOyJ9

