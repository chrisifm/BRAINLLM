import { guard } from "__RT__/brain-guard.js";
import { sig, tpl, insert, on, when } from "__RT__/brain.js";
import { budgetEnter as $brainBudgetbudgetEnter, budgetLeave as $brainBudgetbudgetLeave, budgetOwner as $brainBudgetbudgetOwner, budgetResume as $brainBudgetbudgetResume, budgetStep as $brainBudgetbudgetStep, budgetSuspend as $brainBudgetbudgetSuspend } from "__RT__/brain-budget.js";
import { formScalarsSync as $brainBudgetSyncScalarForm } from "__RT__/brain-form.js";
import { dev, fragment as $brainBudgetFragment } from "__RT__/brain-dev.js";
import { urlBind as $urlBind } from "__RT__/brain-url.js";
import { form } from "__RT__/brain-form.js";
import Button from "./ui/button.brain.js";
import Checkbox from "./ui/checkbox.brain.js";
import Input from "./ui/input.brain.js";
import Select from "./ui/select.brain.js";
import Table from "./ui/table.brain.js";
import Tabs from "./ui/tabs.brain.js";
import Toast from "./ui/toast.brain.js";
import toast from "./toast.jsbrain.js";
const $t0 = $brainBudgetFragment(tpl("Save"), "ull0eg", 0);
const $t1 = $brainBudgetFragment(tpl("<p data-b=\"ull0eg\">Overview panel</p>"), "ull0eg", 1);
const $t2 = $brainBudgetFragment(tpl("<p data-b=\"ull0eg\">Details panel</p>"), "ull0eg", 2);
const $t3 = $brainBudgetFragment(tpl("<!><!>"), "ull0eg", 3);
const $t4 = $brainBudgetFragment(tpl("<main data-b=\"ull0eg\"><h1>Form</h1> <form novalidate><!> <!> <!> <!></form> <!> <!> <!></main>"), "ull0eg", 4);
function Form($p = {}) {
  const $brainBudgetOwner = $brainBudgetbudgetOwner();
  const f = $brainBudgetSyncScalarForm({"email":["email",true],"plan":["str",true],"terms":["bool",true]}, { submit: async (v) => { const $$brainBudgetFrame1 = $brainBudgetbudgetEnter({ file: "views/form.brain", component: "Form", line: 5, column: 13 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame1); {
    $brainBudgetbudgetResume($$brainBudgetFrame1, await $brainBudgetbudgetSuspend($$brainBudgetFrame1)(new Promise((r) => { const $$brainBudgetFrame2 = $brainBudgetbudgetEnter({ file: "views/form.brain", component: "Form", line: 6, column: 25 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame2); { return setTimeout(r, 20); } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame2); } })));
    toast.send({ text: 'Saved ' + v.email, kind: 'ok' });
  } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame1); } } });
  const tab = sig('a', "tab");
  const rows = Array.from({ length: 300 }, (_, i) => { const $$brainBudgetFrame3 = $brainBudgetbudgetEnter({ file: "views/form.brain", component: "Form", line: 11, column: 44 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame3); { return ({ id: i + 1, name: 'Row ' + (i + 1), n: (i * 37) % 101 }); } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame3); } });
  const cols = [{ key: 'id', label: 'ID' }, { key: 'name', label: 'Name' }, { key: 'n', label: 'Score' }];
  $urlBind(tab, "ui.ull0eg9778j3.tab");
  const $r17 = $t4();
  const $e4 = $r17.firstChild.nextSibling.nextSibling;
  const $e5 = $e4.firstChild;
  const $e6 = $e5.nextSibling.nextSibling;
  const $e7 = $e6.nextSibling.nextSibling;
  const $e8 = $e7.nextSibling.nextSibling;
  const $e10 = $e4.nextSibling.nextSibling;
  const $e15 = $e10.nextSibling.nextSibling;
  const $e16 = $e15.nextSibling.nextSibling;
  on($e4, "submit", (e) => { e.preventDefault(); (f.submit)(e); });
  insert(null, Input({ "label": "Email", "name": "email", get "value"() { return f.v.email; }, "bind:value": (v) => { f.v.email = v; }, get "error"() { return f.err.email; } }), $e5);
  insert(null, Select({ "label": "Plan", "name": "plan", "options": ['Free', 'Pro'], "placeholder": "Choose a plan", get "value"() { return f.v.plan; }, "bind:value": (v) => { f.v.plan = v; }, get "error"() { return f.err.plan; } }), $e6);
  insert(null, Checkbox({ "label": "I accept the terms", "name": "terms", get "checked"() { return f.v.terms; }, "bind:checked": (v) => { f.v.terms = v; }, get "error"() { return f.err.terms; } }), $e7);
  insert(null, Button({ "type": "submit", get "busy"() { return f.busy; }, children: () => {
      const $r9 = $t0();
      return $r9;
    } }), $e8);
  insert(null, Tabs({ "label": "Sections", "tabs": [{ id: 'a', label: 'Overview' }, { id: 'b', label: 'Details' }], get "value"() { return tab.v; }, "bind:value": (v) => { tab.v = v; }, children: () => {
      const $r14 = $t3();
      const $e11 = $r14.firstChild;
      when(null, () => tab.v === 'a', () => {
        const $r12 = $t1();
        return $r12;
      }, () => {
        const $r13 = $t2();
        return $r13;
      }, $e11);
      return $r14;
    } }), $e10);
  insert(null, Table({ "caption": "Scores", "rows": rows, "columns": cols, "height": 400 }), $e15);
  insert(null, Toast({ get "from"() { return toast; }, "ms": 50 }), $e16);
  return $r17;
}
Form.h = "ull0eg";
Form.seo = ($p) => ({ "title": "Form", "description": "Kit form components with form().", "canonical": "/form" });
Form.f = "views/form.brain";
Form.$hmr = { version: 1, wiring: "import { guard } from \"__RT__/brain-guard.js\";\nimport { sig, tpl, insert, on, when } from \"__RT__/brain.js\";\nimport { budgetEnter as $brainBudgetbudgetEnter, budgetLeave as $brainBudgetbudgetLeave, budgetOwner as $brainBudgetbudgetOwner, budgetResume as $brainBudgetbudgetResume, budgetStep as $brainBudgetbudgetStep, budgetSuspend as $brainBudgetbudgetSuspend } from \"__RT__/brain-budget.js\";\nimport { formScalarsSync as $brainBudgetSyncScalarForm } from \"__RT__/brain-form.js\";\nimport { dev, fragment as $brainBudgetFragment } from \"__RT__/brain-dev.js\";\nimport { urlBind as $urlBind } from \"__RT__/brain-url.js\";\nimport { form } from \"__RT__/brain-form.js\";\nimport Button from \"__RT__/ui/button.brain\";\nimport Checkbox from \"__RT__/ui/checkbox.brain\";\nimport Input from \"__RT__/ui/input.brain\";\nimport Select from \"__RT__/ui/select.brain\";\nimport Table from \"__RT__/ui/table.brain\";\nimport Tabs from \"__RT__/ui/tabs.brain\";\nimport Toast from \"__RT__/ui/toast.brain\";\nimport toast from \"../channels/toast.jsbrain\";\nconst $t0 = $brainBudgetFragment(tpl(\"__brain_static_template__\"), \"ull0eg\", 0);\nconst $t1 = $brainBudgetFragment(tpl(\"__brain_static_template__\"), \"ull0eg\", 1);\nconst $t2 = $brainBudgetFragment(tpl(\"__brain_static_template__\"), \"ull0eg\", 2);\nconst $t3 = $brainBudgetFragment(tpl(\"__brain_static_template__\"), \"ull0eg\", 3);\nconst $t4 = $brainBudgetFragment(tpl(\"__brain_static_template__\"), \"ull0eg\", 4);\nfunction Form($p = {}) {\n  const $brainBudgetOwner = $brainBudgetbudgetOwner();\n  const f = $brainBudgetSyncScalarForm({\"email\":[\"email\",true],\"plan\":[\"str\",true],\"terms\":[\"bool\",true]}, { submit: async (v) => { const $$brainBudgetFrame1 = $brainBudgetbudgetEnter({ file: \"views/form.brain\", component: \"Form\", line: 5, column: 13 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame1); {\n    $brainBudgetbudgetResume($$brainBudgetFrame1, await $brainBudgetbudgetSuspend($$brainBudgetFrame1)(new Promise((r) => { const $$brainBudgetFrame2 = $brainBudgetbudgetEnter({ file: \"views/form.brain\", component: \"Form\", line: 6, column: 25 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame2); { return setTimeout(r, 20); } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame2); } })));\n    toast.send({ text: 'Saved ' + v.email, kind: 'ok' });\n  } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame1); } } });\n  const tab = sig('a', \"tab\");\n  const rows = Array.from({ length: 300 }, (_, i) => { const $$brainBudgetFrame3 = $brainBudgetbudgetEnter({ file: \"views/form.brain\", component: \"Form\", line: 11, column: 44 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame3); { return ({ id: i + 1, name: 'Row ' + (i + 1), n: (i * 37) % 101 }); } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame3); } });\n  const cols = [{ key: 'id', label: 'ID' }, { key: 'name', label: 'Name' }, { key: 'n', label: 'Score' }];\n  $urlBind(tab, \"ui.ull0eg9778j3.tab\");\n  const $r17 = $t4();\n  const $e4 = $r17.firstChild.nextSibling.nextSibling;\n  const $e5 = $e4.firstChild;\n  const $e6 = $e5.nextSibling.nextSibling;\n  const $e7 = $e6.nextSibling.nextSibling;\n  const $e8 = $e7.nextSibling.nextSibling;\n  const $e10 = $e4.nextSibling.nextSibling;\n  const $e15 = $e10.nextSibling.nextSibling;\n  const $e16 = $e15.nextSibling.nextSibling;\n  on($e4, \"submit\", (e) => { e.preventDefault(); (f.submit)(e); });\n  insert(null, Input({ \"label\": \"Email\", \"name\": \"email\", get \"value\"() { return f.v.email; }, \"bind:value\": (v) => { f.v.email = v; }, get \"error\"() { return f.err.email; } }), $e5);\n  insert(null, Select({ \"label\": \"Plan\", \"name\": \"plan\", \"options\": ['Free', 'Pro'], \"placeholder\": \"Choose a plan\", get \"value\"() { return f.v.plan; }, \"bind:value\": (v) => { f.v.plan = v; }, get \"error\"() { return f.err.plan; } }), $e6);\n  insert(null, Checkbox({ \"label\": \"I accept the terms\", \"name\": \"terms\", get \"checked\"() { return f.v.terms; }, \"bind:checked\": (v) => { f.v.terms = v; }, get \"error\"() { return f.err.terms; } }), $e7);\n  insert(null, Button({ \"type\": \"submit\", get \"busy\"() { return f.busy; }, children: () => {\n      const $r9 = $t0();\n      return $r9;\n    } }), $e8);\n  insert(null, Tabs({ \"label\": \"Sections\", \"tabs\": [{ id: 'a', label: 'Overview' }, { id: 'b', label: 'Details' }], get \"value\"() { return tab.v; }, \"bind:value\": (v) => { tab.v = v; }, children: () => {\n      const $r14 = $t3();\n      const $e11 = $r14.firstChild;\n      when(null, () => tab.v === 'a', () => {\n        const $r12 = $t1();\n        return $r12;\n      }, () => {\n        const $r13 = $t2();\n        return $r13;\n      }, $e11);\n      return $r14;\n    } }), $e10);\n  insert(null, Table({ \"caption\": \"Scores\", \"rows\": rows, \"columns\": cols, \"height\": 400 }), $e15);\n  insert(null, Toast({ get \"from\"() { return toast; }, \"ms\": 50 }), $e16);\n  return $r17;\n}\nForm.h = \"ull0eg\";\nForm.seo = ($p) => ({ \"title\": \"Form\", \"description\": \"Kit form components with form().\", \"canonical\": \"/form\" });\nForm.f = \"views/form.brain\";\nexport default dev(guard(Form));\n", templates: ["Save","<p data-b=\"ull0eg\">Overview panel</p>","<p data-b=\"ull0eg\">Details panel</p>","<!><!>","<main data-b=\"ull0eg\"><h1>Form</h1> <form novalidate><!> <!> <!> <!></form> <!> <!> <!></main>"] };
export default dev(guard(Form));
//# sourceMappingURL=data:application/json;base64,eyJ2ZXJzaW9uIjozLCJmaWxlIjoiZm9ybS5icmFpbi5qcyIsInNvdXJjZXMiOlsiZm9ybS5icmFpbiJdLCJzb3VyY2VzQ29udGVudCI6WyI8bWV0YSBzZW8gdGl0bGU9XCJGb3JtXCIgZGVzY3JpcHRpb249XCJLaXQgZm9ybSBjb21wb25lbnRzIHdpdGggZm9ybSgpLlwiIGNhbm9uaWNhbD1cIi9mb3JtXCI+XG48c2NyaXB0PlxuICBpbXBvcnQgdG9hc3QgZnJvbSAnLi4vY2hhbm5lbHMvdG9hc3QuanNicmFpbidcbiAgbGV0IGYgPSBmb3JtKHsgZW1haWw6ICdlbWFpbCEnLCBwbGFuOiAnc3RyIScsIHRlcm1zOiAnYm9vbCEnIH0sIHtcbiAgICBzdWJtaXQ6IGFzeW5jICh2KSA9PiB7XG4gICAgICBhd2FpdCBuZXcgUHJvbWlzZSgocikgPT4gc2V0VGltZW91dChyLCAyMCkpXG4gICAgICB0b2FzdC5zZW5kKHsgdGV4dDogJ1NhdmVkICcgKyB2LmVtYWlsLCBraW5kOiAnb2snIH0pXG4gICAgfSxcbiAgfSlcbiAgbGV0IHRhYiA9IHNpZygnYScpXG4gIGNvbnN0IHJvd3MgPSBBcnJheS5mcm9tKHsgbGVuZ3RoOiAzMDAgfSwgKF8sIGkpID0+ICh7IGlkOiBpICsgMSwgbmFtZTogJ1JvdyAnICsgKGkgKyAxKSwgbjogKGkgKiAzNykgJSAxMDEgfSkpXG4gIGNvbnN0IGNvbHMgPSBbeyBrZXk6ICdpZCcsIGxhYmVsOiAnSUQnIH0sIHsga2V5OiAnbmFtZScsIGxhYmVsOiAnTmFtZScgfSwgeyBrZXk6ICduJywgbGFiZWw6ICdTY29yZScgfV1cbjwvc2NyaXB0PlxuPG1haW4+XG4gIDxoMT5Gb3JtPC9oMT5cbiAgPGZvcm0gb246c3VibWl0PXtmLnN1Ym1pdH0gbm92YWxpZGF0ZT5cbiAgICA8SW5wdXQgbGFiZWw9XCJFbWFpbFwiIG5hbWU9XCJlbWFpbFwiIGJpbmQ6dmFsdWU9e2Yudi5lbWFpbH0gZXJyb3I9e2YuZXJyLmVtYWlsfSAvPlxuICAgIDxTZWxlY3QgbGFiZWw9XCJQbGFuXCIgbmFtZT1cInBsYW5cIiBvcHRpb25zPXtbJ0ZyZWUnLCAnUHJvJ119IHBsYWNlaG9sZGVyPVwiQ2hvb3NlIGEgcGxhblwiIGJpbmQ6dmFsdWU9e2Yudi5wbGFufSBlcnJvcj17Zi5lcnIucGxhbn0gLz5cbiAgICA8Q2hlY2tib3ggbGFiZWw9XCJJIGFjY2VwdCB0aGUgdGVybXNcIiBuYW1lPVwidGVybXNcIiBiaW5kOmNoZWNrZWQ9e2Yudi50ZXJtc30gZXJyb3I9e2YuZXJyLnRlcm1zfSAvPlxuICAgIDxCdXR0b24gdHlwZT1cInN1Ym1pdFwiIGJ1c3k9e2YuYnVzeX0+U2F2ZTwvQnV0dG9uPlxuICA8L2Zvcm0+XG4gIDxUYWJzIGxhYmVsPVwiU2VjdGlvbnNcIiB0YWJzPXtbeyBpZDogJ2EnLCBsYWJlbDogJ092ZXJ2aWV3JyB9LCB7IGlkOiAnYicsIGxhYmVsOiAnRGV0YWlscycgfV19IGJpbmQ6dmFsdWU9e3RhYn0+XG4gICAgeyNpZiB0YWIgPT09ICdhJ308cD5PdmVydmlldyBwYW5lbDwvcD57OmVsc2V9PHA+RGV0YWlscyBwYW5lbDwvcD57L2lmfVxuICA8L1RhYnM+XG4gIDxUYWJsZSBjYXB0aW9uPVwiU2NvcmVzXCIgcm93cz17cm93c30gY29sdW1ucz17Y29sc30gaGVpZ2h0PXs0MDB9IC8+XG4gIDxUb2FzdCBmcm9tPXt0b2FzdH0gbXM9ezUwfSAvPlxuPC9tYWluPlxuIl0sIm5hbWVzIjpbXSwibWFwcGluZ3MiOiI7Ozs7Ozs7Ozs7Ozs7Ozs7Ozs7Ozs7QUFHQTtBQUVBO0FBQ0E7QUFBQTtBQUdBO0FBQ0E7QUFDQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBSUE7QUFDQTtBQUNBO0FBQ0E7QUFDQTtBQUFBO0FBQUE7QUFBQTtBQUVBO0FBQUE7QUFBQTtBQUNBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUVBO0FBQ0E7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTsifQ==
