import { tpl, insert, attr, on } from "__RT__/brain.js";
import { dev, fragment as $brainBudgetFragment } from "__RT__/brain-dev.js";
import Field from "./field.brain.js";
const $t0 = $brainBudgetFragment(tpl("<input data-b=\"9n61wx\">"), "9n61wx", 0);
const $t1 = $brainBudgetFragment(tpl("<!><!>"), "9n61wx", 1);
function Input($p = {}) {
  const $r4 = $t1();
  const $e1 = $r4.firstChild;
  insert(null, Field({ get "label"() { return $p.label; }, get "error"() { return $p.error; }, get "help"() { return $p.help; }, children: () => {
      const $r3 = $t0();
      const $e2 = $r3;
      attr($e2, "type", () => ($p.type ?? 'text'));
      attr($e2, "step", () => ($p.type ?? 'text') === 'number' ? ($p.step ?? 'any') : null);
      attr($e2, "name", () => $p.name);
      attr($e2, "placeholder", () => $p.placeholder);
      attr($e2, "autocomplete", () => $p.autocomplete);
      attr($e2, "required", () => ($p.required ?? false));
      attr($e2, "disabled", () => ($p.disabled ?? false));
      attr($e2, "aria-label", () => $p.label);
      attr($e2, "value", () => $p.value);
      on($e2, "input", (e) => { $p["bind:value"]?.(e.target.value); });
      return $r3;
    } }), $e1);
  return $r4;
}
Input.h = "9n61wx";
Input.$hmr = { version: 1, wiring: "import { tpl, insert, attr, on } from \"__RT__/brain.js\";\nimport { dev, fragment as $brainBudgetFragment } from \"__RT__/brain-dev.js\";\nimport Field from \"__RT__/ui/field.brain\";\nconst $t0 = $brainBudgetFragment(tpl(\"__brain_static_template__\"), \"9n61wx\", 0);\nconst $t1 = $brainBudgetFragment(tpl(\"__brain_static_template__\"), \"9n61wx\", 1);\nfunction Input($p = {}) {\n  const $r4 = $t1();\n  const $e1 = $r4.firstChild;\n  insert(null, Field({ get \"label\"() { return $p.label; }, get \"error\"() { return $p.error; }, get \"help\"() { return $p.help; }, children: () => {\n      const $r3 = $t0();\n      const $e2 = $r3;\n      attr($e2, \"type\", () => ($p.type ?? 'text'));\n      attr($e2, \"step\", () => ($p.type ?? 'text') === 'number' ? ($p.step ?? 'any') : null);\n      attr($e2, \"name\", () => $p.name);\n      attr($e2, \"placeholder\", () => $p.placeholder);\n      attr($e2, \"autocomplete\", () => $p.autocomplete);\n      attr($e2, \"required\", () => ($p.required ?? false));\n      attr($e2, \"disabled\", () => ($p.disabled ?? false));\n      attr($e2, \"aria-label\", () => $p.label);\n      attr($e2, \"value\", () => $p.value);\n      on($e2, \"input\", (e) => { $p[\"bind:value\"]?.(e.target.value); });\n      return $r3;\n    } }), $e1);\n  return $r4;\n}\nInput.h = \"9n61wx\";\nexport default dev(Input);\n", templates: ["<input data-b=\"9n61wx\">","<!><!>"] };
export default dev(Input);
//# sourceMappingURL=data:application/json;base64,eyJ2ZXJzaW9uIjozLCJmaWxlIjoiaW5wdXQuYnJhaW4uanMiLCJzb3VyY2VzIjpbImlucHV0LmJyYWluIl0sInNvdXJjZXNDb250ZW50IjpbIjxtZXRhIHByb3BzPVwibGFiZWw6c3RyIHZhbHVlOnN0cn4gdHlwZTpzdHI9J3RleHQnIHN0ZXA6c3RyPSdhbnknIG5hbWU6c3RyPyBwbGFjZWhvbGRlcjpzdHI/IGF1dG9jb21wbGV0ZTpzdHI/IHJlcXVpcmVkOmJvb2w9ZmFsc2UgZGlzYWJsZWQ6Ym9vbD1mYWxzZSBlcnJvcjpzdHI/IGhlbHA6c3RyP1wiPlxuPEZpZWxkIGxhYmVsPXtsYWJlbH0gZXJyb3I9e2Vycm9yfSBoZWxwPXtoZWxwfT5cbiAgPGlucHV0XG4gICAgdHlwZT17dHlwZX1cbiAgICBzdGVwPXt0eXBlID09PSAnbnVtYmVyJyA/IHN0ZXAgOiBudWxsfVxuICAgIG5hbWU9e25hbWV9XG4gICAgcGxhY2Vob2xkZXI9e3BsYWNlaG9sZGVyfVxuICAgIGF1dG9jb21wbGV0ZT17YXV0b2NvbXBsZXRlfVxuICAgIHJlcXVpcmVkPXtyZXF1aXJlZH1cbiAgICBkaXNhYmxlZD17ZGlzYWJsZWR9XG4gICAgYXJpYS1sYWJlbD17bGFiZWx9XG4gICAgYmluZDp2YWx1ZT17dmFsdWV9XG4gID5cbjwvRmllbGQ+XG48c3R5bGU+XG4gIGlucHV0IHtcbiAgICB3aWR0aDogMTAwJTsgbWluLWhlaWdodDogNDRweDsgcGFkZGluZzogdmFyKC0tc3BhY2UtMikgdmFyKC0tc3BhY2UtMyk7XG4gICAgZm9udDogMTZweC8xLjQgdmFyKC0tZm9udCk7IGNvbG9yOiB2YXIoLS1mZyk7IGJhY2tncm91bmQ6IHZhcigtLXN1cmZhY2UpO1xuICAgIGJvcmRlcjogMXB4IHNvbGlkIHZhcigtLWxpbmUpOyBib3JkZXItcmFkaXVzOiB2YXIoLS1yYWRpdXMtc20pO1xuICB9XG4gIGlucHV0OmZvY3VzLXZpc2libGUgeyBvdXRsaW5lOiAycHggc29saWQgdmFyKC0tYWNjZW50KTsgb3V0bGluZS1vZmZzZXQ6IDFweDsgfVxuICBpbnB1dFthcmlhLWludmFsaWQ9J3RydWUnXSB7IGJvcmRlci1jb2xvcjogdmFyKC0tYmFkKTsgfVxuICBpbnB1dDpkaXNhYmxlZCB7IGNvbG9yOiB2YXIoLS1tdXRlZCk7IH1cbjwvc3R5bGU+XG4iXSwibmFtZXMiOltdLCJtYXBwaW5ncyI6Ijs7Ozs7Ozs7QUFDQTtBQUFBO0FBQUE7QUFDQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBOyJ9

