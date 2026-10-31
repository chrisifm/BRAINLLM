import { sig, drv, tpl, insert, attr, on, list as $list } from "__RT__/brain.js";
import { budgetEnter as $brainBudgetbudgetEnter, budgetLeave as $brainBudgetbudgetLeave, budgetOwner as $brainBudgetbudgetOwner, budgetStep as $brainBudgetbudgetStep } from "__RT__/brain-budget.js";
import { dev, fragment as $brainBudgetFragment } from "__RT__/brain-dev.js";
const $t0 = $brainBudgetFragment(tpl("<button type=\"button\" role=\"tab\"><!></button>"), "zxv5bo", 0);
const $t1 = $brainBudgetFragment(tpl("<div data-b=\"zxv5bo\" class=\"tabs\"><div class=\"list\" role=\"tablist\"><!></div> <div class=\"panel\" role=\"tabpanel\" tabindex=\"0\"><!></div></div>"), "zxv5bo", 1);
function Tabs($p = {}) {
  const $brainBudgetOwner = $brainBudgetbudgetOwner();
  const list = sig(null, "list");
  const current = drv(() => { const $$brainBudgetFrame1 = $brainBudgetbudgetEnter({ file: ".brain/kit/ui/tabs.brain", component: "Tabs", line: 6, column: 23 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame1); { return $p.tabs.find((t) => { const $$brainBudgetFrame2 = $brainBudgetbudgetEnter({ file: ".brain/kit/ui/tabs.brain", component: "Tabs", line: 6, column: 39 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame2); { return t.id === $p.value; } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame2); } }) || $p.tabs[0]; } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame1); } });
  const keys = (e) => { const $$brainBudgetFrame3 = $brainBudgetbudgetEnter({ file: ".brain/kit/ui/tabs.brain", component: "Tabs", line: 7, column: 16 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame3); {
    const i = $p.tabs.indexOf(current.v);
    let j = -1;
    if (e.key === 'ArrowRight') {
      j = (i + 1) % $p.tabs.length;
    } else {
      if (e.key === 'ArrowLeft') {
        j = (i - 1 + $p.tabs.length) % $p.tabs.length;
      } else {
        if (e.key === 'Home') {
          j = 0;
        } else {
          if (e.key === 'End') {
            j = $p.tabs.length - 1;
          }
        }
      }
    }
    if (j < 0) {
      return;
    }
    e.preventDefault();
    $p["bind:value"]?.($p.tabs[j].id);
    list.v.querySelectorAll('[role=tab]')[j].focus();
  } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame3); } };
  const $r11 = $t1();
  const $e4 = $r11.firstChild;
  const $e5 = $e4.firstChild;
  const $e9 = $e4.nextSibling.nextSibling;
  const $e10 = $e9.firstChild;
  attr($e4, "aria-label", () => $p.label);
  list.v = $e4;
  on($e4, "keydown", keys);
  $list(null, () => $p.tabs, (t) => t.id, (t) => {
    const $r8 = $t0();
    const $e6 = $r8;
    const $e7 = $r8.firstChild;
    attr($e6, "aria-selected", () => t.v === current.v ? 'true' : 'false');
    attr($e6, "tabindex", () => t.v === current.v ? 0 : -1);
    on($e6, "click", () => { $p["bind:value"]?.(t.v.id); });
    insert(null, () => t.v.label, $e7);
    return $r8;
  }, $e5);
  attr($e9, "aria-label", () => current.v ? current.v.label : $p.label);
  insert(null, $p.children ? $p.children() : null, $e10);
  return $r11;
}
Tabs.h = "zxv5bo";
Tabs.$hmr = { version: 1, wiring: "import { sig, drv, tpl, insert, attr, on, list as $list } from \"__RT__/brain.js\";\nimport { budgetEnter as $brainBudgetbudgetEnter, budgetLeave as $brainBudgetbudgetLeave, budgetOwner as $brainBudgetbudgetOwner, budgetStep as $brainBudgetbudgetStep } from \"__RT__/brain-budget.js\";\nimport { dev, fragment as $brainBudgetFragment } from \"__RT__/brain-dev.js\";\nconst $t0 = $brainBudgetFragment(tpl(\"__brain_static_template__\"), \"zxv5bo\", 0);\nconst $t1 = $brainBudgetFragment(tpl(\"__brain_static_template__\"), \"zxv5bo\", 1);\nfunction Tabs($p = {}) {\n  const $brainBudgetOwner = $brainBudgetbudgetOwner();\n  const list = sig(null, \"list\");\n  const current = drv(() => { const $$brainBudgetFrame1 = $brainBudgetbudgetEnter({ file: \".brain/kit/ui/tabs.brain\", component: \"Tabs\", line: 6, column: 23 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame1); { return $p.tabs.find((t) => { const $$brainBudgetFrame2 = $brainBudgetbudgetEnter({ file: \".brain/kit/ui/tabs.brain\", component: \"Tabs\", line: 6, column: 39 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame2); { return t.id === $p.value; } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame2); } }) || $p.tabs[0]; } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame1); } });\n  const keys = (e) => { const $$brainBudgetFrame3 = $brainBudgetbudgetEnter({ file: \".brain/kit/ui/tabs.brain\", component: \"Tabs\", line: 7, column: 16 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame3); {\n    const i = $p.tabs.indexOf(current.v);\n    let j = -1;\n    if (e.key === 'ArrowRight') {\n      j = (i + 1) % $p.tabs.length;\n    } else {\n      if (e.key === 'ArrowLeft') {\n        j = (i - 1 + $p.tabs.length) % $p.tabs.length;\n      } else {\n        if (e.key === 'Home') {\n          j = 0;\n        } else {\n          if (e.key === 'End') {\n            j = $p.tabs.length - 1;\n          }\n        }\n      }\n    }\n    if (j < 0) {\n      return;\n    }\n    e.preventDefault();\n    $p[\"bind:value\"]?.($p.tabs[j].id);\n    list.v.querySelectorAll('[role=tab]')[j].focus();\n  } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame3); } };\n  const $r11 = $t1();\n  const $e4 = $r11.firstChild;\n  const $e5 = $e4.firstChild;\n  const $e9 = $e4.nextSibling.nextSibling;\n  const $e10 = $e9.firstChild;\n  attr($e4, \"aria-label\", () => $p.label);\n  list.v = $e4;\n  on($e4, \"keydown\", keys);\n  $list(null, () => $p.tabs, (t) => t.id, (t) => {\n    const $r8 = $t0();\n    const $e6 = $r8;\n    const $e7 = $r8.firstChild;\n    attr($e6, \"aria-selected\", () => t.v === current.v ? 'true' : 'false');\n    attr($e6, \"tabindex\", () => t.v === current.v ? 0 : -1);\n    on($e6, \"click\", () => { $p[\"bind:value\"]?.(t.v.id); });\n    insert(null, () => t.v.label, $e7);\n    return $r8;\n  }, $e5);\n  attr($e9, \"aria-label\", () => current.v ? current.v.label : $p.label);\n  insert(null, $p.children ? $p.children() : null, $e10);\n  return $r11;\n}\nTabs.h = \"zxv5bo\";\nexport default dev(Tabs);\n", templates: ["<button type=\"button\" role=\"tab\"><!></button>","<div data-b=\"zxv5bo\" class=\"tabs\"><div class=\"list\" role=\"tablist\"><!></div> <div class=\"panel\" role=\"tabpanel\" tabindex=\"0\"><!></div></div>"] };
export default dev(Tabs);
//# sourceMappingURL=data:application/json;base64,eyJ2ZXJzaW9uIjozLCJmaWxlIjoidGFicy5icmFpbi5qcyIsInNvdXJjZXMiOlsidGFicy5icmFpbiJdLCJzb3VyY2VzQ29udGVudCI6WyI8bWV0YSBwcm9wcz1cInRhYnM6bGlzdCB2YWx1ZTpzdHJ+IGxhYmVsOnN0clwiPlxuPHNjcmlwdD5cbiAgLy8gdGFiczogW3sgaWQ6ICdhJywgbGFiZWw6ICdPdmVydmlldycgfV0gwrcgdGhlIHBhcmVudCByZW5kZXJzIHRoZSBwYW5lbCBvZiBgdmFsdWVgIGluIHRoZSBzbG90OyBhbiB1bmtub3duIGB2YWx1ZWBcbiAgLy8gc2VsZWN0cyB0aGUgZmlyc3QgdGFiXG4gIGxldCBsaXN0ID0gcmVmKClcbiAgY29uc3QgY3VycmVudCA9IGRydigoKSA9PiB0YWJzLmZpbmQoKHQpID0+IHQuaWQgPT09IHZhbHVlKSB8fCB0YWJzWzBdKVxuICBjb25zdCBrZXlzID0gKGUpID0+IHtcbiAgICBjb25zdCBpID0gdGFicy5pbmRleE9mKGN1cnJlbnQpXG4gICAgbGV0IGogPSAtMVxuICAgIGlmIChlLmtleSA9PT0gJ0Fycm93UmlnaHQnKSBqID0gKGkgKyAxKSAlIHRhYnMubGVuZ3RoXG4gICAgZWxzZSBpZiAoZS5rZXkgPT09ICdBcnJvd0xlZnQnKSBqID0gKGkgLSAxICsgdGFicy5sZW5ndGgpICUgdGFicy5sZW5ndGhcbiAgICBlbHNlIGlmIChlLmtleSA9PT0gJ0hvbWUnKSBqID0gMFxuICAgIGVsc2UgaWYgKGUua2V5ID09PSAnRW5kJykgaiA9IHRhYnMubGVuZ3RoIC0gMVxuICAgIGlmIChqIDwgMCkgcmV0dXJuXG4gICAgZS5wcmV2ZW50RGVmYXVsdCgpXG4gICAgdmFsdWUgPSB0YWJzW2pdLmlkXG4gICAgbGlzdC5xdWVyeVNlbGVjdG9yQWxsKCdbcm9sZT10YWJdJylbal0uZm9jdXMoKVxuICB9XG48L3NjcmlwdD5cbjxkaXYgY2xhc3M9XCJ0YWJzXCI+XG4gIDxkaXYgY2xhc3M9XCJsaXN0XCIgcm9sZT1cInRhYmxpc3RcIiBhcmlhLWxhYmVsPXtsYWJlbH0gYmluZDpyZWY9e2xpc3R9IG9uOmtleWRvd249e2tleXN9PlxuICAgIHsjZWFjaCB0YWJzIGFzIHQgKHQuaWQpfVxuICAgICAgPGJ1dHRvbiB0eXBlPVwiYnV0dG9uXCIgcm9sZT1cInRhYlwiIGFyaWEtc2VsZWN0ZWQ9e3QgPT09IGN1cnJlbnQgPyAndHJ1ZScgOiAnZmFsc2UnfSB0YWJpbmRleD17dCA9PT0gY3VycmVudCA/IDAgOiAtMX0gb246Y2xpY2s9e3ZhbHVlID0gdC5pZH0+e3QubGFiZWx9PC9idXR0b24+XG4gICAgey9lYWNofVxuICA8L2Rpdj5cbiAgPGRpdiBjbGFzcz1cInBhbmVsXCIgcm9sZT1cInRhYnBhbmVsXCIgYXJpYS1sYWJlbD17Y3VycmVudCA/IGN1cnJlbnQubGFiZWwgOiBsYWJlbH0gdGFiaW5kZXg9XCIwXCI+PHNsb3QgLz48L2Rpdj5cbjwvZGl2PlxuPHN0eWxlPlxuICAubGlzdCB7IGRpc3BsYXk6IGZsZXg7IGdhcDogdmFyKC0tc3BhY2UtMSk7IHBhZGRpbmc6IHZhcigtLXNwYWNlLTEpOyBib3JkZXItcmFkaXVzOiB2YXIoLS1yYWRpdXMpOyBiYWNrZ3JvdW5kOiB2YXIoLS1nbGFzcyk7IGJvcmRlcjogMXB4IHNvbGlkIHZhcigtLWdsYXNzLWVkZ2UpOyB3aWR0aDogZml0LWNvbnRlbnQ7IH1cbiAgW3JvbGU9dGFiXSB7IGZvbnQ6IDYwMCAxNHB4LzEuMiB2YXIoLS1mb250KTsgcGFkZGluZzogdmFyKC0tc3BhY2UtMikgdmFyKC0tc3BhY2UtMyk7IGJvcmRlcjogMDsgYm9yZGVyLXJhZGl1czogdmFyKC0tcmFkaXVzLXNtKTsgY29sb3I6IHZhcigtLW11dGVkKTsgYmFja2dyb3VuZDogdHJhbnNwYXJlbnQ7IGN1cnNvcjogcG9pbnRlcjsgfVxuICBbcm9sZT10YWJdW2FyaWEtc2VsZWN0ZWQ9J3RydWUnXSB7IGNvbG9yOiB2YXIoLS1mZyk7IGJhY2tncm91bmQ6IHZhcigtLXN1cmZhY2UpOyBib3gtc2hhZG93OiB2YXIoLS1zaGFkb3cpOyB9XG4gIFtyb2xlPXRhYl06Zm9jdXMtdmlzaWJsZSwgLnBhbmVsOmZvY3VzLXZpc2libGUgeyBvdXRsaW5lOiAycHggc29saWQgdmFyKC0tYWNjZW50KTsgb3V0bGluZS1vZmZzZXQ6IDJweDsgfVxuICAucGFuZWwgeyBwYWRkaW5nOiB2YXIoLS1zcGFjZS00KSAwOyB9XG48L3N0eWxlPlxuIl0sIm5hbWVzIjpbXSwibWFwcGluZ3MiOiI7Ozs7Ozs7QUFJQTtBQUNBO0FBQ0E7QUFDQTtBQUNBO0FBQ0E7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBSUE7QUFBQTtBQUFBO0FBQ0E7QUFDQTtBQUNBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBSUE7QUFBQTtBQUFBO0FBQ0E7QUFBQTtBQUFBO0FBQUE7QUFDQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFHQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTsifQ==

