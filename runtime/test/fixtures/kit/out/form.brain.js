import { guard } from "__RT__/brain-guard.js";
import { sig, tpl, insert, on, when } from "__RT__/brain.js";
import { dev } from "__RT__/brain-dev.js";
import { form } from "__RT__/brain-form.js";
import Button from "./ui/button.brain.js";
import Checkbox from "./ui/checkbox.brain.js";
import Input from "./ui/input.brain.js";
import Select from "./ui/select.brain.js";
import Table from "./ui/table.brain.js";
import Tabs from "./ui/tabs.brain.js";
import Toast from "./ui/toast.brain.js";
import toast from "./toast.jsbrain.js";
const $t0 = tpl("Save");
const $t1 = tpl("<p data-b=\"ull0eg\">Overview panel</p>");
const $t2 = tpl("<p data-b=\"ull0eg\">Details panel</p>");
const $t3 = tpl("<!><!>");
const $t4 = tpl("<main data-b=\"ull0eg\"><h1>Form</h1> <form novalidate><!> <!> <!> <!></form> <!> <!> <!></main>");
function Form($p = {}) {
  const f = form({ email: 'email!', plan: 'str!', terms: 'bool!' }, { submit: async (v) => {
    await new Promise((r) => setTimeout(r, 20));
    toast.send({ text: 'Saved ' + v.email, kind: 'ok' });
  } });
  const tab = sig('a', "tab");
  const rows = Array.from({ length: 300 }, (_, i) => ({ id: i + 1, name: 'Row ' + (i + 1), n: (i * 37) % 101 }));
  const cols = [{ key: 'id', label: 'ID' }, { key: 'name', label: 'Name' }, { key: 'n', label: 'Score' }];
  const $r14 = $t4();
  const $e1 = $r14.firstChild.nextSibling.nextSibling;
  const $e2 = $e1.firstChild;
  const $e3 = $e2.nextSibling.nextSibling;
  const $e4 = $e3.nextSibling.nextSibling;
  const $e5 = $e4.nextSibling.nextSibling;
  const $e7 = $e1.nextSibling.nextSibling;
  const $e12 = $e7.nextSibling.nextSibling;
  const $e13 = $e12.nextSibling.nextSibling;
  on($e1, "submit", (e) => { e.preventDefault(); (f.submit)(e); });
  insert(null, Input({ "label": "Email", "name": "email", get "value"() { return f.v.email; }, "bind:value": (v) => { f.v.email = v; }, get "error"() { return f.err.email; } }), $e2);
  insert(null, Select({ "label": "Plan", "name": "plan", "options": ['Free', 'Pro'], "placeholder": "Choose a plan", get "value"() { return f.v.plan; }, "bind:value": (v) => { f.v.plan = v; }, get "error"() { return f.err.plan; } }), $e3);
  insert(null, Checkbox({ "label": "I accept the terms", "name": "terms", get "checked"() { return f.v.terms; }, "bind:checked": (v) => { f.v.terms = v; }, get "error"() { return f.err.terms; } }), $e4);
  insert(null, Button({ "type": "submit", get "busy"() { return f.busy; }, children: () => {
      const $r6 = $t0();
      return $r6;
    } }), $e5);
  insert(null, Tabs({ "label": "Sections", "tabs": [{ id: 'a', label: 'Overview' }, { id: 'b', label: 'Details' }], get "value"() { return tab.v; }, "bind:value": (v) => { tab.v = v; }, children: () => {
      const $r11 = $t3();
      const $e8 = $r11.firstChild;
      when(null, () => tab.v === 'a', () => {
        const $r9 = $t1();
        return $r9;
      }, () => {
        const $r10 = $t2();
        return $r10;
      }, $e8);
      return $r11;
    } }), $e7);
  insert(null, Table({ "caption": "Scores", "rows": rows, "columns": cols, "height": 400 }), $e12);
  insert(null, Toast({ get "from"() { return toast; }, "ms": 50 }), $e13);
  return $r14;
}
Form.h = "ull0eg";
Form.seo = ($p) => ({ "title": "Form", "description": "Kit form components with form().", "canonical": "/form" });
Form.f = "views/form.brain";
export default dev(guard(Form));
//# sourceMappingURL=data:application/json;base64,eyJ2ZXJzaW9uIjozLCJmaWxlIjoiZm9ybS5icmFpbi5qcyIsInNvdXJjZXMiOlsiZm9ybS5icmFpbiJdLCJzb3VyY2VzQ29udGVudCI6WyI8bWV0YSBzZW8gdGl0bGU9XCJGb3JtXCIgZGVzY3JpcHRpb249XCJLaXQgZm9ybSBjb21wb25lbnRzIHdpdGggZm9ybSgpLlwiIGNhbm9uaWNhbD1cIi9mb3JtXCI+XG48c2NyaXB0PlxuICBpbXBvcnQgdG9hc3QgZnJvbSAnLi4vY2hhbm5lbHMvdG9hc3QuanNicmFpbidcbiAgbGV0IGYgPSBmb3JtKHsgZW1haWw6ICdlbWFpbCEnLCBwbGFuOiAnc3RyIScsIHRlcm1zOiAnYm9vbCEnIH0sIHtcbiAgICBzdWJtaXQ6IGFzeW5jICh2KSA9PiB7XG4gICAgICBhd2FpdCBuZXcgUHJvbWlzZSgocikgPT4gc2V0VGltZW91dChyLCAyMCkpXG4gICAgICB0b2FzdC5zZW5kKHsgdGV4dDogJ1NhdmVkICcgKyB2LmVtYWlsLCBraW5kOiAnb2snIH0pXG4gICAgfSxcbiAgfSlcbiAgbGV0IHRhYiA9IHNpZygnYScpXG4gIGNvbnN0IHJvd3MgPSBBcnJheS5mcm9tKHsgbGVuZ3RoOiAzMDAgfSwgKF8sIGkpID0+ICh7IGlkOiBpICsgMSwgbmFtZTogJ1JvdyAnICsgKGkgKyAxKSwgbjogKGkgKiAzNykgJSAxMDEgfSkpXG4gIGNvbnN0IGNvbHMgPSBbeyBrZXk6ICdpZCcsIGxhYmVsOiAnSUQnIH0sIHsga2V5OiAnbmFtZScsIGxhYmVsOiAnTmFtZScgfSwgeyBrZXk6ICduJywgbGFiZWw6ICdTY29yZScgfV1cbjwvc2NyaXB0PlxuPG1haW4+XG4gIDxoMT5Gb3JtPC9oMT5cbiAgPGZvcm0gb246c3VibWl0PXtmLnN1Ym1pdH0gbm92YWxpZGF0ZT5cbiAgICA8SW5wdXQgbGFiZWw9XCJFbWFpbFwiIG5hbWU9XCJlbWFpbFwiIGJpbmQ6dmFsdWU9e2Yudi5lbWFpbH0gZXJyb3I9e2YuZXJyLmVtYWlsfSAvPlxuICAgIDxTZWxlY3QgbGFiZWw9XCJQbGFuXCIgbmFtZT1cInBsYW5cIiBvcHRpb25zPXtbJ0ZyZWUnLCAnUHJvJ119IHBsYWNlaG9sZGVyPVwiQ2hvb3NlIGEgcGxhblwiIGJpbmQ6dmFsdWU9e2Yudi5wbGFufSBlcnJvcj17Zi5lcnIucGxhbn0gLz5cbiAgICA8Q2hlY2tib3ggbGFiZWw9XCJJIGFjY2VwdCB0aGUgdGVybXNcIiBuYW1lPVwidGVybXNcIiBiaW5kOmNoZWNrZWQ9e2Yudi50ZXJtc30gZXJyb3I9e2YuZXJyLnRlcm1zfSAvPlxuICAgIDxCdXR0b24gdHlwZT1cInN1Ym1pdFwiIGJ1c3k9e2YuYnVzeX0+U2F2ZTwvQnV0dG9uPlxuICA8L2Zvcm0+XG4gIDxUYWJzIGxhYmVsPVwiU2VjdGlvbnNcIiB0YWJzPXtbeyBpZDogJ2EnLCBsYWJlbDogJ092ZXJ2aWV3JyB9LCB7IGlkOiAnYicsIGxhYmVsOiAnRGV0YWlscycgfV19IGJpbmQ6dmFsdWU9e3RhYn0+XG4gICAgeyNpZiB0YWIgPT09ICdhJ308cD5PdmVydmlldyBwYW5lbDwvcD57OmVsc2V9PHA+RGV0YWlscyBwYW5lbDwvcD57L2lmfVxuICA8L1RhYnM+XG4gIDxUYWJsZSBjYXB0aW9uPVwiU2NvcmVzXCIgcm93cz17cm93c30gY29sdW1ucz17Y29sc30gaGVpZ2h0PXs0MDB9IC8+XG4gIDxUb2FzdCBmcm9tPXt0b2FzdH0gbXM9ezUwfSAvPlxuPC9tYWluPlxuIl0sIm5hbWVzIjpbXSwibWFwcGluZ3MiOiI7Ozs7Ozs7Ozs7Ozs7Ozs7OztBQUdBO0FBRUE7QUFDQTtBQUFBO0FBR0E7QUFDQTtBQUNBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBSUE7QUFDQTtBQUNBO0FBQ0E7QUFDQTtBQUFBO0FBQUE7QUFBQTtBQUVBO0FBQUE7QUFBQTtBQUNBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUVBO0FBQ0E7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7In0=
