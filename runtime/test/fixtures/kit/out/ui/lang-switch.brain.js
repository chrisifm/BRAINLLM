import { tpl, insert, attr, on, list, when } from "__RT__/brain.js";
import { budgetEnter as $brainBudgetbudgetEnter, budgetLeave as $brainBudgetbudgetLeave, budgetOwner as $brainBudgetbudgetOwner, budgetStep as $brainBudgetbudgetStep } from "__RT__/brain-budget.js";
import { dev, fragment as $brainBudgetFragment } from "__RT__/brain-dev.js";
import { alternates, suggest, dismiss } from "__RT__/brain-i18n.js";
const $t0 = $brainBudgetFragment(tpl("<li><a><!></a></li>"), "2arc9w", 0);
const $t1 = $brainBudgetFragment(tpl("<p class=\"hint\"><a><!></a> <button type=\"button\" aria-label=\"Dismiss\">×</button></p>"), "2arc9w", 1);
const $t2 = $brainBudgetFragment(tpl("<nav data-b=\"2arc9w\" class=\"lang\"><ul><!></ul> <!></nav>"), "2arc9w", 2);
const $t3 = $brainBudgetFragment(tpl("<!><!>"), "2arc9w", 3);
function LangSwitch($p = {}) {
  const $brainBudgetOwner = $brainBudgetbudgetOwner();
  const langs = alternates();
  const hint = suggest();
  const close = (e) => { const $$brainBudgetFrame1 = $brainBudgetbudgetEnter({ file: ".brain/kit/ui/lang-switch.brain", component: "LangSwitch", line: 8, column: 17 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame1); {
    dismiss();
    e.target.parentNode.setAttribute('hidden', '');
  } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame1); } };
  const $r15 = $t3();
  const $e2 = $r15.firstChild;
  when(null, () => langs.length, () => {
    const $r14 = $t2();
    const $e3 = $r14;
    const $e4 = $r14.firstChild.firstChild;
    const $e8 = $r14.firstChild.nextSibling.nextSibling;
    attr($e3, "aria-label", () => ($p.label ?? 'Language'));
    list(null, () => langs, (l) => l.locale, (l) => {
      const $r7 = $t0();
      const $e5 = $r7.firstChild;
      const $e6 = $e5.firstChild;
      attr($e5, "href", () => l.v.href);
      attr($e5, "hreflang", () => l.v.locale);
      attr($e5, "lang", () => l.v.locale);
      attr($e5, "aria-current", () => l.v.current ? 'page' : null);
      insert(null, () => l.v.name, $e6);
      return $r7;
    }, $e4);
    when(null, () => hint, () => {
      const $r13 = $t1();
      const $e9 = $r13;
      const $e10 = $r13.firstChild;
      const $e11 = $e10.firstChild;
      const $e12 = $e10.nextSibling.nextSibling;
      attr($e9, "lang", hint.locale);
      attr($e10, "href", hint.href);
      attr($e10, "hreflang", hint.locale);
      on($e10, "click", () => { dismiss(); });
      insert(null, hint.name, $e11);
      on($e12, "click", close);
      return $r13;
    }, null, $e8);
    return $r14;
  }, null, $e2);
  return $r15;
}
LangSwitch.h = "2arc9w";
LangSwitch.$hmr = { version: 1, wiring: "import { tpl, insert, attr, on, list, when } from \"__RT__/brain.js\";\nimport { budgetEnter as $brainBudgetbudgetEnter, budgetLeave as $brainBudgetbudgetLeave, budgetOwner as $brainBudgetbudgetOwner, budgetStep as $brainBudgetbudgetStep } from \"__RT__/brain-budget.js\";\nimport { dev, fragment as $brainBudgetFragment } from \"__RT__/brain-dev.js\";\nimport { alternates, suggest, dismiss } from \"__RT__/brain-i18n.js\";\nconst $t0 = $brainBudgetFragment(tpl(\"__brain_static_template__\"), \"2arc9w\", 0);\nconst $t1 = $brainBudgetFragment(tpl(\"__brain_static_template__\"), \"2arc9w\", 1);\nconst $t2 = $brainBudgetFragment(tpl(\"__brain_static_template__\"), \"2arc9w\", 2);\nconst $t3 = $brainBudgetFragment(tpl(\"__brain_static_template__\"), \"2arc9w\", 3);\nfunction LangSwitch($p = {}) {\n  const $brainBudgetOwner = $brainBudgetbudgetOwner();\n  const langs = alternates();\n  const hint = suggest();\n  const close = (e) => { const $$brainBudgetFrame1 = $brainBudgetbudgetEnter({ file: \".brain/kit/ui/lang-switch.brain\", component: \"LangSwitch\", line: 8, column: 17 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame1); {\n    dismiss();\n    e.target.parentNode.setAttribute('hidden', '');\n  } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame1); } };\n  const $r15 = $t3();\n  const $e2 = $r15.firstChild;\n  when(null, () => langs.length, () => {\n    const $r14 = $t2();\n    const $e3 = $r14;\n    const $e4 = $r14.firstChild.firstChild;\n    const $e8 = $r14.firstChild.nextSibling.nextSibling;\n    attr($e3, \"aria-label\", () => ($p.label ?? 'Language'));\n    list(null, () => langs, (l) => l.locale, (l) => {\n      const $r7 = $t0();\n      const $e5 = $r7.firstChild;\n      const $e6 = $e5.firstChild;\n      attr($e5, \"href\", () => l.v.href);\n      attr($e5, \"hreflang\", () => l.v.locale);\n      attr($e5, \"lang\", () => l.v.locale);\n      attr($e5, \"aria-current\", () => l.v.current ? 'page' : null);\n      insert(null, () => l.v.name, $e6);\n      return $r7;\n    }, $e4);\n    when(null, () => hint, () => {\n      const $r13 = $t1();\n      const $e9 = $r13;\n      const $e10 = $r13.firstChild;\n      const $e11 = $e10.firstChild;\n      const $e12 = $e10.nextSibling.nextSibling;\n      attr($e9, \"lang\", hint.locale);\n      attr($e10, \"href\", hint.href);\n      attr($e10, \"hreflang\", hint.locale);\n      on($e10, \"click\", () => { dismiss(); });\n      insert(null, hint.name, $e11);\n      on($e12, \"click\", close);\n      return $r13;\n    }, null, $e8);\n    return $r14;\n  }, null, $e2);\n  return $r15;\n}\nLangSwitch.h = \"2arc9w\";\nexport default dev(LangSwitch);\n", templates: ["<li><a><!></a></li>","<p class=\"hint\"><a><!></a> <button type=\"button\" aria-label=\"Dismiss\">×</button></p>","<nav data-b=\"2arc9w\" class=\"lang\"><ul><!></ul> <!></nav>","<!><!>"] };
export default dev(LangSwitch);
//# sourceMappingURL=data:application/json;base64,eyJ2ZXJzaW9uIjozLCJmaWxlIjoibGFuZy1zd2l0Y2guYnJhaW4uanMiLCJzb3VyY2VzIjpbImxhbmctc3dpdGNoLmJyYWluIl0sInNvdXJjZXNDb250ZW50IjpbIjxtZXRhIHByb3BzPVwibGFiZWw6c3RyPSdMYW5ndWFnZSdcIj5cbjxzY3JpcHQ+XG4gIC8vIGxpbmtzIHRvIHRoaXMgc2FtZSBwYWdlIGluIGV2ZXJ5IGxhbmd1YWdlIG9mIHRoZSBzaXRlIChub25lIHdpdGggYSBzaW5nbGUgbGFuZ3VhZ2UpOyBlYWNoIG5hbWUgaW4gaXRzZWxmXG4gIGNvbnN0IGxhbmdzID0gYWx0ZXJuYXRlcygpXG4gIC8vIHRoZSBicm93c2VyIHByZWZlcnMgYW5vdGhlciBsYW5ndWFnZSB0aGUgc2l0ZSBoYXM6IG9mZmVyZWQgb25jZSwgbmV2ZXIgYW4gYXV0b21hdGljIHJlZGlyZWN0LiBBIHVpLyBjb21wb25lbnQgaGFzXG4gIC8vIG5vIHN0YXRlIChCMDExNik6IGNsb3NpbmcgaGlkZXMgdGhlIGhpbnQncyBvd24gZWxlbWVudCBhbmQgcmVtZW1iZXJzIHRoZSBhbnN3ZXIuXG4gIGNvbnN0IGhpbnQgPSBzdWdnZXN0KClcbiAgY29uc3QgY2xvc2UgPSAoZSkgPT4ge1xuICAgIGRpc21pc3MoKVxuICAgIGUudGFyZ2V0LnBhcmVudE5vZGUuc2V0QXR0cmlidXRlKCdoaWRkZW4nLCAnJylcbiAgfVxuPC9zY3JpcHQ+XG57I2lmIGxhbmdzLmxlbmd0aH1cbiAgPG5hdiBjbGFzcz1cImxhbmdcIiBhcmlhLWxhYmVsPXtsYWJlbH0+XG4gICAgPHVsPlxuICAgICAgeyNlYWNoIGxhbmdzIGFzIGwgKGwubG9jYWxlKX1cbiAgICAgICAgPGxpPjxhIGhyZWY9e2wuaHJlZn0gaHJlZmxhbmc9e2wubG9jYWxlfSBsYW5nPXtsLmxvY2FsZX0gYXJpYS1jdXJyZW50PXtsLmN1cnJlbnQgPyAncGFnZScgOiBudWxsfT57bC5uYW1lfTwvYT48L2xpPlxuICAgICAgey9lYWNofVxuICAgIDwvdWw+XG4gICAgeyNpZiBoaW50fVxuICAgICAgPHAgY2xhc3M9XCJoaW50XCIgbGFuZz17aGludC5sb2NhbGV9PlxuICAgICAgICA8YSBocmVmPXtoaW50LmhyZWZ9IGhyZWZsYW5nPXtoaW50LmxvY2FsZX0gb246Y2xpY2s9e2Rpc21pc3MoKX0+e2hpbnQubmFtZX08L2E+XG4gICAgICAgIDxidXR0b24gdHlwZT1cImJ1dHRvblwiIGFyaWEtbGFiZWw9XCJEaXNtaXNzXCIgb246Y2xpY2s9e2Nsb3NlfT7DlzwvYnV0dG9uPlxuICAgICAgPC9wPlxuICAgIHsvaWZ9XG4gIDwvbmF2Plxuey9pZn1cbjxzdHlsZT5cbiAgLmxhbmcgeyBkaXNwbGF5OiBmbGV4OyBhbGlnbi1pdGVtczogY2VudGVyOyBnYXA6IHZhcigtLXNwYWNlLTMpOyBmb250OiAxNHB4LzEuMyB2YXIoLS1mb250KTsgfVxuICB1bCB7IGRpc3BsYXk6IGZsZXg7IGdhcDogdmFyKC0tc3BhY2UtMSk7IGxpc3Qtc3R5bGU6IG5vbmU7IG1hcmdpbjogMDsgcGFkZGluZzogMDsgfVxuICBhIHsgY29sb3I6IHZhcigtLW11dGVkKTsgdGV4dC1kZWNvcmF0aW9uOiBub25lOyBwYWRkaW5nOiB2YXIoLS1zcGFjZS0xKSB2YXIoLS1zcGFjZS0yKTsgYm9yZGVyLXJhZGl1czogdmFyKC0tcmFkaXVzLXNtKTsgfVxuICBhW2FyaWEtY3VycmVudD0ncGFnZSddIHsgY29sb3I6IHZhcigtLWZnKTsgYmFja2dyb3VuZDogdmFyKC0tc3VyZmFjZSk7IH1cbiAgYTpmb2N1cy12aXNpYmxlLCBidXR0b246Zm9jdXMtdmlzaWJsZSB7IG91dGxpbmU6IDJweCBzb2xpZCB2YXIoLS1hY2NlbnQpOyBvdXRsaW5lLW9mZnNldDogMnB4OyB9XG4gIC5oaW50IHsgZGlzcGxheTogZmxleDsgYWxpZ24taXRlbXM6IGNlbnRlcjsgZ2FwOiB2YXIoLS1zcGFjZS0xKTsgbWFyZ2luOiAwOyBwYWRkaW5nOiB2YXIoLS1zcGFjZS0xKSB2YXIoLS1zcGFjZS0yKTsgYm9yZGVyOiAxcHggc29saWQgdmFyKC0tZ2xhc3MtZWRnZSk7IGJvcmRlci1yYWRpdXM6IHZhcigtLXJhZGl1cy1zbSk7IGJhY2tncm91bmQ6IHZhcigtLWdsYXNzKTsgfVxuICAuaGludCBhIHsgY29sb3I6IHZhcigtLWFjY2VudCk7IH1cbiAgLmhpbnRbaGlkZGVuXSB7IGRpc3BsYXk6IG5vbmU7IH0gLyogY2xvc2VkOiBkaXNwbGF5OmZsZXggYWJvdmUgd291bGQgb3RoZXJ3aXNlIHdpbiBvdmVyIFtoaWRkZW5dICovXG4gIGJ1dHRvbiB7IGJvcmRlcjogMDsgYmFja2dyb3VuZDogdHJhbnNwYXJlbnQ7IGNvbG9yOiB2YXIoLS1tdXRlZCk7IGZvbnQtc2l6ZTogMTZweDsgY3Vyc29yOiBwb2ludGVyOyB9XG48L3N0eWxlPlxuIl0sIm5hbWVzIjpbXSwibWFwcGluZ3MiOiI7Ozs7Ozs7Ozs7QUFHQTtBQUdBO0FBQ0E7QUFDQTtBQUNBO0FBQUE7QUFBQTtBQUFBO0FBR0E7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUNBO0FBRUE7QUFBQTtBQUFBO0FBQUE7QUFDQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUdBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUNBO0FBQ0E7QUFBQTtBQUFBO0FBQUE7QUFDQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTsifQ==

