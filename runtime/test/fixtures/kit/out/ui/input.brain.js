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
Input.$hmr = { version: 1, wiring: "import { tpl, insert, attr, on } from \"__RT__/brain.js\";\nimport { dev, fragment as $brainBudgetFragment } from \"__RT__/brain-dev.js\";\nimport Field from \"__RT__/ui/field.brain\";\nconst $t0 = $brainBudgetFragment(tpl(\"__brain_static_template__\"), \"9n61wx\", 0);\nconst $t1 = $brainBudgetFragment(tpl(\"__brain_static_template__\"), \"9n61wx\", 1);\nfunction Input($p = {}) {\n  const $r4 = $t1();\n  const $e1 = $r4.firstChild;\n  insert(null, Field({ get \"label\"() { return $p.label; }, get \"error\"() { return $p.error; }, get \"help\"() { return $p.help; }, children: () => {\n      const $r3 = $t0();\n      const $e2 = $r3;\n      attr($e2, \"type\", () => ($p.type ?? 'text'));\n      attr($e2, \"name\", () => $p.name);\n      attr($e2, \"placeholder\", () => $p.placeholder);\n      attr($e2, \"autocomplete\", () => $p.autocomplete);\n      attr($e2, \"required\", () => ($p.required ?? false));\n      attr($e2, \"disabled\", () => ($p.disabled ?? false));\n      attr($e2, \"aria-label\", () => $p.label);\n      attr($e2, \"value\", () => $p.value);\n      on($e2, \"input\", (e) => { $p[\"bind:value\"]?.(e.target.value); });\n      return $r3;\n    } }), $e1);\n  return $r4;\n}\nInput.h = \"9n61wx\";\nexport default dev(Input);\n", templates: ["<input data-b=\"9n61wx\">","<!><!>"] };
export default dev(Input);
//# sourceMappingURL=data:application/json;base64,eyJ2ZXJzaW9uIjozLCJmaWxlIjoiaW5wdXQuYnJhaW4uanMiLCJzb3VyY2VzIjpbImlucHV0LmJyYWluIl0sInNvdXJjZXNDb250ZW50IjpbIjxtZXRhIHByb3BzPVwibGFiZWw6c3RyIHZhbHVlOnN0cn4gdHlwZTpzdHI9J3RleHQnIG5hbWU6c3RyPyBwbGFjZWhvbGRlcjpzdHI/IGF1dG9jb21wbGV0ZTpzdHI/IHJlcXVpcmVkOmJvb2w9ZmFsc2UgZGlzYWJsZWQ6Ym9vbD1mYWxzZSBlcnJvcjpzdHI/IGhlbHA6c3RyP1wiPlxuPEZpZWxkIGxhYmVsPXtsYWJlbH0gZXJyb3I9e2Vycm9yfSBoZWxwPXtoZWxwfT5cbiAgPGlucHV0XG4gICAgdHlwZT17dHlwZX1cbiAgICBuYW1lPXtuYW1lfVxuICAgIHBsYWNlaG9sZGVyPXtwbGFjZWhvbGRlcn1cbiAgICBhdXRvY29tcGxldGU9e2F1dG9jb21wbGV0ZX1cbiAgICByZXF1aXJlZD17cmVxdWlyZWR9XG4gICAgZGlzYWJsZWQ9e2Rpc2FibGVkfVxuICAgIGFyaWEtbGFiZWw9e2xhYmVsfVxuICAgIGJpbmQ6dmFsdWU9e3ZhbHVlfVxuICA+XG48L0ZpZWxkPlxuPHN0eWxlPlxuICBpbnB1dCB7XG4gICAgd2lkdGg6IDEwMCU7IG1pbi1oZWlnaHQ6IDQ0cHg7IHBhZGRpbmc6IHZhcigtLXNwYWNlLTIpIHZhcigtLXNwYWNlLTMpO1xuICAgIGZvbnQ6IDE2cHgvMS40IHZhcigtLWZvbnQpOyBjb2xvcjogdmFyKC0tZmcpOyBiYWNrZ3JvdW5kOiB2YXIoLS1zdXJmYWNlKTtcbiAgICBib3JkZXI6IDFweCBzb2xpZCB2YXIoLS1saW5lKTsgYm9yZGVyLXJhZGl1czogdmFyKC0tcmFkaXVzLXNtKTtcbiAgfVxuICBpbnB1dDpmb2N1cy12aXNpYmxlIHsgb3V0bGluZTogMnB4IHNvbGlkIHZhcigtLWFjY2VudCk7IG91dGxpbmUtb2Zmc2V0OiAxcHg7IH1cbiAgaW5wdXRbYXJpYS1pbnZhbGlkPSd0cnVlJ10geyBib3JkZXItY29sb3I6IHZhcigtLWJhZCk7IH1cbiAgaW5wdXQ6ZGlzYWJsZWQgeyBjb2xvcjogdmFyKC0tbXV0ZWQpOyB9XG48L3N0eWxlPlxuIl0sIm5hbWVzIjpbXSwibWFwcGluZ3MiOiI7Ozs7Ozs7O0FBQ0E7QUFBQTtBQUFBO0FBQ0E7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7In0=

