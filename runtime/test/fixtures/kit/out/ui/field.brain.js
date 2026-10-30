import { sig, fx, tpl, insert, attr, on } from "__RT__/brain.js";
import { budgetEnter as $brainBudgetbudgetEnter, budgetLeave as $brainBudgetbudgetLeave, budgetOwner as $brainBudgetbudgetOwner, budgetStep as $brainBudgetbudgetStep } from "__RT__/brain-budget.js";
import { dev, fragment as $brainBudgetFragment } from "__RT__/brain-dev.js";
const $t0 = $brainBudgetFragment(tpl("<div data-b=\"dd1s4v\" class=\"field\"><label><span class=\"label\"><!></span> <!></label> <p class=\"help\"><!></p> <p class=\"error\" role=\"alert\"><!></p></div>"), "dd1s4v", 0);
let $n = 0;
function Field($p = {}) {
  const $brainBudgetOwner = $brainBudgetbudgetOwner();
  // #268: the first instance keeps the ids as written (`/#pricing` works); later ones get `~2`, `~3`…
  const $u = $n++ ? "~" + $n : "";
  const box = sig(null, "box");
  const hint = sig(null, "hint");
  const err = sig(null, "err");
  fx(() => { const $$brainBudgetFrame1 = $brainBudgetbudgetEnter({ file: ".brain/kit/ui/field.brain", component: "Field", line: 7, column: 6 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame1); {
    if (!box.v || !hint.v || !err.v) {
      return;
    }
    const c = box.v.querySelector('input, select, textarea');
    if (!c) {
      return;
    }
    const ids = [];
    if ($p.help) {
      ids.push(hint.v.id);
    }
    if ($p.error) {
      ids.push(err.v.id);
    }
    if (ids.length) {
      c.setAttribute('aria-describedby', ids.join(' '));
    } else {
      c.removeAttribute('aria-describedby');
    }
    if ($p.error) {
      c.setAttribute('aria-invalid', 'true');
    } else {
      c.removeAttribute('aria-invalid');
    }
  } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame1); } });
  const $r9 = $t0();
  const $e2 = $r9;
  const $e3 = $r9.firstChild.firstChild.firstChild;
  const $e4 = $r9.firstChild.firstChild.nextSibling.nextSibling;
  const $e5 = $r9.firstChild.nextSibling.nextSibling;
  const $e6 = $e5.firstChild;
  const $e7 = $e5.nextSibling.nextSibling;
  const $e8 = $e7.firstChild;
  box.v = $e2;
  insert(null, () => $p.label, $e3);
  insert(null, $p.children ? $p.children() : null, $e4);
  attr($e5, "id", "hint" + $u);
  hint.v = $e5;
  attr($e5, "hidden", () => !$p.help);
  insert(null, () => $p.help, $e6);
  attr($e7, "id", "err" + $u);
  err.v = $e7;
  attr($e7, "hidden", () => !$p.error);
  insert(null, () => $p.error, $e8);
  return $r9;
}
Field.h = "dd1s4v";
Field.$hmr = { version: 1, wiring: "import { sig, fx, tpl, insert, attr, on } from \"__RT__/brain.js\";\nimport { budgetEnter as $brainBudgetbudgetEnter, budgetLeave as $brainBudgetbudgetLeave, budgetOwner as $brainBudgetbudgetOwner, budgetStep as $brainBudgetbudgetStep } from \"__RT__/brain-budget.js\";\nimport { dev, fragment as $brainBudgetFragment } from \"__RT__/brain-dev.js\";\nconst $t0 = $brainBudgetFragment(tpl(\"__brain_static_template__\"), \"dd1s4v\", 0);\nlet $n = 0;\nfunction Field($p = {}) {\n  const $brainBudgetOwner = $brainBudgetbudgetOwner();\n  // #268: the first instance keeps the ids as written (`/#pricing` works); later ones get `~2`, `~3`…\n  const $u = $n++ ? \"~\" + $n : \"\";\n  const box = sig(null, \"box\");\n  const hint = sig(null, \"hint\");\n  const err = sig(null, \"err\");\n  fx(() => { const $$brainBudgetFrame1 = $brainBudgetbudgetEnter({ file: \".brain/kit/ui/field.brain\", component: \"Field\", line: 7, column: 6 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame1); {\n    if (!box.v || !hint.v || !err.v) {\n      return;\n    }\n    const c = box.v.querySelector('input, select, textarea');\n    if (!c) {\n      return;\n    }\n    const ids = [];\n    if ($p.help) {\n      ids.push(hint.v.id);\n    }\n    if ($p.error) {\n      ids.push(err.v.id);\n    }\n    if (ids.length) {\n      c.setAttribute('aria-describedby', ids.join(' '));\n    } else {\n      c.removeAttribute('aria-describedby');\n    }\n    if ($p.error) {\n      c.setAttribute('aria-invalid', 'true');\n    } else {\n      c.removeAttribute('aria-invalid');\n    }\n  } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame1); } });\n  const $r9 = $t0();\n  const $e2 = $r9;\n  const $e3 = $r9.firstChild.firstChild.firstChild;\n  const $e4 = $r9.firstChild.firstChild.nextSibling.nextSibling;\n  const $e5 = $r9.firstChild.nextSibling.nextSibling;\n  const $e6 = $e5.firstChild;\n  const $e7 = $e5.nextSibling.nextSibling;\n  const $e8 = $e7.firstChild;\n  box.v = $e2;\n  insert(null, () => $p.label, $e3);\n  insert(null, $p.children ? $p.children() : null, $e4);\n  attr($e5, \"id\", \"hint\" + $u);\n  hint.v = $e5;\n  attr($e5, \"hidden\", () => !$p.help);\n  insert(null, () => $p.help, $e6);\n  attr($e7, \"id\", \"err\" + $u);\n  err.v = $e7;\n  attr($e7, \"hidden\", () => !$p.error);\n  insert(null, () => $p.error, $e8);\n  return $r9;\n}\nField.h = \"dd1s4v\";\nexport default dev(Field);\n", templates: ["<div data-b=\"dd1s4v\" class=\"field\"><label><span class=\"label\"><!></span> <!></label> <p class=\"help\"><!></p> <p class=\"error\" role=\"alert\"><!></p></div>"] };
export default dev(Field);
//# sourceMappingURL=data:application/json;base64,eyJ2ZXJzaW9uIjozLCJmaWxlIjoiZmllbGQuYnJhaW4uanMiLCJzb3VyY2VzIjpbImZpZWxkLmJyYWluIl0sInNvdXJjZXNDb250ZW50IjpbIjxtZXRhIHByb3BzPVwibGFiZWw6c3RyIGVycm9yOnN0cj8gaGVscDpzdHI/XCI+XG48c2NyaXB0PlxuICBsZXQgYm94ID0gcmVmKClcbiAgbGV0IGhpbnQgPSByZWYoKVxuICBsZXQgZXJyID0gcmVmKClcbiAgLy8gbGlua3MgdGhlIHNsb3R0ZWQgY29udHJvbCB0byBpdHMgaGVscCBhbmQgZXJyb3IgdGV4dCAoYXJpYS1kZXNjcmliZWRieSAvIGFyaWEtaW52YWxpZClcbiAgZngoKCkgPT4ge1xuICAgIGlmICghYm94IHx8ICFoaW50IHx8ICFlcnIpIHJldHVyblxuICAgIGNvbnN0IGMgPSBib3gucXVlcnlTZWxlY3RvcignaW5wdXQsIHNlbGVjdCwgdGV4dGFyZWEnKVxuICAgIGlmICghYykgcmV0dXJuXG4gICAgY29uc3QgaWRzID0gW11cbiAgICBpZiAoaGVscCkgaWRzLnB1c2goaGludC5pZClcbiAgICBpZiAoZXJyb3IpIGlkcy5wdXNoKGVyci5pZClcbiAgICBpZiAoaWRzLmxlbmd0aCkgYy5zZXRBdHRyaWJ1dGUoJ2FyaWEtZGVzY3JpYmVkYnknLCBpZHMuam9pbignICcpKVxuICAgIGVsc2UgYy5yZW1vdmVBdHRyaWJ1dGUoJ2FyaWEtZGVzY3JpYmVkYnknKVxuICAgIGlmIChlcnJvcikgYy5zZXRBdHRyaWJ1dGUoJ2FyaWEtaW52YWxpZCcsICd0cnVlJylcbiAgICBlbHNlIGMucmVtb3ZlQXR0cmlidXRlKCdhcmlhLWludmFsaWQnKVxuICB9KVxuPC9zY3JpcHQ+XG48ZGl2IGNsYXNzPVwiZmllbGRcIiBiaW5kOnJlZj17Ym94fT5cbiAgPGxhYmVsPlxuICAgIDxzcGFuIGNsYXNzPVwibGFiZWxcIj57bGFiZWx9PC9zcGFuPlxuICAgIDxzbG90IC8+XG4gIDwvbGFiZWw+XG4gIDxwIGlkPVwiaGludFwiIGNsYXNzPVwiaGVscFwiIGJpbmQ6cmVmPXtoaW50fSBoaWRkZW49eyFoZWxwfT57aGVscH08L3A+XG4gIDxwIGlkPVwiZXJyXCIgY2xhc3M9XCJlcnJvclwiIHJvbGU9XCJhbGVydFwiIGJpbmQ6cmVmPXtlcnJ9IGhpZGRlbj17IWVycm9yfT57ZXJyb3J9PC9wPlxuPC9kaXY+XG48c3R5bGU+XG4gIC5maWVsZCB7IGRpc3BsYXk6IGdyaWQ7IGdhcDogdmFyKC0tc3BhY2UtMSk7IH1cbiAgbGFiZWwgeyBkaXNwbGF5OiBncmlkOyBnYXA6IHZhcigtLXNwYWNlLTEpOyB9XG4gIC5sYWJlbCB7IGZvbnQ6IDYwMCAxNHB4LzEuMyB2YXIoLS1mb250KTsgY29sb3I6IHZhcigtLWZnKTsgfVxuICAuaGVscCB7IGZvbnQtc2l6ZTogMTNweDsgY29sb3I6IHZhcigtLW11dGVkKTsgbWFyZ2luOiAwOyB9XG4gIC5lcnJvciB7IGZvbnQtc2l6ZTogMTNweDsgY29sb3I6IHZhcigtLWJhZCk7IG1hcmdpbjogMDsgfVxuPC9zdHlsZT5cbiJdLCJuYW1lcyI6W10sIm1hcHBpbmdzIjoiOzs7Ozs7Ozs7QUFFQTtBQUNBO0FBQ0E7QUFFQTtBQUNBO0FBQUE7QUFBQTtBQUNBO0FBQ0E7QUFBQTtBQUFBO0FBQ0E7QUFDQTtBQUFBO0FBQUE7QUFDQTtBQUFBO0FBQUE7QUFDQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBRUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUlBO0FBRUE7QUFDQTtBQUVBO0FBQUE7QUFBQTtBQUFBO0FBQ0E7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBOyJ9

