import { tpl, insert, attr, on, when } from "__RT__/brain.js";
import { budgetOwner as $brainBudgetbudgetOwner } from "__RT__/brain-budget.js";
import { dev, fragment as $brainBudgetFragment } from "__RT__/brain-dev.js";
const $t0 = $brainBudgetFragment(tpl("<p class=\"error\" role=\"alert\"><!></p>"), "qhrpuy", 0);
const $t1 = $brainBudgetFragment(tpl("<div data-b=\"qhrpuy\" class=\"check\"><label><input type=\"checkbox\"> <span><!></span></label> <!></div>"), "qhrpuy", 1);
function Checkbox($p = {}) {
  const $brainBudgetOwner = $brainBudgetbudgetOwner();
  const $r6 = $t1();
  const $e1 = $r6.firstChild.firstChild;
  const $e2 = $e1.nextSibling.nextSibling.firstChild;
  const $e3 = $r6.firstChild.nextSibling.nextSibling;
  attr($e1, "name", () => $p.name);
  attr($e1, "disabled", () => ($p.disabled ?? false));
  attr($e1, "aria-invalid", () => $p.error ? 'true' : null);
  attr($e1, "checked", () => $p.checked);
  on($e1, "change", (e) => { $p["bind:checked"]?.(e.target.checked); });
  insert(null, () => $p.label, $e2);
  when(null, () => $p.error, () => {
    const $r5 = $t0();
    const $e4 = $r5.firstChild;
    insert(null, () => $p.error, $e4);
    return $r5;
  }, null, $e3);
  return $r6;
}
Checkbox.h = "qhrpuy";
Checkbox.$hmr = { version: 1, wiring: "import { tpl, insert, attr, on, when } from \"__RT__/brain.js\";\nimport { budgetOwner as $brainBudgetbudgetOwner } from \"__RT__/brain-budget.js\";\nimport { dev, fragment as $brainBudgetFragment } from \"__RT__/brain-dev.js\";\nconst $t0 = $brainBudgetFragment(tpl(\"__brain_static_template__\"), \"qhrpuy\", 0);\nconst $t1 = $brainBudgetFragment(tpl(\"__brain_static_template__\"), \"qhrpuy\", 1);\nfunction Checkbox($p = {}) {\n  const $brainBudgetOwner = $brainBudgetbudgetOwner();\n  const $r6 = $t1();\n  const $e1 = $r6.firstChild.firstChild;\n  const $e2 = $e1.nextSibling.nextSibling.firstChild;\n  const $e3 = $r6.firstChild.nextSibling.nextSibling;\n  attr($e1, \"name\", () => $p.name);\n  attr($e1, \"disabled\", () => ($p.disabled ?? false));\n  attr($e1, \"aria-invalid\", () => $p.error ? 'true' : null);\n  attr($e1, \"checked\", () => $p.checked);\n  on($e1, \"change\", (e) => { $p[\"bind:checked\"]?.(e.target.checked); });\n  insert(null, () => $p.label, $e2);\n  when(null, () => $p.error, () => {\n    const $r5 = $t0();\n    const $e4 = $r5.firstChild;\n    insert(null, () => $p.error, $e4);\n    return $r5;\n  }, null, $e3);\n  return $r6;\n}\nCheckbox.h = \"qhrpuy\";\nexport default dev(Checkbox);\n", templates: ["<p class=\"error\" role=\"alert\"><!></p>","<div data-b=\"qhrpuy\" class=\"check\"><label><input type=\"checkbox\"> <span><!></span></label> <!></div>"] };
export default dev(Checkbox);
//# sourceMappingURL=data:application/json;base64,eyJ2ZXJzaW9uIjozLCJmaWxlIjoiY2hlY2tib3guYnJhaW4uanMiLCJzb3VyY2VzIjpbImNoZWNrYm94LmJyYWluIl0sInNvdXJjZXNDb250ZW50IjpbIjxtZXRhIHByb3BzPVwibGFiZWw6c3RyIGNoZWNrZWQ6Ym9vbH4gbmFtZTpzdHI/IGRpc2FibGVkOmJvb2w9ZmFsc2UgZXJyb3I6c3RyP1wiPlxuPGRpdiBjbGFzcz1cImNoZWNrXCI+XG4gIDxsYWJlbD5cbiAgICA8aW5wdXQgdHlwZT1cImNoZWNrYm94XCIgbmFtZT17bmFtZX0gZGlzYWJsZWQ9e2Rpc2FibGVkfSBhcmlhLWludmFsaWQ9e2Vycm9yID8gJ3RydWUnIDogbnVsbH0gYmluZDpjaGVja2VkPXtjaGVja2VkfT5cbiAgICA8c3Bhbj57bGFiZWx9PC9zcGFuPlxuICA8L2xhYmVsPlxuICB7I2lmIGVycm9yfTxwIGNsYXNzPVwiZXJyb3JcIiByb2xlPVwiYWxlcnRcIj57ZXJyb3J9PC9wPnsvaWZ9XG48L2Rpdj5cbjxzdHlsZT5cbiAgbGFiZWwgeyBkaXNwbGF5OiBpbmxpbmUtZmxleDsgYWxpZ24taXRlbXM6IGNlbnRlcjsgZ2FwOiB2YXIoLS1zcGFjZS0yKTsgZm9udDogMTVweC8xLjQgdmFyKC0tZm9udCk7IGNvbG9yOiB2YXIoLS1mZyk7IGN1cnNvcjogcG9pbnRlcjsgfVxuICBpbnB1dCB7IHdpZHRoOiAxOHB4OyBoZWlnaHQ6IDE4cHg7IH1cbiAgLmVycm9yIHsgZm9udC1zaXplOiAxM3B4OyBjb2xvcjogdmFyKC0tYmFkKTsgbWFyZ2luOiB2YXIoLS1zcGFjZS0xKSAwIDA7IH1cbiAgQHN1cHBvcnRzIChhY2NlbnQtY29sb3I6IGF1dG8pIHsgaW5wdXQgeyBhY2NlbnQtY29sb3I6IHZhcigtLWFjY2VudCk7IH0gfVxuPC9zdHlsZT5cbiJdLCJuYW1lcyI6W10sIm1hcHBpbmdzIjoiOzs7Ozs7Ozs7OztBQUdBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFDQTtBQUVBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7In0=

