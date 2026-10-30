import { sig, drv, fx, tpl, insert, attr, on, list, when } from "__RT__/brain.js";
import { budgetEnter as $brainBudgetbudgetEnter, budgetLeave as $brainBudgetbudgetLeave, budgetOwner as $brainBudgetbudgetOwner, budgetStep as $brainBudgetbudgetStep } from "__RT__/brain-budget.js";
import { dev, fragment as $brainBudgetFragment } from "__RT__/brain-dev.js";
import Field from "./field.brain.js";
const $t0 = $brainBudgetFragment(tpl("<option value=\"\" disabled><!></option>"), "bj0c8d", 0);
const $t1 = $brainBudgetFragment(tpl("<option><!></option>"), "bj0c8d", 1);
const $t2 = $brainBudgetFragment(tpl("<select data-b=\"bj0c8d\"><!> <!></select>"), "bj0c8d", 2);
const $t3 = $brainBudgetFragment(tpl("<!><!>"), "bj0c8d", 3);
function Select($p = {}) {
  const $brainBudgetOwner = $brainBudgetbudgetOwner();
  const items = drv(() => { const $$brainBudgetFrame1 = $brainBudgetbudgetEnter({ file: ".brain/kit/ui/select.brain", component: "Select", line: 4, column: 21 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame1); { return $p.options.map((o) => { const $$brainBudgetFrame2 = $brainBudgetbudgetEnter({ file: ".brain/kit/ui/select.brain", component: "Select", line: 4, column: 39 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame2); { return (typeof o === 'object' ? o : { value: String(o), label: String(o) }); } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame2); } }); } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame1); } });
  const sel = sig(null, "sel");
  fx(() => { const $$brainBudgetFrame3 = $brainBudgetbudgetEnter({ file: ".brain/kit/ui/select.brain", component: "Select", line: 7, column: 6 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame3); {
    const n = items.v.length;
    if (!sel.v || !n) {
      return;
    }
    sel.v.value = $p.value || '';
  } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame3); } });
  const $r14 = $t3();
  const $e4 = $r14.firstChild;
  insert(null, Field({ get "label"() { return $p.label; }, get "error"() { return $p.error; }, get "help"() { return $p.help; }, children: () => {
      const $r13 = $t2();
      const $e5 = $r13;
      const $e6 = $r13.firstChild;
      const $e9 = $e6.nextSibling.nextSibling;
      sel.v = $e5;
      attr($e5, "name", () => $p.name);
      attr($e5, "required", () => ($p.required ?? false));
      attr($e5, "disabled", () => ($p.disabled ?? false));
      attr($e5, "aria-label", () => $p.label);
      attr($e5, "value", () => $p.value);
      on($e5, "change", (e) => { $p["bind:value"]?.(e.target.value); });
      when(null, () => $p.placeholder, () => {
        const $r8 = $t0();
        const $e7 = $r8.firstChild;
        insert(null, () => $p.placeholder, $e7);
        return $r8;
      }, null, $e6);
      list(null, () => items.v, (o) => o.value, (o) => {
        const $r12 = $t1();
        const $e10 = $r12;
        const $e11 = $r12.firstChild;
        attr($e10, "value", () => o.v.value);
        insert(null, () => o.v.label, $e11);
        return $r12;
      }, $e9);
      return $r13;
    } }), $e4);
  return $r14;
}
Select.h = "bj0c8d";
Select.$hmr = { version: 1, wiring: "import { sig, drv, fx, tpl, insert, attr, on, list, when } from \"__RT__/brain.js\";\nimport { budgetEnter as $brainBudgetbudgetEnter, budgetLeave as $brainBudgetbudgetLeave, budgetOwner as $brainBudgetbudgetOwner, budgetStep as $brainBudgetbudgetStep } from \"__RT__/brain-budget.js\";\nimport { dev, fragment as $brainBudgetFragment } from \"__RT__/brain-dev.js\";\nimport Field from \"__RT__/ui/field.brain\";\nconst $t0 = $brainBudgetFragment(tpl(\"__brain_static_template__\"), \"bj0c8d\", 0);\nconst $t1 = $brainBudgetFragment(tpl(\"__brain_static_template__\"), \"bj0c8d\", 1);\nconst $t2 = $brainBudgetFragment(tpl(\"__brain_static_template__\"), \"bj0c8d\", 2);\nconst $t3 = $brainBudgetFragment(tpl(\"__brain_static_template__\"), \"bj0c8d\", 3);\nfunction Select($p = {}) {\n  const $brainBudgetOwner = $brainBudgetbudgetOwner();\n  const items = drv(() => { const $$brainBudgetFrame1 = $brainBudgetbudgetEnter({ file: \".brain/kit/ui/select.brain\", component: \"Select\", line: 4, column: 21 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame1); { return $p.options.map((o) => { const $$brainBudgetFrame2 = $brainBudgetbudgetEnter({ file: \".brain/kit/ui/select.brain\", component: \"Select\", line: 4, column: 39 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame2); { return (typeof o === 'object' ? o : { value: String(o), label: String(o) }); } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame2); } }); } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame1); } });\n  const sel = sig(null, \"sel\");\n  fx(() => { const $$brainBudgetFrame3 = $brainBudgetbudgetEnter({ file: \".brain/kit/ui/select.brain\", component: \"Select\", line: 7, column: 6 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame3); {\n    const n = items.v.length;\n    if (!sel.v || !n) {\n      return;\n    }\n    sel.v.value = $p.value || '';\n  } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame3); } });\n  const $r14 = $t3();\n  const $e4 = $r14.firstChild;\n  insert(null, Field({ get \"label\"() { return $p.label; }, get \"error\"() { return $p.error; }, get \"help\"() { return $p.help; }, children: () => {\n      const $r13 = $t2();\n      const $e5 = $r13;\n      const $e6 = $r13.firstChild;\n      const $e9 = $e6.nextSibling.nextSibling;\n      sel.v = $e5;\n      attr($e5, \"name\", () => $p.name);\n      attr($e5, \"required\", () => ($p.required ?? false));\n      attr($e5, \"disabled\", () => ($p.disabled ?? false));\n      attr($e5, \"aria-label\", () => $p.label);\n      attr($e5, \"value\", () => $p.value);\n      on($e5, \"change\", (e) => { $p[\"bind:value\"]?.(e.target.value); });\n      when(null, () => $p.placeholder, () => {\n        const $r8 = $t0();\n        const $e7 = $r8.firstChild;\n        insert(null, () => $p.placeholder, $e7);\n        return $r8;\n      }, null, $e6);\n      list(null, () => items.v, (o) => o.value, (o) => {\n        const $r12 = $t1();\n        const $e10 = $r12;\n        const $e11 = $r12.firstChild;\n        attr($e10, \"value\", () => o.v.value);\n        insert(null, () => o.v.label, $e11);\n        return $r12;\n      }, $e9);\n      return $r13;\n    } }), $e4);\n  return $r14;\n}\nSelect.h = \"bj0c8d\";\nexport default dev(Select);\n", templates: ["<option value=\"\" disabled><!></option>","<option><!></option>","<select data-b=\"bj0c8d\"><!> <!></select>","<!><!>"] };
export default dev(Select);
//# sourceMappingURL=data:application/json;base64,eyJ2ZXJzaW9uIjozLCJmaWxlIjoic2VsZWN0LmJyYWluLmpzIiwic291cmNlcyI6WyJzZWxlY3QuYnJhaW4iXSwic291cmNlc0NvbnRlbnQiOlsiPG1ldGEgcHJvcHM9XCJsYWJlbDpzdHIgdmFsdWU6c3RyfiBvcHRpb25zOmxpc3QgcGxhY2Vob2xkZXI6c3RyPyBuYW1lOnN0cj8gcmVxdWlyZWQ6Ym9vbD1mYWxzZSBkaXNhYmxlZDpib29sPWZhbHNlIGVycm9yOnN0cj8gaGVscDpzdHI/XCI+XG48c2NyaXB0PlxuICAvLyBvcHRpb25zOiBbJ2EnLCAnYiddIG9yIFt7IHZhbHVlOiAnYScsIGxhYmVsOiAnQScgfV1cbiAgY29uc3QgaXRlbXMgPSBkcnYoKCkgPT4gb3B0aW9ucy5tYXAoKG8pID0+ICh0eXBlb2YgbyA9PT0gJ29iamVjdCcgPyBvIDogeyB2YWx1ZTogU3RyaW5nKG8pLCBsYWJlbDogU3RyaW5nKG8pIH0pKSlcbiAgbGV0IHNlbCA9IHJlZigpXG4gIC8vIGEgPHNlbGVjdD4gdmFsdWUgb25seSBzdGlja3Mgb25jZSBpdHMgb3B0aW9ucyBleGlzdDogcmUtYXBwbHkgaXQgYWZ0ZXIgdGhleSByZW5kZXJcbiAgZngoKCkgPT4ge1xuICAgIGNvbnN0IG4gPSBpdGVtcy5sZW5ndGhcbiAgICBpZiAoIXNlbCB8fCAhbikgcmV0dXJuXG4gICAgc2VsLnZhbHVlID0gdmFsdWUgfHwgJydcbiAgfSlcbjwvc2NyaXB0PlxuPEZpZWxkIGxhYmVsPXtsYWJlbH0gZXJyb3I9e2Vycm9yfSBoZWxwPXtoZWxwfT5cbiAgPHNlbGVjdCBiaW5kOnJlZj17c2VsfSBuYW1lPXtuYW1lfSByZXF1aXJlZD17cmVxdWlyZWR9IGRpc2FibGVkPXtkaXNhYmxlZH0gYXJpYS1sYWJlbD17bGFiZWx9IGJpbmQ6dmFsdWU9e3ZhbHVlfT5cbiAgICB7I2lmIHBsYWNlaG9sZGVyfTxvcHRpb24gdmFsdWU9XCJcIiBkaXNhYmxlZD57cGxhY2Vob2xkZXJ9PC9vcHRpb24+ey9pZn1cbiAgICB7I2VhY2ggaXRlbXMgYXMgbyAoby52YWx1ZSl9XG4gICAgICA8b3B0aW9uIHZhbHVlPXtvLnZhbHVlfT57by5sYWJlbH08L29wdGlvbj5cbiAgICB7L2VhY2h9XG4gIDwvc2VsZWN0PlxuPC9GaWVsZD5cbjxzdHlsZT5cbiAgc2VsZWN0IHtcbiAgICB3aWR0aDogMTAwJTsgbWluLWhlaWdodDogNDRweDsgcGFkZGluZzogdmFyKC0tc3BhY2UtMikgdmFyKC0tc3BhY2UtMyk7XG4gICAgZm9udDogMTZweC8xLjQgdmFyKC0tZm9udCk7IGNvbG9yOiB2YXIoLS1mZyk7IGJhY2tncm91bmQ6IHZhcigtLXN1cmZhY2UpO1xuICAgIGJvcmRlcjogMXB4IHNvbGlkIHZhcigtLWxpbmUpOyBib3JkZXItcmFkaXVzOiB2YXIoLS1yYWRpdXMtc20pO1xuICB9XG4gIHNlbGVjdDpmb2N1cy12aXNpYmxlIHsgb3V0bGluZTogMnB4IHNvbGlkIHZhcigtLWFjY2VudCk7IG91dGxpbmUtb2Zmc2V0OiAxcHg7IH1cbiAgc2VsZWN0W2FyaWEtaW52YWxpZD0ndHJ1ZSddIHsgYm9yZGVyLWNvbG9yOiB2YXIoLS1iYWQpOyB9XG48L3N0eWxlPlxuIl0sIm5hbWVzIjpbXSwibWFwcGluZ3MiOiI7Ozs7Ozs7Ozs7QUFHQTtBQUNBO0FBRUE7QUFDQTtBQUNBO0FBQUE7QUFBQTtBQUNBO0FBQUE7QUFBQTtBQUFBO0FBR0E7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUNBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQ0E7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQ0E7QUFBQTtBQUFBO0FBQUE7QUFDQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBOyJ9

