import { sig, drv, tpl, insert, attr, cls, on, list, when } from "__RT__/brain.js";
import { budgetEnter as $brainBudgetbudgetEnter, budgetLeave as $brainBudgetbudgetLeave, budgetOwner as $brainBudgetbudgetOwner, budgetStep as $brainBudgetbudgetStep } from "__RT__/brain-budget.js";
import { dev, fragment as $brainBudgetFragment } from "__RT__/brain-dev.js";
const $t0 = $brainBudgetFragment(tpl("<button type=\"button\"><!> <span aria-hidden=\"true\"><!></span></button>"), "5pxkt9", 0);
const $t1 = $brainBudgetFragment(tpl("<!><!>"), "5pxkt9", 1);
const $t2 = $brainBudgetFragment(tpl("<th scope=\"col\"><!></th>"), "5pxkt9", 2);
const $t3 = $brainBudgetFragment(tpl("<tr><td class=\"msg bad\" role=\"alert\"><!></td></tr>"), "5pxkt9", 3);
const $t4 = $brainBudgetFragment(tpl("<tr><td class=\"msg\">Loading…</td></tr>"), "5pxkt9", 4);
const $t5 = $brainBudgetFragment(tpl("<tr class=\"pad\" aria-hidden=\"true\"></tr>"), "5pxkt9", 5);
const $t6 = $brainBudgetFragment(tpl("<td><!></td>"), "5pxkt9", 6);
const $t7 = $brainBudgetFragment(tpl("<tr><!></tr>"), "5pxkt9", 7);
const $t8 = $brainBudgetFragment(tpl("<tr><td class=\"msg\"><!></td></tr>"), "5pxkt9", 8);
const $t9 = $brainBudgetFragment(tpl("<!> <!> <!>"), "5pxkt9", 9);
const $t10 = $brainBudgetFragment(tpl("<div data-b=\"5pxkt9\" class=\"wrap\"><table><caption><!></caption> <thead><tr><!></tr></thead> <tbody><!></tbody></table></div>"), "5pxkt9", 10);
function Table($p = {}) {
  const $brainBudgetOwner = $brainBudgetbudgetOwner();
  const VIRTUAL_FROM = 200;
  const OVERSCAN = 10;
  const sortKey = sig('', "sortKey");
  const dir = sig(1, "dir");
  const top = sig(0, "top");
  const sorted = drv(() => { const $$brainBudgetFrame1 = $brainBudgetbudgetEnter({ file: ".brain/kit/ui/table.brain", component: "Table", line: 9, column: 22 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame1); {
    if (!sortKey.v) {
      return $p.rows;
    }
    const k = sortKey.v;
    const d = dir.v;
    return [...$p.rows].sort((a, b) => { const $$brainBudgetFrame2 = $brainBudgetbudgetEnter({ file: ".brain/kit/ui/table.brain", component: "Table", line: 13, column: 27 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame2); { return (a[k] > b[k] ? d : a[k] < b[k] ? -d : 0); } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame2); } });
  } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame1); } });
  const virtual = drv(() => { const $$brainBudgetFrame3 = $brainBudgetbudgetEnter({ file: ".brain/kit/ui/table.brain", component: "Table", line: 15, column: 23 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame3); { return sorted.v.length > VIRTUAL_FROM; } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame3); } });
  const first = drv(() => { const $$brainBudgetFrame4 = $brainBudgetbudgetEnter({ file: ".brain/kit/ui/table.brain", component: "Table", line: 16, column: 21 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame4); { return (virtual.v ? Math.max(0, Math.floor(top.v / ($p.rowHeight ?? 40)) - OVERSCAN) : 0); } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame4); } });
  const last = drv(() => { const $$brainBudgetFrame5 = $brainBudgetbudgetEnter({ file: ".brain/kit/ui/table.brain", component: "Table", line: 17, column: 20 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame5); { return (virtual.v ? Math.min(sorted.v.length, first.v + Math.ceil(($p.height ?? 480) / ($p.rowHeight ?? 40)) + 2 * OVERSCAN) : sorted.v.length); } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame5); } });
  const visible = drv(() => { const $$brainBudgetFrame6 = $brainBudgetbudgetEnter({ file: ".brain/kit/ui/table.brain", component: "Table", line: 18, column: 23 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame6); { return sorted.v.slice(first.v, last.v); } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame6); } });
  const sortBy = (k) => { const $$brainBudgetFrame7 = $brainBudgetbudgetEnter({ file: ".brain/kit/ui/table.brain", component: "Table", line: 19, column: 18 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame7); {
    if (sortKey.v === k) {
      dir.v = -dir.v;
    } else {
      sortKey.v = k;
      dir.v = 1;
    }
  } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame7); } };
  const ariaSort = (k) => { const $$brainBudgetFrame8 = $brainBudgetbudgetEnter({ file: ".brain/kit/ui/table.brain", component: "Table", line: 23, column: 20 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame8); { return (sortKey.v === k ? (dir.v === 1 ? 'ascending' : 'descending') : 'none'); } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame8); } };
  const arrow = (k) => { const $$brainBudgetFrame9 = $brainBudgetbudgetEnter({ file: ".brain/kit/ui/table.brain", component: "Table", line: 24, column: 17 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame9); { return (sortKey.v === k ? (dir.v === 1 ? '▲' : '▼') : ''); } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame9); } };
  const onScroll = (e) => { const $$brainBudgetFrame10 = $brainBudgetbudgetEnter({ file: ".brain/kit/ui/table.brain", component: "Table", line: 25, column: 20 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame10); {
    top.v = e.target.scrollTop;
  } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame10); } };
  const $r47 = $t10();
  const $e11 = $r47;
  const $e12 = $r47.firstChild;
  const $e13 = $e12.firstChild.firstChild;
  const $e14 = $e12.firstChild.nextSibling.nextSibling.firstChild.firstChild;
  const $e24 = $e12.firstChild.nextSibling.nextSibling.nextSibling.nextSibling.firstChild;
  cls($e11, "virtual", () => virtual.v);
  attr($e11, "style:max-height", () => virtual.v ? ($p.height ?? 480) + 'px' : null);
  on($e11, "scroll", onScroll);
  attr($e12, "aria-busy", () => ($p.loading ?? false) ? 'true' : null);
  attr($e12, "aria-rowcount", () => sorted.v.length + 1);
  insert(null, () => $p.caption, $e13);
  list(null, () => $p.columns, (c) => c.key, (c) => {
    const $r23 = $t2();
    const $e15 = $r23;
    const $e16 = $r23.firstChild;
    attr($e15, "aria-sort", () => ariaSort(c.v.key));
    when(null, () => ($p.sortable ?? true) && c.v.sort !== false, () => {
      const $r20 = $t0();
      const $e17 = $r20;
      const $e18 = $r20.firstChild;
      const $e19 = $e18.nextSibling.nextSibling.firstChild;
      on($e17, "click", () => { sortBy(c.v.key); });
      insert(null, () => c.v.label, $e18);
      insert(null, () => arrow(c.v.key), $e19);
      return $r20;
    }, () => {
      const $r22 = $t1();
      const $e21 = $r22.firstChild;
      insert(null, () => c.v.label, $e21);
      return $r22;
    }, $e16);
    return $r23;
  }, $e14);
  when(null, () => $p.error, () => {
    const $r27 = $t3();
    const $e25 = $r27.firstChild;
    const $e26 = $e25.firstChild;
    attr($e25, "colspan", () => $p.columns.length);
    insert(null, () => $p.error, $e26);
    return $r27;
  }, () => {
    const $r28 = $t1();
    when(null, () => ($p.loading ?? false), () => {
      const $r30 = $t4();
      const $e29 = $r30.firstChild;
      attr($e29, "colspan", () => $p.columns.length);
      return $r30;
    }, () => {
      const $r46 = $t9();
      const $e31 = $r46.firstChild;
      const $e34 = $e31.nextSibling.nextSibling;
      const $e43 = $e34.nextSibling.nextSibling;
      when(null, () => first.v > 0, () => {
        const $r33 = $t5();
        const $e32 = $r33;
        attr($e32, "style:height", () => first.v * ($p.rowHeight ?? 40) + 'px');
        return $r33;
      }, null, $e31);
      list(null, () => visible.v, (r) => r[($p.rowKey ?? 'id')], (r) => {
        const $r39 = $t7();
        const $e35 = $r39;
        const $e36 = $r39.firstChild;
        attr($e35, "style:height", () => ($p.rowHeight ?? 40) + 'px');
        list(null, () => $p.columns, (c) => c.key, (c) => {
          const $r38 = $t6();
          const $e37 = $r38.firstChild;
          insert(null, () => r.v[c.v.key], $e37);
          return $r38;
        }, $e36);
        return $r39;
      }, $e34, () => {
        const $r42 = $t8();
        const $e40 = $r42.firstChild;
        const $e41 = $e40.firstChild;
        attr($e40, "colspan", () => $p.columns.length);
        insert(null, () => ($p.empty ?? 'No data yet.'), $e41);
        return $r42;
      });
      when(null, () => last.v < sorted.v.length, () => {
        const $r45 = $t5();
        const $e44 = $r45;
        attr($e44, "style:height", () => (sorted.v.length - last.v) * ($p.rowHeight ?? 40) + 'px');
        return $r45;
      }, null, $e43);
      return $r46;
    }, $r28.firstChild);
    return $r28;
  }, $e24);
  return $r47;
}
Table.h = "5pxkt9";
Table.$hmr = { version: 1, wiring: "import { sig, drv, tpl, insert, attr, cls, on, list, when } from \"__RT__/brain.js\";\nimport { budgetEnter as $brainBudgetbudgetEnter, budgetLeave as $brainBudgetbudgetLeave, budgetOwner as $brainBudgetbudgetOwner, budgetStep as $brainBudgetbudgetStep } from \"__RT__/brain-budget.js\";\nimport { dev, fragment as $brainBudgetFragment } from \"__RT__/brain-dev.js\";\nconst $t0 = $brainBudgetFragment(tpl(\"__brain_static_template__\"), \"5pxkt9\", 0);\nconst $t1 = $brainBudgetFragment(tpl(\"__brain_static_template__\"), \"5pxkt9\", 1);\nconst $t2 = $brainBudgetFragment(tpl(\"__brain_static_template__\"), \"5pxkt9\", 2);\nconst $t3 = $brainBudgetFragment(tpl(\"__brain_static_template__\"), \"5pxkt9\", 3);\nconst $t4 = $brainBudgetFragment(tpl(\"__brain_static_template__\"), \"5pxkt9\", 4);\nconst $t5 = $brainBudgetFragment(tpl(\"__brain_static_template__\"), \"5pxkt9\", 5);\nconst $t6 = $brainBudgetFragment(tpl(\"__brain_static_template__\"), \"5pxkt9\", 6);\nconst $t7 = $brainBudgetFragment(tpl(\"__brain_static_template__\"), \"5pxkt9\", 7);\nconst $t8 = $brainBudgetFragment(tpl(\"__brain_static_template__\"), \"5pxkt9\", 8);\nconst $t9 = $brainBudgetFragment(tpl(\"__brain_static_template__\"), \"5pxkt9\", 9);\nconst $t10 = $brainBudgetFragment(tpl(\"__brain_static_template__\"), \"5pxkt9\", 10);\nfunction Table($p = {}) {\n  const $brainBudgetOwner = $brainBudgetbudgetOwner();\n  const VIRTUAL_FROM = 200;\n  const OVERSCAN = 10;\n  const sortKey = sig('', \"sortKey\");\n  const dir = sig(1, \"dir\");\n  const top = sig(0, \"top\");\n  const sorted = drv(() => { const $$brainBudgetFrame1 = $brainBudgetbudgetEnter({ file: \".brain/kit/ui/table.brain\", component: \"Table\", line: 9, column: 22 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame1); {\n    if (!sortKey.v) {\n      return $p.rows;\n    }\n    const k = sortKey.v;\n    const d = dir.v;\n    return [...$p.rows].sort((a, b) => { const $$brainBudgetFrame2 = $brainBudgetbudgetEnter({ file: \".brain/kit/ui/table.brain\", component: \"Table\", line: 13, column: 27 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame2); { return (a[k] > b[k] ? d : a[k] < b[k] ? -d : 0); } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame2); } });\n  } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame1); } });\n  const virtual = drv(() => { const $$brainBudgetFrame3 = $brainBudgetbudgetEnter({ file: \".brain/kit/ui/table.brain\", component: \"Table\", line: 15, column: 23 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame3); { return sorted.v.length > VIRTUAL_FROM; } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame3); } });\n  const first = drv(() => { const $$brainBudgetFrame4 = $brainBudgetbudgetEnter({ file: \".brain/kit/ui/table.brain\", component: \"Table\", line: 16, column: 21 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame4); { return (virtual.v ? Math.max(0, Math.floor(top.v / ($p.rowHeight ?? 40)) - OVERSCAN) : 0); } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame4); } });\n  const last = drv(() => { const $$brainBudgetFrame5 = $brainBudgetbudgetEnter({ file: \".brain/kit/ui/table.brain\", component: \"Table\", line: 17, column: 20 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame5); { return (virtual.v ? Math.min(sorted.v.length, first.v + Math.ceil(($p.height ?? 480) / ($p.rowHeight ?? 40)) + 2 * OVERSCAN) : sorted.v.length); } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame5); } });\n  const visible = drv(() => { const $$brainBudgetFrame6 = $brainBudgetbudgetEnter({ file: \".brain/kit/ui/table.brain\", component: \"Table\", line: 18, column: 23 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame6); { return sorted.v.slice(first.v, last.v); } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame6); } });\n  const sortBy = (k) => { const $$brainBudgetFrame7 = $brainBudgetbudgetEnter({ file: \".brain/kit/ui/table.brain\", component: \"Table\", line: 19, column: 18 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame7); {\n    if (sortKey.v === k) {\n      dir.v = -dir.v;\n    } else {\n      sortKey.v = k;\n      dir.v = 1;\n    }\n  } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame7); } };\n  const ariaSort = (k) => { const $$brainBudgetFrame8 = $brainBudgetbudgetEnter({ file: \".brain/kit/ui/table.brain\", component: \"Table\", line: 23, column: 20 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame8); { return (sortKey.v === k ? (dir.v === 1 ? 'ascending' : 'descending') : 'none'); } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame8); } };\n  const arrow = (k) => { const $$brainBudgetFrame9 = $brainBudgetbudgetEnter({ file: \".brain/kit/ui/table.brain\", component: \"Table\", line: 24, column: 17 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame9); { return (sortKey.v === k ? (dir.v === 1 ? '▲' : '▼') : ''); } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame9); } };\n  const onScroll = (e) => { const $$brainBudgetFrame10 = $brainBudgetbudgetEnter({ file: \".brain/kit/ui/table.brain\", component: \"Table\", line: 25, column: 20 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame10); {\n    top.v = e.target.scrollTop;\n  } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame10); } };\n  const $r47 = $t10();\n  const $e11 = $r47;\n  const $e12 = $r47.firstChild;\n  const $e13 = $e12.firstChild.firstChild;\n  const $e14 = $e12.firstChild.nextSibling.nextSibling.firstChild.firstChild;\n  const $e24 = $e12.firstChild.nextSibling.nextSibling.nextSibling.nextSibling.firstChild;\n  cls($e11, \"virtual\", () => virtual.v);\n  attr($e11, \"style:max-height\", () => virtual.v ? ($p.height ?? 480) + 'px' : null);\n  on($e11, \"scroll\", onScroll);\n  attr($e12, \"aria-busy\", () => ($p.loading ?? false) ? 'true' : null);\n  attr($e12, \"aria-rowcount\", () => sorted.v.length + 1);\n  insert(null, () => $p.caption, $e13);\n  list(null, () => $p.columns, (c) => c.key, (c) => {\n    const $r23 = $t2();\n    const $e15 = $r23;\n    const $e16 = $r23.firstChild;\n    attr($e15, \"aria-sort\", () => ariaSort(c.v.key));\n    when(null, () => ($p.sortable ?? true) && c.v.sort !== false, () => {\n      const $r20 = $t0();\n      const $e17 = $r20;\n      const $e18 = $r20.firstChild;\n      const $e19 = $e18.nextSibling.nextSibling.firstChild;\n      on($e17, \"click\", () => { sortBy(c.v.key); });\n      insert(null, () => c.v.label, $e18);\n      insert(null, () => arrow(c.v.key), $e19);\n      return $r20;\n    }, () => {\n      const $r22 = $t1();\n      const $e21 = $r22.firstChild;\n      insert(null, () => c.v.label, $e21);\n      return $r22;\n    }, $e16);\n    return $r23;\n  }, $e14);\n  when(null, () => $p.error, () => {\n    const $r27 = $t3();\n    const $e25 = $r27.firstChild;\n    const $e26 = $e25.firstChild;\n    attr($e25, \"colspan\", () => $p.columns.length);\n    insert(null, () => $p.error, $e26);\n    return $r27;\n  }, () => {\n    const $r28 = $t1();\n    when(null, () => ($p.loading ?? false), () => {\n      const $r30 = $t4();\n      const $e29 = $r30.firstChild;\n      attr($e29, \"colspan\", () => $p.columns.length);\n      return $r30;\n    }, () => {\n      const $r46 = $t9();\n      const $e31 = $r46.firstChild;\n      const $e34 = $e31.nextSibling.nextSibling;\n      const $e43 = $e34.nextSibling.nextSibling;\n      when(null, () => first.v > 0, () => {\n        const $r33 = $t5();\n        const $e32 = $r33;\n        attr($e32, \"style:height\", () => first.v * ($p.rowHeight ?? 40) + 'px');\n        return $r33;\n      }, null, $e31);\n      list(null, () => visible.v, (r) => r[($p.rowKey ?? 'id')], (r) => {\n        const $r39 = $t7();\n        const $e35 = $r39;\n        const $e36 = $r39.firstChild;\n        attr($e35, \"style:height\", () => ($p.rowHeight ?? 40) + 'px');\n        list(null, () => $p.columns, (c) => c.key, (c) => {\n          const $r38 = $t6();\n          const $e37 = $r38.firstChild;\n          insert(null, () => r.v[c.v.key], $e37);\n          return $r38;\n        }, $e36);\n        return $r39;\n      }, $e34, () => {\n        const $r42 = $t8();\n        const $e40 = $r42.firstChild;\n        const $e41 = $e40.firstChild;\n        attr($e40, \"colspan\", () => $p.columns.length);\n        insert(null, () => ($p.empty ?? 'No data yet.'), $e41);\n        return $r42;\n      });\n      when(null, () => last.v < sorted.v.length, () => {\n        const $r45 = $t5();\n        const $e44 = $r45;\n        attr($e44, \"style:height\", () => (sorted.v.length - last.v) * ($p.rowHeight ?? 40) + 'px');\n        return $r45;\n      }, null, $e43);\n      return $r46;\n    }, $r28.firstChild);\n    return $r28;\n  }, $e24);\n  return $r47;\n}\nTable.h = \"5pxkt9\";\nexport default dev(Table);\n", templates: ["<button type=\"button\"><!> <span aria-hidden=\"true\"><!></span></button>","<!><!>","<th scope=\"col\"><!></th>","<tr><td class=\"msg bad\" role=\"alert\"><!></td></tr>","<tr><td class=\"msg\">Loading…</td></tr>","<tr class=\"pad\" aria-hidden=\"true\"></tr>","<td><!></td>","<tr><!></tr>","<tr><td class=\"msg\"><!></td></tr>","<!> <!> <!>","<div data-b=\"5pxkt9\" class=\"wrap\"><table><caption><!></caption> <thead><tr><!></tr></thead> <tbody><!></tbody></table></div>"] };
export default dev(Table);
//# sourceMappingURL=data:application/json;base64,eyJ2ZXJzaW9uIjozLCJmaWxlIjoidGFibGUuYnJhaW4uanMiLCJzb3VyY2VzIjpbInRhYmxlLmJyYWluIl0sInNvdXJjZXNDb250ZW50IjpbIjxtZXRhIHByb3BzPVwicm93czpsaXN0IGNvbHVtbnM6bGlzdCBjYXB0aW9uOnN0ciByb3dLZXk6c3RyPSdpZCcgc29ydGFibGU6Ym9vbD10cnVlIGxvYWRpbmc6Ym9vbD1mYWxzZSBlcnJvcjpzdHI/IGVtcHR5OnN0cj0nTm8gZGF0YSB5ZXQuJyByb3dIZWlnaHQ6bnVtPTQwIGhlaWdodDpudW09NDgwXCI+XG48c2NyaXB0PlxuICAvLyBjb2x1bW5zOiBbeyBrZXk6ICduYW1lJywgbGFiZWw6ICdOYW1lJywgc29ydD86IGZhbHNlIH1dIMK3IG1vcmUgdGhhbiAyMDAgcm93cyDihpIgb25seSB0aGUgdmlzaWJsZSB3aW5kb3cgaXMgcmVuZGVyZWRcbiAgY29uc3QgVklSVFVBTF9GUk9NID0gMjAwXG4gIGNvbnN0IE9WRVJTQ0FOID0gMTBcbiAgbGV0IHNvcnRLZXkgPSBzaWcoJycpXG4gIGxldCBkaXIgPSBzaWcoMSlcbiAgbGV0IHRvcCA9IHNpZygwKVxuICBjb25zdCBzb3J0ZWQgPSBkcnYoKCkgPT4ge1xuICAgIGlmICghc29ydEtleSkgcmV0dXJuIHJvd3NcbiAgICBjb25zdCBrID0gc29ydEtleVxuICAgIGNvbnN0IGQgPSBkaXJcbiAgICByZXR1cm4gWy4uLnJvd3NdLnNvcnQoKGEsIGIpID0+IChhW2tdID4gYltrXSA/IGQgOiBhW2tdIDwgYltrXSA/IC1kIDogMCkpXG4gIH0pXG4gIGNvbnN0IHZpcnR1YWwgPSBkcnYoKCkgPT4gc29ydGVkLmxlbmd0aCA+IFZJUlRVQUxfRlJPTSlcbiAgY29uc3QgZmlyc3QgPSBkcnYoKCkgPT4gKHZpcnR1YWwgPyBNYXRoLm1heCgwLCBNYXRoLmZsb29yKHRvcCAvIHJvd0hlaWdodCkgLSBPVkVSU0NBTikgOiAwKSlcbiAgY29uc3QgbGFzdCA9IGRydigoKSA9PiAodmlydHVhbCA/IE1hdGgubWluKHNvcnRlZC5sZW5ndGgsIGZpcnN0ICsgTWF0aC5jZWlsKGhlaWdodCAvIHJvd0hlaWdodCkgKyAyICogT1ZFUlNDQU4pIDogc29ydGVkLmxlbmd0aCkpXG4gIGNvbnN0IHZpc2libGUgPSBkcnYoKCkgPT4gc29ydGVkLnNsaWNlKGZpcnN0LCBsYXN0KSlcbiAgY29uc3Qgc29ydEJ5ID0gKGspID0+IHtcbiAgICBpZiAoc29ydEtleSA9PT0gaykgZGlyID0gLWRpclxuICAgIGVsc2UgeyBzb3J0S2V5ID0gazsgZGlyID0gMSB9XG4gIH1cbiAgY29uc3QgYXJpYVNvcnQgPSAoaykgPT4gKHNvcnRLZXkgPT09IGsgPyAoZGlyID09PSAxID8gJ2FzY2VuZGluZycgOiAnZGVzY2VuZGluZycpIDogJ25vbmUnKVxuICBjb25zdCBhcnJvdyA9IChrKSA9PiAoc29ydEtleSA9PT0gayA/IChkaXIgPT09IDEgPyAn4payJyA6ICfilrwnKSA6ICcnKVxuICBjb25zdCBvblNjcm9sbCA9IChlKSA9PiB7IHRvcCA9IGUudGFyZ2V0LnNjcm9sbFRvcCB9XG48L3NjcmlwdD5cbjxkaXYgY2xhc3M9XCJ3cmFwXCIgY2xhc3M6dmlydHVhbD17dmlydHVhbH0gc3R5bGU6bWF4LWhlaWdodD17dmlydHVhbCA/IGhlaWdodCArICdweCcgOiBudWxsfSBvbjpzY3JvbGw9e29uU2Nyb2xsfT5cbiAgPHRhYmxlIGFyaWEtYnVzeT17bG9hZGluZyA/ICd0cnVlJyA6IG51bGx9IGFyaWEtcm93Y291bnQ9e3NvcnRlZC5sZW5ndGggKyAxfT5cbiAgICA8Y2FwdGlvbj57Y2FwdGlvbn08L2NhcHRpb24+XG4gICAgPHRoZWFkPlxuICAgICAgPHRyPlxuICAgICAgICB7I2VhY2ggY29sdW1ucyBhcyBjIChjLmtleSl9XG4gICAgICAgICAgPHRoIHNjb3BlPVwiY29sXCIgYXJpYS1zb3J0PXthcmlhU29ydChjLmtleSl9PlxuICAgICAgICAgICAgeyNpZiBzb3J0YWJsZSAmJiBjLnNvcnQgIT09IGZhbHNlfVxuICAgICAgICAgICAgICA8YnV0dG9uIHR5cGU9XCJidXR0b25cIiBvbjpjbGljaz17c29ydEJ5KGMua2V5KX0+e2MubGFiZWx9IDxzcGFuIGFyaWEtaGlkZGVuPVwidHJ1ZVwiPnthcnJvdyhjLmtleSl9PC9zcGFuPjwvYnV0dG9uPlxuICAgICAgICAgICAgezplbHNlfVxuICAgICAgICAgICAgICB7Yy5sYWJlbH1cbiAgICAgICAgICAgIHsvaWZ9XG4gICAgICAgICAgPC90aD5cbiAgICAgICAgey9lYWNofVxuICAgICAgPC90cj5cbiAgICA8L3RoZWFkPlxuICAgIDx0Ym9keT5cbiAgICAgIHsjaWYgZXJyb3J9XG4gICAgICAgIDx0cj48dGQgY2xhc3M9XCJtc2cgYmFkXCIgY29sc3Bhbj17Y29sdW1ucy5sZW5ndGh9IHJvbGU9XCJhbGVydFwiPntlcnJvcn08L3RkPjwvdHI+XG4gICAgICB7OmVsc2UgaWYgbG9hZGluZ31cbiAgICAgICAgPHRyPjx0ZCBjbGFzcz1cIm1zZ1wiIGNvbHNwYW49e2NvbHVtbnMubGVuZ3RofT5Mb2FkaW5n4oCmPC90ZD48L3RyPlxuICAgICAgezplbHNlfVxuICAgICAgICB7I2lmIGZpcnN0ID4gMH08dHIgY2xhc3M9XCJwYWRcIiBhcmlhLWhpZGRlbj1cInRydWVcIiBzdHlsZTpoZWlnaHQ9e2ZpcnN0ICogcm93SGVpZ2h0ICsgJ3B4J30+PC90cj57L2lmfVxuICAgICAgICB7I2VhY2ggdmlzaWJsZSBhcyByIChyW3Jvd0tleV0pfVxuICAgICAgICAgIDx0ciBzdHlsZTpoZWlnaHQ9e3Jvd0hlaWdodCArICdweCd9PlxuICAgICAgICAgICAgeyNlYWNoIGNvbHVtbnMgYXMgYyAoYy5rZXkpfTx0ZD57cltjLmtleV19PC90ZD57L2VhY2h9XG4gICAgICAgICAgPC90cj5cbiAgICAgICAgezplbXB0eX1cbiAgICAgICAgICA8dHI+PHRkIGNsYXNzPVwibXNnXCIgY29sc3Bhbj17Y29sdW1ucy5sZW5ndGh9PntlbXB0eX08L3RkPjwvdHI+XG4gICAgICAgIHsvZWFjaH1cbiAgICAgICAgeyNpZiBsYXN0IDwgc29ydGVkLmxlbmd0aH08dHIgY2xhc3M9XCJwYWRcIiBhcmlhLWhpZGRlbj1cInRydWVcIiBzdHlsZTpoZWlnaHQ9eyhzb3J0ZWQubGVuZ3RoIC0gbGFzdCkgKiByb3dIZWlnaHQgKyAncHgnfT48L3RyPnsvaWZ9XG4gICAgICB7L2lmfVxuICAgIDwvdGJvZHk+XG4gIDwvdGFibGU+XG48L2Rpdj5cbjxzdHlsZT5cbiAgLndyYXAgeyBvdmVyZmxvdzogYXV0bzsgYm9yZGVyOiAxcHggc29saWQgdmFyKC0tZ2xhc3MtZWRnZSk7IGJvcmRlci1yYWRpdXM6IHZhcigtLXJhZGl1cyk7IGJhY2tncm91bmQ6IHZhcigtLWdsYXNzKTsgYm94LXNoYWRvdzogdmFyKC0tc2hhZG93KTsgfVxuICB0YWJsZSB7IHdpZHRoOiAxMDAlOyBib3JkZXItY29sbGFwc2U6IGNvbGxhcHNlOyBmb250OiAxNHB4LzEuNCB2YXIoLS1mb250KTsgY29sb3I6IHZhcigtLWZnKTsgZm9udC12YXJpYW50LW51bWVyaWM6IHRhYnVsYXItbnVtczsgfVxuICBjYXB0aW9uIHsgdGV4dC1hbGlnbjogbGVmdDsgZm9udC13ZWlnaHQ6IDY1MDsgcGFkZGluZzogdmFyKC0tc3BhY2UtMykgdmFyKC0tc3BhY2UtMykgdmFyKC0tc3BhY2UtMik7IH1cbiAgdGgsIHRkIHsgdGV4dC1hbGlnbjogbGVmdDsgcGFkZGluZzogdmFyKC0tc3BhY2UtMikgdmFyKC0tc3BhY2UtMyk7IGJvcmRlci1ib3R0b206IDFweCBzb2xpZCB2YXIoLS1saW5lKTsgfVxuICB0aGVhZCB0aCB7IHBvc2l0aW9uOiBzdGlja3k7IHRvcDogMDsgYmFja2dyb3VuZDogdmFyKC0tc3VyZmFjZSk7IGZvbnQtd2VpZ2h0OiA2MDA7IGNvbG9yOiB2YXIoLS1tdXRlZCk7IH1cbiAgdGggYnV0dG9uIHsgZm9udDogaW5oZXJpdDsgY29sb3I6IGluaGVyaXQ7IGJhY2tncm91bmQ6IG5vbmU7IGJvcmRlcjogMDsgcGFkZGluZzogMDsgY3Vyc29yOiBwb2ludGVyOyB9XG4gIHRoIGJ1dHRvbjpmb2N1cy12aXNpYmxlIHsgb3V0bGluZTogMnB4IHNvbGlkIHZhcigtLWFjY2VudCk7IG91dGxpbmUtb2Zmc2V0OiAycHg7IH1cbiAgLm1zZyB7IHRleHQtYWxpZ246IGNlbnRlcjsgY29sb3I6IHZhcigtLW11dGVkKTsgcGFkZGluZzogdmFyKC0tc3BhY2UtNSk7IH1cbiAgLm1zZy5iYWQgeyBjb2xvcjogdmFyKC0tYmFkKTsgfVxuICAucGFkIHRkIHsgcGFkZGluZzogMDsgYm9yZGVyOiAwOyB9XG48L3N0eWxlPlxuIl0sIm5hbWVzIjpbXSwibWFwcGluZ3MiOiI7Ozs7Ozs7Ozs7Ozs7Ozs7QUFHQTtBQUNBO0FBQ0E7QUFDQTtBQUNBO0FBQ0E7QUFDQTtBQUFBO0FBQUE7QUFDQTtBQUNBO0FBQ0E7QUFBQTtBQUVBO0FBQ0E7QUFDQTtBQUNBO0FBQ0E7QUFDQTtBQUFBO0FBQUE7QUFDQTtBQUFBO0FBQUE7QUFBQTtBQUVBO0FBQ0E7QUFDQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFFQTtBQUFBO0FBQUE7QUFDQTtBQUFBO0FBQ0E7QUFHQTtBQUFBO0FBQUE7QUFBQTtBQUNBO0FBQ0E7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUNBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBRUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQU9BO0FBQUE7QUFBQTtBQUFBO0FBQ0E7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUVBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBRUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQ0E7QUFBQTtBQUFBO0FBQUE7QUFDQTtBQUNBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFHQTtBQUFBO0FBQUE7QUFBQTtBQUVBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTsifQ==

