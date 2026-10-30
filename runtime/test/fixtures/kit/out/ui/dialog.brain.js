import { sig, fx, tpl, insert, attr, on } from "__RT__/brain.js";
import { budgetEnter as $brainBudgetbudgetEnter, budgetLeave as $brainBudgetbudgetLeave, budgetOwner as $brainBudgetbudgetOwner, budgetStep as $brainBudgetbudgetStep } from "__RT__/brain-budget.js";
import { dev, fragment as $brainBudgetFragment } from "__RT__/brain-dev.js";
import { expose } from "__RT__/brain-ch.js";
import { urlDialog } from "__RT__/brain-url.js";
const $t0 = $brainBudgetFragment(tpl("<dialog data-b=\"n3ohka\"><header><h2><!></h2> <button type=\"button\" class=\"x\">×</button></header> <div class=\"body\"><!></div> <footer><!></footer></dialog>"), "n3ohka", 0);
let $n = 0;
function Dialog($p = {}) {
  const $brainBudgetOwner = $brainBudgetbudgetOwner();
  // #268: the first instance keeps the ids as written (`/#pricing` works); later ones get `~2`, `~3`…
  const $u = $n++ ? "~" + $n : "";
  const dlg = sig(null, "dlg");
  const open = () => { const $$brainBudgetFrame1 = $brainBudgetbudgetEnter({ file: ".brain/kit/ui/dialog.brain", component: "Dialog", line: 5, column: 16 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame1); {
    if (!dlg.v.open) {
      dlg.v.showModal();
    }
  } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame1); } };
  const close = () => { const $$brainBudgetFrame2 = $brainBudgetbudgetEnter({ file: ".brain/kit/ui/dialog.brain", component: "Dialog", line: 6, column: 17 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame2); {
    if (dlg.v.open) {
      dlg.v.close();
    }
  } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame2); } };
  fx(() => { const $$brainBudgetFrame3 = $brainBudgetbudgetEnter({ file: ".brain/kit/ui/dialog.brain", component: "Dialog", line: 7, column: 6 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame3); {
    if (dlg.v) {
      return urlDialog(dlg.v, ($p.urlKey ?? ''));
    }
  } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame3); } });
  expose($p, { open, close });
  const $r10 = $t0();
  const $e4 = $r10;
  const $e5 = $r10.firstChild.firstChild;
  const $e6 = $e5.firstChild;
  const $e7 = $e5.nextSibling.nextSibling;
  const $e8 = $r10.firstChild.nextSibling.nextSibling.firstChild;
  const $e9 = $r10.firstChild.nextSibling.nextSibling.nextSibling.nextSibling.firstChild;
  dlg.v = $e4;
  attr($e4, "aria-labelledby", "dlg-title" + $u);
  on($e4, "close", () => { $p["on:close"]?.(); });
  attr($e5, "id", "dlg-title" + $u);
  insert(null, () => $p.title, $e6);
  attr($e7, "aria-label", () => ($p.closeLabel ?? 'Close'));
  on($e7, "click", close);
  insert(null, $p.children ? $p.children() : null, $e8);
  insert(null, $p["slot:footer"] ? $p["slot:footer"]() : null, $e9);
  return $r10;
}
Dialog.h = "n3ohka";
Dialog.$hmr = { version: 1, wiring: "import { sig, fx, tpl, insert, attr, on } from \"__RT__/brain.js\";\nimport { budgetEnter as $brainBudgetbudgetEnter, budgetLeave as $brainBudgetbudgetLeave, budgetOwner as $brainBudgetbudgetOwner, budgetStep as $brainBudgetbudgetStep } from \"__RT__/brain-budget.js\";\nimport { dev, fragment as $brainBudgetFragment } from \"__RT__/brain-dev.js\";\nimport { expose } from \"__RT__/brain-ch.js\";\nimport { urlDialog } from \"__RT__/brain-url.js\";\nconst $t0 = $brainBudgetFragment(tpl(\"__brain_static_template__\"), \"n3ohka\", 0);\nlet $n = 0;\nfunction Dialog($p = {}) {\n  const $brainBudgetOwner = $brainBudgetbudgetOwner();\n  // #268: the first instance keeps the ids as written (`/#pricing` works); later ones get `~2`, `~3`…\n  const $u = $n++ ? \"~\" + $n : \"\";\n  const dlg = sig(null, \"dlg\");\n  const open = () => { const $$brainBudgetFrame1 = $brainBudgetbudgetEnter({ file: \".brain/kit/ui/dialog.brain\", component: \"Dialog\", line: 5, column: 16 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame1); {\n    if (!dlg.v.open) {\n      dlg.v.showModal();\n    }\n  } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame1); } };\n  const close = () => { const $$brainBudgetFrame2 = $brainBudgetbudgetEnter({ file: \".brain/kit/ui/dialog.brain\", component: \"Dialog\", line: 6, column: 17 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame2); {\n    if (dlg.v.open) {\n      dlg.v.close();\n    }\n  } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame2); } };\n  fx(() => { const $$brainBudgetFrame3 = $brainBudgetbudgetEnter({ file: \".brain/kit/ui/dialog.brain\", component: \"Dialog\", line: 7, column: 6 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame3); {\n    if (dlg.v) {\n      return urlDialog(dlg.v, ($p.urlKey ?? ''));\n    }\n  } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame3); } });\n  expose($p, { open, close });\n  const $r10 = $t0();\n  const $e4 = $r10;\n  const $e5 = $r10.firstChild.firstChild;\n  const $e6 = $e5.firstChild;\n  const $e7 = $e5.nextSibling.nextSibling;\n  const $e8 = $r10.firstChild.nextSibling.nextSibling.firstChild;\n  const $e9 = $r10.firstChild.nextSibling.nextSibling.nextSibling.nextSibling.firstChild;\n  dlg.v = $e4;\n  attr($e4, \"aria-labelledby\", \"dlg-title\" + $u);\n  on($e4, \"close\", () => { $p[\"on:close\"]?.(); });\n  attr($e5, \"id\", \"dlg-title\" + $u);\n  insert(null, () => $p.title, $e6);\n  attr($e7, \"aria-label\", () => ($p.closeLabel ?? 'Close'));\n  on($e7, \"click\", close);\n  insert(null, $p.children ? $p.children() : null, $e8);\n  insert(null, $p[\"slot:footer\"] ? $p[\"slot:footer\"]() : null, $e9);\n  return $r10;\n}\nDialog.h = \"n3ohka\";\nexport default dev(Dialog);\n", templates: ["<dialog data-b=\"n3ohka\"><header><h2><!></h2> <button type=\"button\" class=\"x\">×</button></header> <div class=\"body\"><!></div> <footer><!></footer></dialog>"] };
export default dev(Dialog);
//# sourceMappingURL=data:application/json;base64,eyJ2ZXJzaW9uIjozLCJmaWxlIjoiZGlhbG9nLmJyYWluLmpzIiwic291cmNlcyI6WyJkaWFsb2cuYnJhaW4iXSwic291cmNlc0NvbnRlbnQiOlsiPG1ldGEgcHJvcHM9XCJ0aXRsZTpzdHIgY2xvc2VMYWJlbDpzdHI9J0Nsb3NlJyB1cmxLZXk6c3RyPScnXCIgZW1pdHM9XCJjbG9zZVwiIGV4cG9zZT1cIm9wZW4gY2xvc2VcIj5cbjxzY3JpcHQ+XG4gIGxldCBkbGcgPSByZWYoKVxuICAvLyBuYXRpdmUgbW9kYWwgPGRpYWxvZz46IGZvY3VzIG1vdmVzIGluc2lkZSwgdGhlIHJlc3Qgb2YgdGhlIHBhZ2UgaXMgaW5lcnQsIEVzY2FwZSBjbG9zZXNcbiAgY29uc3Qgb3BlbiA9ICgpID0+IHsgaWYgKCFkbGcub3BlbikgZGxnLnNob3dNb2RhbCgpIH1cbiAgY29uc3QgY2xvc2UgPSAoKSA9PiB7IGlmIChkbGcub3BlbikgZGxnLmNsb3NlKCkgfVxuICBmeCgoKSA9PiB7IGlmIChkbGcpIHJldHVybiB1cmxEaWFsb2coZGxnLCB1cmxLZXkpIH0pXG48L3NjcmlwdD5cbjxkaWFsb2cgYmluZDpyZWY9e2RsZ30gYXJpYS1sYWJlbGxlZGJ5PVwiZGxnLXRpdGxlXCIgb246Y2xvc2U9e2VtaXQoJ2Nsb3NlJyl9PlxuICA8aGVhZGVyPlxuICAgIDxoMiBpZD1cImRsZy10aXRsZVwiPnt0aXRsZX08L2gyPlxuICAgIDxidXR0b24gdHlwZT1cImJ1dHRvblwiIGNsYXNzPVwieFwiIGFyaWEtbGFiZWw9e2Nsb3NlTGFiZWx9IG9uOmNsaWNrPXtjbG9zZX0+w5c8L2J1dHRvbj5cbiAgPC9oZWFkZXI+XG4gIDxkaXYgY2xhc3M9XCJib2R5XCI+PHNsb3QgLz48L2Rpdj5cbiAgPGZvb3Rlcj48c2xvdCBuYW1lPVwiZm9vdGVyXCIgLz48L2Zvb3Rlcj5cbjwvZGlhbG9nPlxuPHN0eWxlPlxuICBkaWFsb2cge1xuICAgIHdpZHRoOiBtaW4oNTIwcHgsIGNhbGMoMTAwdncgLSAzMnB4KSk7IHBhZGRpbmc6IDA7IGJvcmRlcjogMXB4IHNvbGlkIHZhcigtLWdsYXNzLWVkZ2UpO1xuICAgIGJvcmRlci1yYWRpdXM6IHZhcigtLXJhZGl1cy1sZyk7IGNvbG9yOiB2YXIoLS1mZyk7IGJhY2tncm91bmQ6IHZhcigtLWdsYXNzKTsgYm94LXNoYWRvdzogdmFyKC0tc2hhZG93KTtcblxuICB9XG4gIGRpYWxvZzo6YmFja2Ryb3AgeyBiYWNrZ3JvdW5kOiB2YXIoLS1zY3JpbSk7IH1cbiAgaGVhZGVyIHsgZGlzcGxheTogZmxleDsgYWxpZ24taXRlbXM6IGNlbnRlcjsganVzdGlmeS1jb250ZW50OiBzcGFjZS1iZXR3ZWVuOyBwYWRkaW5nOiB2YXIoLS1zcGFjZS00KSB2YXIoLS1zcGFjZS00KSAwOyB9XG4gIGgyIHsgZm9udDogNjUwIDE4cHgvMS4zIHZhcigtLWZvbnQpOyBtYXJnaW46IDA7IH1cbiAgLnggeyBmb250LXNpemU6IDIycHg7IGxpbmUtaGVpZ2h0OiAxOyB3aWR0aDogMzZweDsgaGVpZ2h0OiAzNnB4OyBib3JkZXI6IDA7IGJvcmRlci1yYWRpdXM6IHZhcigtLXJhZGl1cy1zbSk7IGNvbG9yOiB2YXIoLS1tdXRlZCk7IGJhY2tncm91bmQ6IHZhcigtLXN1cmZhY2UpOyBjdXJzb3I6IHBvaW50ZXI7IH1cbiAgLmJvZHkgeyBwYWRkaW5nOiB2YXIoLS1zcGFjZS00KTsgfVxuICBmb290ZXIgeyBkaXNwbGF5OiBmbGV4OyBqdXN0aWZ5LWNvbnRlbnQ6IGZsZXgtZW5kOyBnYXA6IHZhcigtLXNwYWNlLTIpOyBwYWRkaW5nOiAwIHZhcigtLXNwYWNlLTQpIHZhcigtLXNwYWNlLTQpOyB9XG4gIGZvb3RlcjplbXB0eSB7IGRpc3BsYXk6IG5vbmU7IH1cbiAgQHN1cHBvcnRzIChiYWNrZHJvcC1maWx0ZXI6IGJsdXIoMXB4KSkgeyBkaWFsb2cgeyBiYWNrZHJvcC1maWx0ZXI6IGJsdXIoMjRweCkgc2F0dXJhdGUoMS40KTsgfSBkaWFsb2c6OmJhY2tkcm9wIHsgYmFja2Ryb3AtZmlsdGVyOiBibHVyKDRweCk7IH0gfVxuPC9zdHlsZT5cbiJdLCJuYW1lcyI6W10sIm1hcHBpbmdzIjoiOzs7Ozs7Ozs7OztBQUVBO0FBRUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUNBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFDQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUVBO0FBQUE7QUFBQTtBQUVBO0FBQUE7QUFDQTtBQUFBO0FBRUE7QUFDQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7In0=

