import { tpl, insert, attr } from "__RT__/brain.js";
import { budgetEnter as $brainBudgetbudgetEnter, budgetLeave as $brainBudgetbudgetLeave, budgetOwner as $brainBudgetbudgetOwner, budgetStep as $brainBudgetbudgetStep } from "__RT__/brain-budget.js";
import { dev, fragment as $brainBudgetFragment } from "__RT__/brain-dev.js";
const $t0 = $brainBudgetFragment(tpl("<div data-b=\"5ht90z\" class=\"stack\"><!></div>"), "5ht90z", 0);
function Stack($p = {}) {
  const $brainBudgetOwner = $brainBudgetbudgetOwner();
  const sp = (n) => { const $$brainBudgetFrame1 = $brainBudgetbudgetEnter({ file: ".brain/kit/ui/stack.brain", component: "Stack", line: 5, column: 14 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame1); { return (n == null || n === '' ? null : String(n) === '0' ? '0' : 'var(--space-' + n + ')'); } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame1); } };
  const $r4 = $t0();
  const $e2 = $r4;
  const $e3 = $r4.firstChild;
  attr($e2, "style:gap", () => sp(($p.gap ?? '3')));
  attr($e2, "style:align-items", () => ($p.align ?? 'stretch'));
  insert(null, $p.children ? $p.children() : null, $e3);
  return $r4;
}
Stack.h = "5ht90z";
Stack.$hmr = { version: 1, wiring: "import { tpl, insert, attr } from \"__RT__/brain.js\";\nimport { budgetEnter as $brainBudgetbudgetEnter, budgetLeave as $brainBudgetbudgetLeave, budgetOwner as $brainBudgetbudgetOwner, budgetStep as $brainBudgetbudgetStep } from \"__RT__/brain-budget.js\";\nimport { dev, fragment as $brainBudgetFragment } from \"__RT__/brain-dev.js\";\nconst $t0 = $brainBudgetFragment(tpl(\"__brain_static_template__\"), \"5ht90z\", 0);\nfunction Stack($p = {}) {\n  const $brainBudgetOwner = $brainBudgetbudgetOwner();\n  const sp = (n) => { const $$brainBudgetFrame1 = $brainBudgetbudgetEnter({ file: \".brain/kit/ui/stack.brain\", component: \"Stack\", line: 5, column: 14 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame1); { return (n == null || n === '' ? null : String(n) === '0' ? '0' : 'var(--space-' + n + ')'); } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame1); } };\n  const $r4 = $t0();\n  const $e2 = $r4;\n  const $e3 = $r4.firstChild;\n  attr($e2, \"style:gap\", () => sp(($p.gap ?? '3')));\n  attr($e2, \"style:align-items\", () => ($p.align ?? 'stretch'));\n  insert(null, $p.children ? $p.children() : null, $e3);\n  return $r4;\n}\nStack.h = \"5ht90z\";\nexport default dev(Stack);\n", templates: ["<div data-b=\"5ht90z\" class=\"stack\"><!></div>"] };
export default dev(Stack);
//# sourceMappingURL=data:application/json;base64,eyJ2ZXJzaW9uIjozLCJmaWxlIjoic3RhY2suYnJhaW4uanMiLCJzb3VyY2VzIjpbInN0YWNrLmJyYWluIl0sInNvdXJjZXNDb250ZW50IjpbIjxtZXRhIHByb3BzPVwiZ2FwOnN0cj0nMycgYWxpZ246c3RyPSdzdHJldGNoJ1wiPlxuPHNjcmlwdD5cbiAgLy8gYSB2ZXJ0aWNhbCBmbG93OiBjaGlsZHJlbiBvbmUgdW5kZXIgdGhlIG90aGVyIHdpdGggYSB0b2tlbiBnYXA7IGFsaWduOiBzdGFydCB8IGNlbnRlciB8IGVuZCB8IHN0cmV0Y2hcbiAgLy8gc3BhY2luZyBwcm9wcyB0YWtlIGEgdG9rZW4gc3RlcCBmcm9tIHRva2Vucy5jc3NicmFpbiAoMOKAkzYg4oaSIC0tc3BhY2UtTiksIG5ldmVyIHJhdyBDU1NcbiAgY29uc3Qgc3AgPSAobikgPT4gKG4gPT0gbnVsbCB8fCBuID09PSAnJyA/IG51bGwgOiBTdHJpbmcobikgPT09ICcwJyA/ICcwJyA6ICd2YXIoLS1zcGFjZS0nICsgbiArICcpJylcbjwvc2NyaXB0PlxuPGRpdiBjbGFzcz1cInN0YWNrXCIgc3R5bGU6Z2FwPXtzcChnYXApfSBzdHlsZTphbGlnbi1pdGVtcz17YWxpZ259PjxzbG90IC8+PC9kaXY+XG48c3R5bGU+XG4gIC5zdGFjayB7IGRpc3BsYXk6IGZsZXg7IGZsZXgtZGlyZWN0aW9uOiBjb2x1bW47IG1pbi13aWR0aDogMDsgfVxuPC9zdHlsZT5cbiJdLCJuYW1lcyI6W10sIm1hcHBpbmdzIjoiOzs7Ozs7QUFJQTtBQUFBO0FBQUE7QUFBQTtBQUVBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7In0=

