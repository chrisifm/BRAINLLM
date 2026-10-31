import { guard } from "__RT__/brain-guard.js";
import { sig, drv, fx, tpl, insert, cls, on } from "__RT__/brain.js";
import { budgetEnter as $brainBudgetbudgetEnter, budgetLeave as $brainBudgetbudgetLeave, budgetOwner as $brainBudgetbudgetOwner, budgetStep as $brainBudgetbudgetStep } from "__RT__/brain-budget.js";
const $t0 = tpl("<button data-b=\"rs62nv\"><!>: <!> × 2 = <!></button>");
function Counter($p = {}) {
  const $brainBudgetOwner = $brainBudgetbudgetOwner();
  const n = sig(($p.start ?? 0));
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
export default guard(Counter);
