import { sig, fx, tpl, insert, attr, on, list } from "__RT__/brain.js";
import { budgetEnter as $brainBudgetbudgetEnter, budgetLeave as $brainBudgetbudgetLeave, budgetOwner as $brainBudgetbudgetOwner, budgetStep as $brainBudgetbudgetStep } from "__RT__/brain-budget.js";
import { dev, fragment as $brainBudgetFragment } from "__RT__/brain-dev.js";
const $t0 = $brainBudgetFragment(tpl("<button type=\"button\" role=\"menuitem\" tabindex=\"-1\"><!></button>"), "gapqim", 0);
const $t1 = $brainBudgetFragment(tpl("<div data-b=\"gapqim\" class=\"menu\"><button type=\"button\" aria-haspopup=\"menu\"><!></button> <div class=\"pop\" role=\"menu\"><!></div></div>"), "gapqim", 1);
function Menu($p = {}) {
  const $brainBudgetOwner = $brainBudgetbudgetOwner();
  const open = sig(false, "open");
  const pop = sig(null, "pop");
  const btn = sig(null, "btn");
  const enabled = () => { const $$brainBudgetFrame1 = $brainBudgetbudgetEnter({ file: ".brain/kit/ui/menu.brain", component: "Menu", line: 7, column: 19 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame1); { return [...pop.v.querySelectorAll('[role=menuitem]:not([disabled])')]; } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame1); } };
  const choose = (it) => { const $$brainBudgetFrame2 = $brainBudgetbudgetEnter({ file: ".brain/kit/ui/menu.brain", component: "Menu", line: 8, column: 18 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame2); {
    open.v = false;
    btn.v.focus();
    $p["on:select"]?.(it);
  } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame2); } };
  const keys = (e) => { const $$brainBudgetFrame3 = $brainBudgetbudgetEnter({ file: ".brain/kit/ui/menu.brain", component: "Menu", line: 13, column: 16 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame3); {
    const all = enabled();
    const i = all.indexOf(e.target);
    if (e.key === 'Escape') {
      open.v = false;
      btn.v.focus();
    } else {
      if (e.key === 'ArrowDown' && all.length) {
        e.preventDefault();
        all[(i + 1) % all.length].focus();
      } else {
        if (e.key === 'ArrowUp' && all.length) {
          e.preventDefault();
          all[(i - 1 + all.length) % all.length].focus();
        }
      }
    }
  } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame3); } };
  fx(() => { const $$brainBudgetFrame4 = $brainBudgetbudgetEnter({ file: ".brain/kit/ui/menu.brain", component: "Menu", line: 27, column: 6 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame4); {
    if (!open.v || !pop.v) {
      return;
    }
    const h = requestAnimationFrame(() => { const $$brainBudgetFrame5 = $brainBudgetbudgetEnter({ file: ".brain/kit/ui/menu.brain", component: "Menu", line: 30, column: 37 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame5); {
      const all = enabled();
      if (all.length) {
        all[0].focus();
      }
    } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame5); } });
    const out = (e) => { const $$brainBudgetFrame6 = $brainBudgetbudgetEnter({ file: ".brain/kit/ui/menu.brain", component: "Menu", line: 34, column: 17 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame6); {
      if (!pop.v.contains(e.target) && !btn.v.contains(e.target)) {
        open.v = false;
      }
    } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame6); } };
    pop.v.ownerDocument.addEventListener('pointerdown', out);
    return () => { const $$brainBudgetFrame7 = $brainBudgetbudgetEnter({ file: ".brain/kit/ui/menu.brain", component: "Menu", line: 38, column: 12 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame7); {
      cancelAnimationFrame(h);
      pop.v.ownerDocument.removeEventListener('pointerdown', out);
    } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame7); } };
  } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame4); } });
  const $r15 = $t1();
  const $e8 = $r15.firstChild;
  const $e9 = $e8.firstChild;
  const $e10 = $e8.nextSibling.nextSibling;
  const $e11 = $e10.firstChild;
  btn.v = $e8;
  attr($e8, "aria-expanded", () => open.v ? 'true' : 'false');
  on($e8, "click", () => { open.v = !open.v; });
  insert(null, () => $p.label, $e9);
  attr($e10, "aria-label", () => $p.label);
  pop.v = $e10;
  attr($e10, "hidden", () => !open.v);
  on($e10, "keydown", keys);
  list(null, () => $p.items, (it) => it.id, (it) => {
    const $r14 = $t0();
    const $e12 = $r14;
    const $e13 = $r14.firstChild;
    attr($e12, "disabled", () => it.v.disabled);
    on($e12, "click", () => { choose(it.v); });
    insert(null, () => it.v.label, $e13);
    return $r14;
  }, $e11);
  return $r15;
}
Menu.h = "gapqim";
Menu.$hmr = { version: 1, wiring: "import { sig, fx, tpl, insert, attr, on, list } from \"__RT__/brain.js\";\nimport { budgetEnter as $brainBudgetbudgetEnter, budgetLeave as $brainBudgetbudgetLeave, budgetOwner as $brainBudgetbudgetOwner, budgetStep as $brainBudgetbudgetStep } from \"__RT__/brain-budget.js\";\nimport { dev, fragment as $brainBudgetFragment } from \"__RT__/brain-dev.js\";\nconst $t0 = $brainBudgetFragment(tpl(\"__brain_static_template__\"), \"gapqim\", 0);\nconst $t1 = $brainBudgetFragment(tpl(\"__brain_static_template__\"), \"gapqim\", 1);\nfunction Menu($p = {}) {\n  const $brainBudgetOwner = $brainBudgetbudgetOwner();\n  const open = sig(false, \"open\");\n  const pop = sig(null, \"pop\");\n  const btn = sig(null, \"btn\");\n  const enabled = () => { const $$brainBudgetFrame1 = $brainBudgetbudgetEnter({ file: \".brain/kit/ui/menu.brain\", component: \"Menu\", line: 7, column: 19 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame1); { return [...pop.v.querySelectorAll('[role=menuitem]:not([disabled])')]; } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame1); } };\n  const choose = (it) => { const $$brainBudgetFrame2 = $brainBudgetbudgetEnter({ file: \".brain/kit/ui/menu.brain\", component: \"Menu\", line: 8, column: 18 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame2); {\n    open.v = false;\n    btn.v.focus();\n    $p[\"on:select\"]?.(it);\n  } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame2); } };\n  const keys = (e) => { const $$brainBudgetFrame3 = $brainBudgetbudgetEnter({ file: \".brain/kit/ui/menu.brain\", component: \"Menu\", line: 13, column: 16 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame3); {\n    const all = enabled();\n    const i = all.indexOf(e.target);\n    if (e.key === 'Escape') {\n      open.v = false;\n      btn.v.focus();\n    } else {\n      if (e.key === 'ArrowDown' && all.length) {\n        e.preventDefault();\n        all[(i + 1) % all.length].focus();\n      } else {\n        if (e.key === 'ArrowUp' && all.length) {\n          e.preventDefault();\n          all[(i - 1 + all.length) % all.length].focus();\n        }\n      }\n    }\n  } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame3); } };\n  fx(() => { const $$brainBudgetFrame4 = $brainBudgetbudgetEnter({ file: \".brain/kit/ui/menu.brain\", component: \"Menu\", line: 27, column: 6 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame4); {\n    if (!open.v || !pop.v) {\n      return;\n    }\n    const h = requestAnimationFrame(() => { const $$brainBudgetFrame5 = $brainBudgetbudgetEnter({ file: \".brain/kit/ui/menu.brain\", component: \"Menu\", line: 30, column: 37 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame5); {\n      const all = enabled();\n      if (all.length) {\n        all[0].focus();\n      }\n    } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame5); } });\n    const out = (e) => { const $$brainBudgetFrame6 = $brainBudgetbudgetEnter({ file: \".brain/kit/ui/menu.brain\", component: \"Menu\", line: 34, column: 17 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame6); {\n      if (!pop.v.contains(e.target) && !btn.v.contains(e.target)) {\n        open.v = false;\n      }\n    } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame6); } };\n    pop.v.ownerDocument.addEventListener('pointerdown', out);\n    return () => { const $$brainBudgetFrame7 = $brainBudgetbudgetEnter({ file: \".brain/kit/ui/menu.brain\", component: \"Menu\", line: 38, column: 12 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame7); {\n      cancelAnimationFrame(h);\n      pop.v.ownerDocument.removeEventListener('pointerdown', out);\n    } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame7); } };\n  } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame4); } });\n  const $r15 = $t1();\n  const $e8 = $r15.firstChild;\n  const $e9 = $e8.firstChild;\n  const $e10 = $e8.nextSibling.nextSibling;\n  const $e11 = $e10.firstChild;\n  btn.v = $e8;\n  attr($e8, \"aria-expanded\", () => open.v ? 'true' : 'false');\n  on($e8, \"click\", () => { open.v = !open.v; });\n  insert(null, () => $p.label, $e9);\n  attr($e10, \"aria-label\", () => $p.label);\n  pop.v = $e10;\n  attr($e10, \"hidden\", () => !open.v);\n  on($e10, \"keydown\", keys);\n  list(null, () => $p.items, (it) => it.id, (it) => {\n    const $r14 = $t0();\n    const $e12 = $r14;\n    const $e13 = $r14.firstChild;\n    attr($e12, \"disabled\", () => it.v.disabled);\n    on($e12, \"click\", () => { choose(it.v); });\n    insert(null, () => it.v.label, $e13);\n    return $r14;\n  }, $e11);\n  return $r15;\n}\nMenu.h = \"gapqim\";\nexport default dev(Menu);\n", templates: ["<button type=\"button\" role=\"menuitem\" tabindex=\"-1\"><!></button>","<div data-b=\"gapqim\" class=\"menu\"><button type=\"button\" aria-haspopup=\"menu\"><!></button> <div class=\"pop\" role=\"menu\"><!></div></div>"] };
export default dev(Menu);
//# sourceMappingURL=data:application/json;base64,eyJ2ZXJzaW9uIjozLCJmaWxlIjoibWVudS5icmFpbi5qcyIsInNvdXJjZXMiOlsibWVudS5icmFpbiJdLCJzb3VyY2VzQ29udGVudCI6WyI8bWV0YSBwcm9wcz1cImxhYmVsOnN0ciBpdGVtczpsaXN0XCIgZW1pdHM9XCJzZWxlY3Q6b2JqXCI+XG48c2NyaXB0PlxuICAvLyBpdGVtczogW3sgaWQ6ICdlZGl0JywgbGFiZWw6ICdFZGl0JywgZGlzYWJsZWQ/OiB0cnVlIH1dXG4gIGxldCBvcGVuID0gc2lnKGZhbHNlKVxuICBsZXQgcG9wID0gcmVmKClcbiAgbGV0IGJ0biA9IHJlZigpXG4gIGNvbnN0IGVuYWJsZWQgPSAoKSA9PiBbLi4ucG9wLnF1ZXJ5U2VsZWN0b3JBbGwoJ1tyb2xlPW1lbnVpdGVtXTpub3QoW2Rpc2FibGVkXSknKV1cbiAgY29uc3QgY2hvb3NlID0gKGl0KSA9PiB7XG4gICAgb3BlbiA9IGZhbHNlXG4gICAgYnRuLmZvY3VzKClcbiAgICBlbWl0KCdzZWxlY3QnLCBpdClcbiAgfVxuICBjb25zdCBrZXlzID0gKGUpID0+IHtcbiAgICBjb25zdCBhbGwgPSBlbmFibGVkKClcbiAgICBjb25zdCBpID0gYWxsLmluZGV4T2YoZS50YXJnZXQpXG4gICAgaWYgKGUua2V5ID09PSAnRXNjYXBlJykge1xuICAgICAgb3BlbiA9IGZhbHNlXG4gICAgICBidG4uZm9jdXMoKVxuICAgIH0gZWxzZSBpZiAoZS5rZXkgPT09ICdBcnJvd0Rvd24nICYmIGFsbC5sZW5ndGgpIHtcbiAgICAgIGUucHJldmVudERlZmF1bHQoKVxuICAgICAgYWxsWyhpICsgMSkgJSBhbGwubGVuZ3RoXS5mb2N1cygpXG4gICAgfSBlbHNlIGlmIChlLmtleSA9PT0gJ0Fycm93VXAnICYmIGFsbC5sZW5ndGgpIHtcbiAgICAgIGUucHJldmVudERlZmF1bHQoKVxuICAgICAgYWxsWyhpIC0gMSArIGFsbC5sZW5ndGgpICUgYWxsLmxlbmd0aF0uZm9jdXMoKVxuICAgIH1cbiAgfVxuICBmeCgoKSA9PiB7XG4gICAgaWYgKCFvcGVuIHx8ICFwb3ApIHJldHVyblxuICAgIC8vIG5leHQgZnJhbWU6IHRoZSBwb3B1cCBpcyB2aXNpYmxlIChoaWRkZW4gcmVtb3ZlZCkgb25seSBhZnRlciB0aGlzIHVwZGF0ZSBpcyBwYWludGVkXG4gICAgY29uc3QgaCA9IHJlcXVlc3RBbmltYXRpb25GcmFtZSgoKSA9PiB7XG4gICAgICBjb25zdCBhbGwgPSBlbmFibGVkKClcbiAgICAgIGlmIChhbGwubGVuZ3RoKSBhbGxbMF0uZm9jdXMoKVxuICAgIH0pXG4gICAgY29uc3Qgb3V0ID0gKGUpID0+IHtcbiAgICAgIGlmICghcG9wLmNvbnRhaW5zKGUudGFyZ2V0KSAmJiAhYnRuLmNvbnRhaW5zKGUudGFyZ2V0KSkgb3BlbiA9IGZhbHNlXG4gICAgfVxuICAgIHBvcC5vd25lckRvY3VtZW50LmFkZEV2ZW50TGlzdGVuZXIoJ3BvaW50ZXJkb3duJywgb3V0KVxuICAgIHJldHVybiAoKSA9PiB7XG4gICAgICBjYW5jZWxBbmltYXRpb25GcmFtZShoKVxuICAgICAgcG9wLm93bmVyRG9jdW1lbnQucmVtb3ZlRXZlbnRMaXN0ZW5lcigncG9pbnRlcmRvd24nLCBvdXQpXG4gICAgfVxuICB9KVxuPC9zY3JpcHQ+XG48ZGl2IGNsYXNzPVwibWVudVwiPlxuICA8YnV0dG9uIHR5cGU9XCJidXR0b25cIiBiaW5kOnJlZj17YnRufSBhcmlhLWhhc3BvcHVwPVwibWVudVwiIGFyaWEtZXhwYW5kZWQ9e29wZW4gPyAndHJ1ZScgOiAnZmFsc2UnfSBvbjpjbGljaz17b3BlbiA9ICFvcGVufT57bGFiZWx9PC9idXR0b24+XG4gIDxkaXYgY2xhc3M9XCJwb3BcIiByb2xlPVwibWVudVwiIGFyaWEtbGFiZWw9e2xhYmVsfSBiaW5kOnJlZj17cG9wfSBoaWRkZW49eyFvcGVufSBvbjprZXlkb3duPXtrZXlzfT5cbiAgICB7I2VhY2ggaXRlbXMgYXMgaXQgKGl0LmlkKX1cbiAgICAgIDxidXR0b24gdHlwZT1cImJ1dHRvblwiIHJvbGU9XCJtZW51aXRlbVwiIHRhYmluZGV4PVwiLTFcIiBkaXNhYmxlZD17aXQuZGlzYWJsZWR9IG9uOmNsaWNrPXtjaG9vc2UoaXQpfT57aXQubGFiZWx9PC9idXR0b24+XG4gICAgey9lYWNofVxuICA8L2Rpdj5cbjwvZGl2PlxuPHN0eWxlPlxuICAubWVudSB7IHBvc2l0aW9uOiByZWxhdGl2ZTsgZGlzcGxheTogaW5saW5lLWJsb2NrOyB9XG4gIC5tZW51ID4gYnV0dG9uIHsgZm9udDogNjAwIDE0cHgvMS4yIHZhcigtLWZvbnQpOyBwYWRkaW5nOiB2YXIoLS1zcGFjZS0yKSB2YXIoLS1zcGFjZS0zKTsgYm9yZGVyOiAxcHggc29saWQgdmFyKC0tZ2xhc3MtZWRnZSk7IGJvcmRlci1yYWRpdXM6IHZhcigtLXJhZGl1cy1zbSk7IGNvbG9yOiB2YXIoLS1mZyk7IGJhY2tncm91bmQ6IHZhcigtLWdsYXNzKTsgY3Vyc29yOiBwb2ludGVyOyB9XG4gIC5wb3Age1xuICAgIHBvc2l0aW9uOiBhYnNvbHV0ZTsgdG9wOiBjYWxjKDEwMCUgKyA2cHgpOyBsZWZ0OiAwOyBtaW4td2lkdGg6IDE4MHB4OyBkaXNwbGF5OiBncmlkOyBwYWRkaW5nOiB2YXIoLS1zcGFjZS0xKTsgei1pbmRleDogNTA7XG4gICAgYm9yZGVyOiAxcHggc29saWQgdmFyKC0tZ2xhc3MtZWRnZSk7IGJvcmRlci1yYWRpdXM6IHZhcigtLXJhZGl1cyk7IGJhY2tncm91bmQ6IHZhcigtLWdsYXNzKTsgYm94LXNoYWRvdzogdmFyKC0tc2hhZG93KTsgYmFja2Ryb3AtZmlsdGVyOiBibHVyKDIwcHgpIHNhdHVyYXRlKDEuNCk7XG4gIH1cbiAgLnBvcFtoaWRkZW5dIHsgZGlzcGxheTogbm9uZTsgfVxuICBbcm9sZT1tZW51aXRlbV0geyB0ZXh0LWFsaWduOiBsZWZ0OyBmb250OiAxNHB4LzEuMyB2YXIoLS1mb250KTsgcGFkZGluZzogdmFyKC0tc3BhY2UtMikgdmFyKC0tc3BhY2UtMyk7IGJvcmRlcjogMDsgYm9yZGVyLXJhZGl1czogdmFyKC0tcmFkaXVzLXNtKTsgY29sb3I6IHZhcigtLWZnKTsgYmFja2dyb3VuZDogdHJhbnNwYXJlbnQ7IGN1cnNvcjogcG9pbnRlcjsgfVxuICBbcm9sZT1tZW51aXRlbV06aG92ZXIsIFtyb2xlPW1lbnVpdGVtXTpmb2N1cy12aXNpYmxlIHsgYmFja2dyb3VuZDogdmFyKC0tc3VyZmFjZSk7IG91dGxpbmU6IG5vbmU7IH1cbiAgW3JvbGU9bWVudWl0ZW1dOmRpc2FibGVkIHsgY29sb3I6IHZhcigtLW11dGVkKTsgY3Vyc29yOiBub3QtYWxsb3dlZDsgfVxuICBidXR0b246Zm9jdXMtdmlzaWJsZSB7IG91dGxpbmU6IDJweCBzb2xpZCB2YXIoLS1hY2NlbnQpOyBvdXRsaW5lLW9mZnNldDogMnB4OyB9XG48L3N0eWxlPlxuIl0sIm5hbWVzIjpbXSwibWFwcGluZ3MiOiI7Ozs7Ozs7QUFHQTtBQUNBO0FBQ0E7QUFDQTtBQUNBO0FBQ0E7QUFDQTtBQUNBO0FBQUE7QUFFQTtBQUNBO0FBQ0E7QUFDQTtBQUNBO0FBQ0E7QUFBQTtBQUFBO0FBRUE7QUFDQTtBQUFBO0FBQUE7QUFFQTtBQUNBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFHQTtBQUNBO0FBQUE7QUFBQTtBQUVBO0FBQ0E7QUFDQTtBQUFBO0FBQUE7QUFBQTtBQUVBO0FBQ0E7QUFBQTtBQUFBO0FBQUE7QUFFQTtBQUNBO0FBQ0E7QUFDQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBS0E7QUFBQTtBQUFBO0FBQUE7QUFDQTtBQUFBO0FBQUE7QUFBQTtBQUNBO0FBQUE7QUFBQTtBQUFBO0FBQ0E7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7In0=

