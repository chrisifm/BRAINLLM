import { guard } from "__RT__/brain-guard.js";
import { sig, drv, tpl, insert, attr, cls, on, list } from "__RT__/brain.js";
import { budgetEnter as $brainBudgetbudgetEnter, budgetLeave as $brainBudgetbudgetLeave, budgetOwner as $brainBudgetbudgetOwner, budgetStep as $brainBudgetbudgetStep } from "__RT__/brain-budget.js";
const $t0 = tpl("<li><input type=\"checkbox\"> <span><!></span> <button aria-label=\"Delete\">×</button></li>");
const $t1 = tpl("<li class=\"empty\">Nothing here.</li>");
const $t2 = tpl("<section data-b=\"lxd7ly\" class=\"todo\"><h2><!></h2> <form><input placeholder=\"What needs doing?\" aria-label=\"New task\"> <button>Add</button></form> <ul><!></ul> <footer><span><!> left</span> <button>All</button> <button>Open</button> <button>Done</button></footer></section>");
function Todo($p = {}) {
  const $brainBudgetOwner = $brainBudgetbudgetOwner();
  const items = sig([]);
  const text = sig('');
  const filter = sig('all');
  let nextId = 1;
  const visible = drv(() => { const $$brainBudgetFrame1 = $brainBudgetbudgetEnter({ file: "comp/todo.brain", component: "Todo", line: 9, column: 21 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame1); {
    if (filter.v === 'done') {
      return items.v.filter((it) => { const $$brainBudgetFrame2 = $brainBudgetbudgetEnter({ file: "comp/todo.brain", component: "Todo", line: 10, column: 48 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame2); { return it.done; } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame2); } });
    }
    if (filter.v === 'open') {
      return items.v.filter((it) => { const $$brainBudgetFrame3 = $brainBudgetbudgetEnter({ file: "comp/todo.brain", component: "Todo", line: 11, column: 48 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame3); { return !it.done; } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame3); } });
    }
    return items.v;
  } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame1); } });
  const left = drv(() => { const $$brainBudgetFrame4 = $brainBudgetbudgetEnter({ file: "comp/todo.brain", component: "Todo", line: 14, column: 18 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame4); { return items.v.filter((it) => { const $$brainBudgetFrame5 = $brainBudgetbudgetEnter({ file: "comp/todo.brain", component: "Todo", line: 14, column: 37 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame5); { return !it.done; } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame5); } }).length; } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame4); } });
  const add = () => { const $$brainBudgetFrame6 = $brainBudgetbudgetEnter({ file: "comp/todo.brain", component: "Todo", line: 16, column: 15 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame6); {
    const t = text.v.trim();
    if (t === '') {
      return;
    }
    items.v = [...items.v, { id: nextId++, text: t, done: false }];
    text.v = '';
  } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame6); } };
  const toggle = (id) => { const $$brainBudgetFrame7 = $brainBudgetbudgetEnter({ file: "comp/todo.brain", component: "Todo", line: 22, column: 18 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame7); {
    items.v = items.v.map((it) => { const $$brainBudgetFrame8 = $brainBudgetbudgetEnter({ file: "comp/todo.brain", component: "Todo", line: 23, column: 23 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame8); { return it.id === id ? { ...it, done: !it.done } : it; } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame8); } });
  } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame7); } };
  const remove = (id) => { const $$brainBudgetFrame9 = $brainBudgetbudgetEnter({ file: "comp/todo.brain", component: "Todo", line: 25, column: 18 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame9); {
    items.v = items.v.filter((it) => { const $$brainBudgetFrame10 = $brainBudgetbudgetEnter({ file: "comp/todo.brain", component: "Todo", line: 26, column: 26 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame10); { return it.id !== id; } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame10); } });
  } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame9); } };
  const $r26 = $t2();
  const $e11 = $r26.firstChild.firstChild;
  const $e12 = $r26.firstChild.nextSibling.nextSibling;
  const $e13 = $e12.firstChild;
  const $e14 = $e13.nextSibling.nextSibling;
  const $e15 = $e12.nextSibling.nextSibling.firstChild;
  const $e22 = $e12.nextSibling.nextSibling.nextSibling.nextSibling.firstChild.firstChild;
  const $e23 = $e12.nextSibling.nextSibling.nextSibling.nextSibling.firstChild.nextSibling.nextSibling;
  const $e24 = $e23.nextSibling.nextSibling;
  const $e25 = $e24.nextSibling.nextSibling;
  insert(null, () => ($p.title ?? 'To do'), $e11);
  on($e12, "submit", (e) => { e.preventDefault(); (add)(e); });
  attr($e13, "value", () => text.v);
  on($e13, "input", (e) => { text.v = e.target.value; });
  attr($e14, "disabled", () => text.v.trim() === '');
  list(null, () => visible.v, (it) => it.id, (it) => {
    const $r20 = $t0();
    const $e16 = $r20;
    const $e17 = $r20.firstChild;
    const $e18 = $e17.nextSibling.nextSibling.firstChild;
    const $e19 = $e17.nextSibling.nextSibling.nextSibling.nextSibling;
    cls($e16, "done", () => it.v.done);
    attr($e17, "checked", () => it.v.done);
    on($e17, "change", () => { toggle(it.v.id); });
    attr($e17, "aria-label", () => `Mark ${it.v.text}`);
    insert(null, () => it.v.text, $e18);
    on($e19, "click", () => { remove(it.v.id); });
    return $r20;
  }, $e15, () => {
    const $r21 = $t1();
    return $r21;
  });
  insert(null, () => left.v, $e22);
  cls($e23, "on", () => filter.v === 'all');
  on($e23, "click", () => { filter.v = 'all'; });
  cls($e24, "on", () => filter.v === 'open');
  on($e24, "click", () => { filter.v = 'open'; });
  cls($e25, "on", () => filter.v === 'done');
  on($e25, "click", () => { filter.v = 'done'; });
  return $r26;
}
Todo.h = "lxd7ly";
Todo.f = "comp/todo.brain";
export default guard(Todo);
