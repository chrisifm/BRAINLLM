import { guard } from "__RT__/brain-guard.js";
import { sig, drv, fx, tpl, insert, cls, on } from "__RT__/brain.js";
import { budgetEnter as $brainBudgetbudgetEnter, budgetLeave as $brainBudgetbudgetLeave, budgetOwner as $brainBudgetbudgetOwner, budgetStep as $brainBudgetbudgetStep } from "__RT__/brain-budget.js";
import { dev, fragment as $brainBudgetFragment } from "__RT__/brain-dev.js";
const $t0 = $brainBudgetFragment(tpl("<button data-b=\"rs62nv\"><!>: <!> × 2 = <!></button>"), "rs62nv", 0);
function Counter($p = {}) {
  const $brainBudgetOwner = $brainBudgetbudgetOwner();
  const n = sig(($p.start ?? 0), "n");
  const dbl = drv(() => { const $$brainBudgetFrame1 = $brainBudgetbudgetEnter({ file: "comp/counter.brain", component: "Counter", line: 5, column: 17 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame1); { return n.v * 2; } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame1); } });
  fx(() => { const $$brainBudgetFrame2 = $brainBudgetbudgetEnter({ file: "comp/counter.brain", component: "Counter", line: 6, column: 6 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame2); { return console.log(n.v); } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame2); } });
  const $r7 = $t0();
  const $e3 = $r7;
  const $e4 = $r7.firstChild;
  const $e5 = $e4.nextSibling.nextSibling;
  const $e6 = $e5.nextSibling.nextSibling;
  on($e3, "click", () => { n.v++; });
  cls($e3, "hot", () => n.v > 10);
  insert(null, () => ($p.label ?? 'Clicks'), $e4);
  insert(null, () => n.v, $e5);
  insert(null, () => dbl.v, $e6);
  return $r7;
}
Counter.h = "rs62nv";
Counter.f = "comp/counter.brain";
Counter.$hmr = { version: 1, wiring: "import { guard } from \"__RT__/brain-guard.js\";\nimport { sig, drv, fx, tpl, insert, cls, on } from \"__RT__/brain.js\";\nimport { budgetEnter as $brainBudgetbudgetEnter, budgetLeave as $brainBudgetbudgetLeave, budgetOwner as $brainBudgetbudgetOwner, budgetStep as $brainBudgetbudgetStep } from \"__RT__/brain-budget.js\";\nimport { dev, fragment as $brainBudgetFragment } from \"__RT__/brain-dev.js\";\nconst $t0 = $brainBudgetFragment(tpl(\"__brain_static_template__\"), \"rs62nv\", 0);\nfunction Counter($p = {}) {\n  const $brainBudgetOwner = $brainBudgetbudgetOwner();\n  const n = sig(($p.start ?? 0), \"n\");\n  const dbl = drv(() => { const $$brainBudgetFrame1 = $brainBudgetbudgetEnter({ file: \"comp/counter.brain\", component: \"Counter\", line: 5, column: 17 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame1); { return n.v * 2; } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame1); } });\n  fx(() => { const $$brainBudgetFrame2 = $brainBudgetbudgetEnter({ file: \"comp/counter.brain\", component: \"Counter\", line: 6, column: 6 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame2); { return console.log(n.v); } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame2); } });\n  const $r7 = $t0();\n  const $e3 = $r7;\n  const $e4 = $r7.firstChild;\n  const $e5 = $e4.nextSibling.nextSibling;\n  const $e6 = $e5.nextSibling.nextSibling;\n  on($e3, \"click\", () => { n.v++; });\n  cls($e3, \"hot\", () => n.v > 10);\n  insert(null, () => ($p.label ?? 'Clicks'), $e4);\n  insert(null, () => n.v, $e5);\n  insert(null, () => dbl.v, $e6);\n  return $r7;\n}\nCounter.h = \"rs62nv\";\nCounter.f = \"comp/counter.brain\";\nexport default dev(guard(Counter));\n", templates: ["<button data-b=\"rs62nv\"><!>: <!> × 2 = <!></button>"] };
export default dev(guard(Counter));
//# sourceMappingURL=data:application/json;base64,eyJ2ZXJzaW9uIjozLCJmaWxlIjoiY291bnRlci5icmFpbi5qcyIsInNvdXJjZXMiOlsiY291bnRlci5icmFpbiJdLCJzb3VyY2VzQ29udGVudCI6WyI8bWV0YSBwcm9wcz1cInN0YXJ0Om51bT0wIGxhYmVsOnN0cj0nQ2xpY2tzJ1wiPlxuXG48c2NyaXB0PlxuICBsZXQgbiA9IHNpZyhzdGFydClcbiAgbGV0IGRibCA9IGRydigoKSA9PiBuICogMilcbiAgZngoKCkgPT4gbG9nKG4pKVxuPC9zY3JpcHQ+XG5cbjxidXR0b24gb246Y2xpY2s9e24rK30gY2xhc3M6aG90PXtuID4gMTB9PntsYWJlbH06IHtufSDDlyAyID0ge2RibH08L2J1dHRvbj5cblxuPHN0eWxlPlxuICBidXR0b24geyBwYWRkaW5nOiB2YXIoLS1zcGFjZS0yKTsgJi5ob3QgeyBjb2xvcjogdmFyKC0tYmFkKTsgfSB9XG48L3N0eWxlPlxuIl0sIm5hbWVzIjpbXSwibWFwcGluZ3MiOiI7Ozs7Ozs7QUFHQTtBQUNBO0FBQ0E7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBR0E7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTsifQ==
