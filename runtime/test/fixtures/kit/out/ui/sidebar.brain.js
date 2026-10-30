import { tpl, insert, attr, cls } from "__RT__/brain.js";
import { budgetEnter as $brainBudgetbudgetEnter, budgetLeave as $brainBudgetbudgetLeave, budgetOwner as $brainBudgetbudgetOwner, budgetStep as $brainBudgetbudgetStep } from "__RT__/brain-budget.js";
import { dev, fragment as $brainBudgetFragment } from "__RT__/brain-dev.js";
const $t0 = $brainBudgetFragment(tpl("<div data-b=\"tdaamf\" class=\"sidebar\"><div class=\"side\"><!></div> <div class=\"main\"><!></div></div>"), "tdaamf", 0);
function Sidebar($p = {}) {
  const $brainBudgetOwner = $brainBudgetbudgetOwner();
  const sp = (n) => { const $$brainBudgetFrame1 = $brainBudgetbudgetEnter({ file: ".brain/kit/ui/sidebar.brain", component: "Sidebar", line: 6, column: 14 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame1); { return (n == null || n === '' ? null : String(n) === '0' ? '0' : 'var(--space-' + n + ')'); } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame1); } };
  const $r7 = $t0();
  const $e2 = $r7;
  const $e3 = $r7.firstChild;
  const $e4 = $e3.firstChild;
  const $e5 = $e3.nextSibling.nextSibling;
  const $e6 = $e5.firstChild;
  cls($e2, "right", () => ($p.side ?? 'left') === 'right');
  attr($e2, "style:gap", () => sp(($p.gap ?? '4')));
  attr($e3, "style:flex-basis", () => ($p.width ?? '16rem'));
  insert(null, $p["slot:side"] ? $p["slot:side"]() : null, $e4);
  attr($e5, "style:min-width", () => ($p.min ?? '50%'));
  insert(null, $p.children ? $p.children() : null, $e6);
  return $r7;
}
Sidebar.h = "tdaamf";
Sidebar.$hmr = { version: 1, wiring: "import { tpl, insert, attr, cls } from \"__RT__/brain.js\";\nimport { budgetEnter as $brainBudgetbudgetEnter, budgetLeave as $brainBudgetbudgetLeave, budgetOwner as $brainBudgetbudgetOwner, budgetStep as $brainBudgetbudgetStep } from \"__RT__/brain-budget.js\";\nimport { dev, fragment as $brainBudgetFragment } from \"__RT__/brain-dev.js\";\nconst $t0 = $brainBudgetFragment(tpl(\"__brain_static_template__\"), \"tdaamf\", 0);\nfunction Sidebar($p = {}) {\n  const $brainBudgetOwner = $brainBudgetbudgetOwner();\n  const sp = (n) => { const $$brainBudgetFrame1 = $brainBudgetbudgetEnter({ file: \".brain/kit/ui/sidebar.brain\", component: \"Sidebar\", line: 6, column: 14 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame1); { return (n == null || n === '' ? null : String(n) === '0' ? '0' : 'var(--space-' + n + ')'); } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame1); } };\n  const $r7 = $t0();\n  const $e2 = $r7;\n  const $e3 = $r7.firstChild;\n  const $e4 = $e3.firstChild;\n  const $e5 = $e3.nextSibling.nextSibling;\n  const $e6 = $e5.firstChild;\n  cls($e2, \"right\", () => ($p.side ?? 'left') === 'right');\n  attr($e2, \"style:gap\", () => sp(($p.gap ?? '4')));\n  attr($e3, \"style:flex-basis\", () => ($p.width ?? '16rem'));\n  insert(null, $p[\"slot:side\"] ? $p[\"slot:side\"]() : null, $e4);\n  attr($e5, \"style:min-width\", () => ($p.min ?? '50%'));\n  insert(null, $p.children ? $p.children() : null, $e6);\n  return $r7;\n}\nSidebar.h = \"tdaamf\";\nexport default dev(Sidebar);\n", templates: ["<div data-b=\"tdaamf\" class=\"sidebar\"><div class=\"side\"><!></div> <div class=\"main\"><!></div></div>"] };
export default dev(Sidebar);
//# sourceMappingURL=data:application/json;base64,eyJ2ZXJzaW9uIjozLCJmaWxlIjoic2lkZWJhci5icmFpbi5qcyIsInNvdXJjZXMiOlsic2lkZWJhci5icmFpbiJdLCJzb3VyY2VzQ29udGVudCI6WyI8bWV0YSBwcm9wcz1cInNpZGU6c3RyPSdsZWZ0JyB3aWR0aDpzdHI9JzE2cmVtJyBnYXA6c3RyPSc0JyBtaW46c3RyPSc1MCUnXCI+XG48c2NyaXB0PlxuICAvLyBhIHNpZGViYXIgbmV4dCB0byB0aGUgbWFpbiBjb250ZW50IHRoYXQgc3RhY2tzIGFib3ZlIGl0IHdoZW4gdGhlIG1haW4gY29sdW1uIHdvdWxkIGdldCBuYXJyb3dlciB0aGFuIGBtaW5gXG4gIC8vIChubyBtZWRpYSBxdWVyeSkuIENvbnRlbnQgZ29lcyBpbiA8ZGl2IHNsb3Q9XCJzaWRlXCI+IGFuZCB0aGUgZGVmYXVsdCBzbG90LlxuICAvLyBzcGFjaW5nIHByb3BzIHRha2UgYSB0b2tlbiBzdGVwIGZyb20gdG9rZW5zLmNzc2JyYWluICgw4oCTNiDihpIgLS1zcGFjZS1OKSwgbmV2ZXIgcmF3IENTU1xuICBjb25zdCBzcCA9IChuKSA9PiAobiA9PSBudWxsIHx8IG4gPT09ICcnID8gbnVsbCA6IFN0cmluZyhuKSA9PT0gJzAnID8gJzAnIDogJ3ZhcigtLXNwYWNlLScgKyBuICsgJyknKVxuPC9zY3JpcHQ+XG48ZGl2IGNsYXNzPVwic2lkZWJhclwiIGNsYXNzOnJpZ2h0PXtzaWRlID09PSAncmlnaHQnfSBzdHlsZTpnYXA9e3NwKGdhcCl9PlxuICA8ZGl2IGNsYXNzPVwic2lkZVwiIHN0eWxlOmZsZXgtYmFzaXM9e3dpZHRofT48c2xvdCBuYW1lPVwic2lkZVwiIC8+PC9kaXY+XG4gIDxkaXYgY2xhc3M9XCJtYWluXCIgc3R5bGU6bWluLXdpZHRoPXttaW59PjxzbG90IC8+PC9kaXY+XG48L2Rpdj5cbjxzdHlsZT5cbiAgLnNpZGViYXIgeyBkaXNwbGF5OiBmbGV4OyBmbGV4LXdyYXA6IHdyYXA7IH1cbiAgLnNpZGViYXIucmlnaHQgeyBmbGV4LWRpcmVjdGlvbjogcm93LXJldmVyc2U7IH1cbiAgLnNpZGUgeyBmbGV4LWdyb3c6IDE7IH1cbiAgLm1haW4geyBmbGV4LWJhc2lzOiAwOyBmbGV4LWdyb3c6IDk5OTsgfVxuPC9zdHlsZT5cbiJdLCJuYW1lcyI6W10sIm1hcHBpbmdzIjoiOzs7Ozs7QUFLQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUVBO0FBQUE7QUFDQTtBQUFBO0FBQ0E7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7In0=

