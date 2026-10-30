import { fx, tpl, insert, attr, on, when } from "__RT__/brain.js";
import { budgetEnter as $brainBudgetbudgetEnter, budgetLeave as $brainBudgetbudgetLeave, budgetOwner as $brainBudgetbudgetOwner, budgetStep as $brainBudgetbudgetStep } from "__RT__/brain-budget.js";
import { dev, fragment as $brainBudgetFragment } from "__RT__/brain-dev.js";
const $t0 = $brainBudgetFragment(tpl("<a>Sign in</a>"), "bepy4g", 0);
const $t1 = $brainBudgetFragment(tpl("<button type=\"button\">Retry</button>"), "bepy4g", 1);
const $t2 = $brainBudgetFragment(tpl("<small>Reference: <!></small>"), "bepy4g", 2);
const $t3 = $brainBudgetFragment(tpl("<div data-b=\"bepy4g\" class=\"error-state\" role=\"alert\"><p><!></p> <!> <!> <!></div>"), "bepy4g", 3);
const $t4 = $brainBudgetFragment(tpl("<!><!>"), "bepy4g", 4);
function ErrorState($p = {}) {
  const $brainBudgetOwner = $brainBudgetbudgetOwner();
  const text = (e) => { const $$brainBudgetFrame1 = $brainBudgetbudgetEnter({ file: ".brain/kit/ui/error-state.brain", component: "ErrorState", line: 6, column: 16 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame1); {
    if (!e) {
      return '';
    }
    if (e.kind === 'offline') {
      return 'You are offline.';
    }
    if (e.kind === 'network') {
      return 'The connection was lost.';
    }
    if (e.kind === 'timeout') {
      return 'The server took too long to answer.';
    }
    if (e.kind === 'schema') {
      return 'The server sent something unexpected.';
    }
    if (e.status === 401) {
      return 'Sign in to continue.';
    }
    if (e.status === 403) {
      return 'You do not have permission to see this.';
    }
    if (e.status === 404) {
      return 'Not found.';
    }
    if (e.status === 422) {
      return 'Check the highlighted fields.';
    }
    if (e.status === 429) {
      return 'Too many requests. Trying again in ' + (e.retryAfter || 30) + ' seconds.';
    }
    return 'Something went wrong on our side.';
  } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame1); } };
  const canRetry = (e) => { const $$brainBudgetFrame2 = $brainBudgetbudgetEnter({ file: ".brain/kit/ui/error-state.brain", component: "ErrorState", line: 19, column: 20 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame2); { return ! !$p.retry && ! !e && (e.kind !== 'http' || e.status >= 500); } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame2); } };
  const signIn = () => { const $$brainBudgetFrame3 = $brainBudgetbudgetEnter({ file: ".brain/kit/ui/error-state.brain", component: "ErrorState", line: 20, column: 18 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame3); { return ($p.login ?? '/login') + (globalThis.location ? '?next=' + encodeURIComponent(location.pathname + location.search) : ''); } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame3); } };
  fx(() => { const $$brainBudgetFrame4 = $brainBudgetbudgetEnter({ file: ".brain/kit/ui/error-state.brain", component: "ErrorState", line: 21, column: 6 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame4); {
    if (!$p.retry || !$p.error || ($p.error.kind !== 'offline' && $p.error.kind !== 'network') || !globalThis.addEventListener) {
      return;
    }
    const again = () => { const $$brainBudgetFrame5 = $brainBudgetbudgetEnter({ file: ".brain/kit/ui/error-state.brain", component: "ErrorState", line: 23, column: 19 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame5); { return $p.retry(); } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame5); } };
    globalThis.addEventListener('online', again);
    return () => { const $$brainBudgetFrame6 = $brainBudgetbudgetEnter({ file: ".brain/kit/ui/error-state.brain", component: "ErrorState", line: 25, column: 12 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame6); { return globalThis.removeEventListener('online', again); } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame6); } };
  } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame4); } });
  fx(() => { const $$brainBudgetFrame7 = $brainBudgetbudgetEnter({ file: ".brain/kit/ui/error-state.brain", component: "ErrorState", line: 27, column: 6 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame7); {
    if (!$p.retry || !$p.error || $p.error.status !== 429) {
      return;
    }
    const t = setTimeout(() => { const $$brainBudgetFrame8 = $brainBudgetbudgetEnter({ file: ".brain/kit/ui/error-state.brain", component: "ErrorState", line: 29, column: 26 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame8); { return $p.retry(); } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame8); } }, ($p.error.retryAfter || 30) * 1000);
    return () => { const $$brainBudgetFrame9 = $brainBudgetbudgetEnter({ file: ".brain/kit/ui/error-state.brain", component: "ErrorState", line: 30, column: 12 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame9); { return clearTimeout(t); } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame9); } };
  } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame7); } });
  const $r22 = $t4();
  const $e10 = $r22.firstChild;
  when(null, () => $p.error && $p.error.kind !== 'aborted', () => {
    const $r21 = $t3();
    const $e11 = $r21.firstChild.firstChild;
    const $e12 = $r21.firstChild.nextSibling.nextSibling;
    const $e15 = $e12.nextSibling.nextSibling;
    const $e18 = $e15.nextSibling.nextSibling;
    insert(null, () => text($p.error), $e11);
    when(null, () => $p.error && $p.error.status === 401, () => {
      const $r14 = $t0();
      const $e13 = $r14;
      attr($e13, "href", () => signIn());
      return $r14;
    }, null, $e12);
    when(null, () => canRetry($p.error), () => {
      const $r17 = $t1();
      const $e16 = $r17;
      on($e16, "click", () => { $p.retry(); });
      return $r17;
    }, null, $e15);
    when(null, () => $p.error && $p.error.id && $p.error.status >= 500, () => {
      const $r20 = $t2();
      const $e19 = $r20.firstChild.nextSibling;
      insert(null, () => $p.error.id, $e19);
      return $r20;
    }, null, $e18);
    return $r21;
  }, null, $e10);
  return $r22;
}
ErrorState.h = "bepy4g";
ErrorState.$hmr = { version: 1, wiring: "import { fx, tpl, insert, attr, on, when } from \"__RT__/brain.js\";\nimport { budgetEnter as $brainBudgetbudgetEnter, budgetLeave as $brainBudgetbudgetLeave, budgetOwner as $brainBudgetbudgetOwner, budgetStep as $brainBudgetbudgetStep } from \"__RT__/brain-budget.js\";\nimport { dev, fragment as $brainBudgetFragment } from \"__RT__/brain-dev.js\";\nconst $t0 = $brainBudgetFragment(tpl(\"__brain_static_template__\"), \"bepy4g\", 0);\nconst $t1 = $brainBudgetFragment(tpl(\"__brain_static_template__\"), \"bepy4g\", 1);\nconst $t2 = $brainBudgetFragment(tpl(\"__brain_static_template__\"), \"bepy4g\", 2);\nconst $t3 = $brainBudgetFragment(tpl(\"__brain_static_template__\"), \"bepy4g\", 3);\nconst $t4 = $brainBudgetFragment(tpl(\"__brain_static_template__\"), \"bepy4g\", 4);\nfunction ErrorState($p = {}) {\n  const $brainBudgetOwner = $brainBudgetbudgetOwner();\n  const text = (e) => { const $$brainBudgetFrame1 = $brainBudgetbudgetEnter({ file: \".brain/kit/ui/error-state.brain\", component: \"ErrorState\", line: 6, column: 16 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame1); {\n    if (!e) {\n      return '';\n    }\n    if (e.kind === 'offline') {\n      return 'You are offline.';\n    }\n    if (e.kind === 'network') {\n      return 'The connection was lost.';\n    }\n    if (e.kind === 'timeout') {\n      return 'The server took too long to answer.';\n    }\n    if (e.kind === 'schema') {\n      return 'The server sent something unexpected.';\n    }\n    if (e.status === 401) {\n      return 'Sign in to continue.';\n    }\n    if (e.status === 403) {\n      return 'You do not have permission to see this.';\n    }\n    if (e.status === 404) {\n      return 'Not found.';\n    }\n    if (e.status === 422) {\n      return 'Check the highlighted fields.';\n    }\n    if (e.status === 429) {\n      return 'Too many requests. Trying again in ' + (e.retryAfter || 30) + ' seconds.';\n    }\n    return 'Something went wrong on our side.';\n  } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame1); } };\n  const canRetry = (e) => { const $$brainBudgetFrame2 = $brainBudgetbudgetEnter({ file: \".brain/kit/ui/error-state.brain\", component: \"ErrorState\", line: 19, column: 20 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame2); { return ! !$p.retry && ! !e && (e.kind !== 'http' || e.status >= 500); } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame2); } };\n  const signIn = () => { const $$brainBudgetFrame3 = $brainBudgetbudgetEnter({ file: \".brain/kit/ui/error-state.brain\", component: \"ErrorState\", line: 20, column: 18 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame3); { return ($p.login ?? '/login') + (globalThis.location ? '?next=' + encodeURIComponent(location.pathname + location.search) : ''); } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame3); } };\n  fx(() => { const $$brainBudgetFrame4 = $brainBudgetbudgetEnter({ file: \".brain/kit/ui/error-state.brain\", component: \"ErrorState\", line: 21, column: 6 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame4); {\n    if (!$p.retry || !$p.error || ($p.error.kind !== 'offline' && $p.error.kind !== 'network') || !globalThis.addEventListener) {\n      return;\n    }\n    const again = () => { const $$brainBudgetFrame5 = $brainBudgetbudgetEnter({ file: \".brain/kit/ui/error-state.brain\", component: \"ErrorState\", line: 23, column: 19 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame5); { return $p.retry(); } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame5); } };\n    globalThis.addEventListener('online', again);\n    return () => { const $$brainBudgetFrame6 = $brainBudgetbudgetEnter({ file: \".brain/kit/ui/error-state.brain\", component: \"ErrorState\", line: 25, column: 12 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame6); { return globalThis.removeEventListener('online', again); } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame6); } };\n  } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame4); } });\n  fx(() => { const $$brainBudgetFrame7 = $brainBudgetbudgetEnter({ file: \".brain/kit/ui/error-state.brain\", component: \"ErrorState\", line: 27, column: 6 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame7); {\n    if (!$p.retry || !$p.error || $p.error.status !== 429) {\n      return;\n    }\n    const t = setTimeout(() => { const $$brainBudgetFrame8 = $brainBudgetbudgetEnter({ file: \".brain/kit/ui/error-state.brain\", component: \"ErrorState\", line: 29, column: 26 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame8); { return $p.retry(); } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame8); } }, ($p.error.retryAfter || 30) * 1000);\n    return () => { const $$brainBudgetFrame9 = $brainBudgetbudgetEnter({ file: \".brain/kit/ui/error-state.brain\", component: \"ErrorState\", line: 30, column: 12 }, $brainBudgetOwner); try { $brainBudgetbudgetStep($$brainBudgetFrame9); { return clearTimeout(t); } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame9); } };\n  } } finally { $brainBudgetbudgetLeave($$brainBudgetFrame7); } });\n  const $r22 = $t4();\n  const $e10 = $r22.firstChild;\n  when(null, () => $p.error && $p.error.kind !== 'aborted', () => {\n    const $r21 = $t3();\n    const $e11 = $r21.firstChild.firstChild;\n    const $e12 = $r21.firstChild.nextSibling.nextSibling;\n    const $e15 = $e12.nextSibling.nextSibling;\n    const $e18 = $e15.nextSibling.nextSibling;\n    insert(null, () => text($p.error), $e11);\n    when(null, () => $p.error && $p.error.status === 401, () => {\n      const $r14 = $t0();\n      const $e13 = $r14;\n      attr($e13, \"href\", () => signIn());\n      return $r14;\n    }, null, $e12);\n    when(null, () => canRetry($p.error), () => {\n      const $r17 = $t1();\n      const $e16 = $r17;\n      on($e16, \"click\", () => { $p.retry(); });\n      return $r17;\n    }, null, $e15);\n    when(null, () => $p.error && $p.error.id && $p.error.status >= 500, () => {\n      const $r20 = $t2();\n      const $e19 = $r20.firstChild.nextSibling;\n      insert(null, () => $p.error.id, $e19);\n      return $r20;\n    }, null, $e18);\n    return $r21;\n  }, null, $e10);\n  return $r22;\n}\nErrorState.h = \"bepy4g\";\nexport default dev(ErrorState);\n", templates: ["<a>Sign in</a>","<button type=\"button\">Retry</button>","<small>Reference: <!></small>","<div data-b=\"bepy4g\" class=\"error-state\" role=\"alert\"><p><!></p> <!> <!> <!></div>","<!><!>"] };
export default dev(ErrorState);
//# sourceMappingURL=data:application/json;base64,eyJ2ZXJzaW9uIjozLCJmaWxlIjoiZXJyb3Itc3RhdGUuYnJhaW4uanMiLCJzb3VyY2VzIjpbImVycm9yLXN0YXRlLmJyYWluIl0sInNvdXJjZXNDb250ZW50IjpbIjxtZXRhIHByb3BzPVwiZXJyb3I6YW55IHJldHJ5OmZuPyBsb2dpbjpzdHI9Jy9sb2dpbidcIj5cbjxzY3JpcHQ+XG4gIC8vIE9uZSBtZXNzYWdlIGFuZCBvbmUgYWN0aW9uIHBlciBlcnJvciBraW5kIChJTlRFUk5BTFMgwqc0MC4yKSwgZm9yIHs6Y2F0Y2ggZX0gYW5kIGZhaWxlZCByZXNvdXJjZXMuIE9mZmxpbmUgYW5kXG4gIC8vIG5ldHdvcmsgZXJyb3JzIHJldHJ5IGJ5IHRoZW1zZWx2ZXMgd2hlbiB0aGUgY29ubmVjdGlvbiBjb21lcyBiYWNrOyA0MjkgcmV0cmllcyBhZnRlciBpdHMgUmV0cnktQWZ0ZXIuIEEgY2FuY2VsbGVkXG4gIC8vIHJlcXVlc3QgKGtpbmQgJ2Fib3J0ZWQnKSBzaG93cyBub3RoaW5nLlxuICBjb25zdCB0ZXh0ID0gKGUpID0+IHtcbiAgICBpZiAoIWUpIHJldHVybiAnJ1xuICAgIGlmIChlLmtpbmQgPT09ICdvZmZsaW5lJykgcmV0dXJuICdZb3UgYXJlIG9mZmxpbmUuJ1xuICAgIGlmIChlLmtpbmQgPT09ICduZXR3b3JrJykgcmV0dXJuICdUaGUgY29ubmVjdGlvbiB3YXMgbG9zdC4nXG4gICAgaWYgKGUua2luZCA9PT0gJ3RpbWVvdXQnKSByZXR1cm4gJ1RoZSBzZXJ2ZXIgdG9vayB0b28gbG9uZyB0byBhbnN3ZXIuJ1xuICAgIGlmIChlLmtpbmQgPT09ICdzY2hlbWEnKSByZXR1cm4gJ1RoZSBzZXJ2ZXIgc2VudCBzb21ldGhpbmcgdW5leHBlY3RlZC4nXG4gICAgaWYgKGUuc3RhdHVzID09PSA0MDEpIHJldHVybiAnU2lnbiBpbiB0byBjb250aW51ZS4nXG4gICAgaWYgKGUuc3RhdHVzID09PSA0MDMpIHJldHVybiAnWW91IGRvIG5vdCBoYXZlIHBlcm1pc3Npb24gdG8gc2VlIHRoaXMuJ1xuICAgIGlmIChlLnN0YXR1cyA9PT0gNDA0KSByZXR1cm4gJ05vdCBmb3VuZC4nXG4gICAgaWYgKGUuc3RhdHVzID09PSA0MjIpIHJldHVybiAnQ2hlY2sgdGhlIGhpZ2hsaWdodGVkIGZpZWxkcy4nXG4gICAgaWYgKGUuc3RhdHVzID09PSA0MjkpIHJldHVybiAnVG9vIG1hbnkgcmVxdWVzdHMuIFRyeWluZyBhZ2FpbiBpbiAnICsgKGUucmV0cnlBZnRlciB8fCAzMCkgKyAnIHNlY29uZHMuJ1xuICAgIHJldHVybiAnU29tZXRoaW5nIHdlbnQgd3Jvbmcgb24gb3VyIHNpZGUuJ1xuICB9XG4gIGNvbnN0IGNhblJldHJ5ID0gKGUpID0+ICEhcmV0cnkgJiYgISFlICYmIChlLmtpbmQgIT09ICdodHRwJyB8fCBlLnN0YXR1cyA+PSA1MDApXG4gIGNvbnN0IHNpZ25JbiA9ICgpID0+IGxvZ2luICsgKGdsb2JhbFRoaXMubG9jYXRpb24gPyAnP25leHQ9JyArIGVuY29kZVVSSUNvbXBvbmVudChsb2NhdGlvbi5wYXRobmFtZSArIGxvY2F0aW9uLnNlYXJjaCkgOiAnJylcbiAgZngoKCkgPT4ge1xuICAgIGlmICghcmV0cnkgfHwgIWVycm9yIHx8IChlcnJvci5raW5kICE9PSAnb2ZmbGluZScgJiYgZXJyb3Iua2luZCAhPT0gJ25ldHdvcmsnKSB8fCAhZ2xvYmFsVGhpcy5hZGRFdmVudExpc3RlbmVyKSByZXR1cm4gLy8gbm8gd2luZG93IGV2ZW50cyB3aGlsZSB0aGUgc2VydmVyIHJlbmRlcnNcbiAgICBjb25zdCBhZ2FpbiA9ICgpID0+IHJldHJ5KClcbiAgICBnbG9iYWxUaGlzLmFkZEV2ZW50TGlzdGVuZXIoJ29ubGluZScsIGFnYWluKVxuICAgIHJldHVybiAoKSA9PiBnbG9iYWxUaGlzLnJlbW92ZUV2ZW50TGlzdGVuZXIoJ29ubGluZScsIGFnYWluKVxuICB9KVxuICBmeCgoKSA9PiB7XG4gICAgaWYgKCFyZXRyeSB8fCAhZXJyb3IgfHwgZXJyb3Iuc3RhdHVzICE9PSA0MjkpIHJldHVyblxuICAgIGNvbnN0IHQgPSBzZXRUaW1lb3V0KCgpID0+IHJldHJ5KCksIChlcnJvci5yZXRyeUFmdGVyIHx8IDMwKSAqIDEwMDApXG4gICAgcmV0dXJuICgpID0+IGNsZWFyVGltZW91dCh0KVxuICB9KVxuPC9zY3JpcHQ+XG57I2lmIGVycm9yICYmIGVycm9yLmtpbmQgIT09ICdhYm9ydGVkJ31cbjxkaXYgY2xhc3M9XCJlcnJvci1zdGF0ZVwiIHJvbGU9XCJhbGVydFwiPlxuICA8cD57dGV4dChlcnJvcil9PC9wPlxuICB7I2lmIGVycm9yICYmIGVycm9yLnN0YXR1cyA9PT0gNDAxfTxhIGhyZWY9e3NpZ25JbigpfT5TaWduIGluPC9hPnsvaWZ9XG4gIHsjaWYgY2FuUmV0cnkoZXJyb3IpfTxidXR0b24gdHlwZT1cImJ1dHRvblwiIG9uOmNsaWNrPXtyZXRyeSgpfT5SZXRyeTwvYnV0dG9uPnsvaWZ9XG4gIHsjaWYgZXJyb3IgJiYgZXJyb3IuaWQgJiYgZXJyb3Iuc3RhdHVzID49IDUwMH08c21hbGw+UmVmZXJlbmNlOiB7ZXJyb3IuaWR9PC9zbWFsbD57L2lmfVxuPC9kaXY+XG57L2lmfVxuPHN0eWxlPlxuICAuZXJyb3Itc3RhdGUge1xuICAgIGRpc3BsYXk6IGdyaWQ7IGp1c3RpZnktaXRlbXM6IHN0YXJ0OyBnYXA6IHZhcigtLXNwYWNlLTIpOyBwYWRkaW5nOiB2YXIoLS1zcGFjZS00KTsgZm9udDogMTRweC8xLjUgdmFyKC0tZm9udCk7XG4gICAgY29sb3I6IHZhcigtLWZnKTsgYmFja2dyb3VuZDogdmFyKC0tZ2xhc3MpOyBib3JkZXI6IDFweCBzb2xpZCB2YXIoLS1nbGFzcy1lZGdlKTsgYm9yZGVyLXJhZGl1czogdmFyKC0tcmFkaXVzKTtcbiAgfVxuICBwIHsgbWFyZ2luOiAwOyB9XG4gIGEgeyBjb2xvcjogdmFyKC0tYWNjZW50KTsgfVxuICBidXR0b24ge1xuICAgIGZvbnQ6IGluaGVyaXQ7IHBhZGRpbmc6IHZhcigtLXNwYWNlLTEpIHZhcigtLXNwYWNlLTMpOyBib3JkZXI6IDFweCBzb2xpZCB2YXIoLS1saW5lKTsgYm9yZGVyLXJhZGl1czogdmFyKC0tcmFkaXVzLXNtKTtcbiAgICBjb2xvcjogdmFyKC0tZmcpOyBiYWNrZ3JvdW5kOiB2YXIoLS1zdXJmYWNlKTsgY3Vyc29yOiBwb2ludGVyO1xuICB9XG4gIHNtYWxsIHsgY29sb3I6IHZhcigtLW11dGVkKTsgfVxuPC9zdHlsZT5cbiJdLCJuYW1lcyI6W10sIm1hcHBpbmdzIjoiOzs7Ozs7Ozs7O0FBS0E7QUFDQTtBQUFBO0FBQUE7QUFDQTtBQUFBO0FBQUE7QUFDQTtBQUFBO0FBQUE7QUFDQTtBQUFBO0FBQUE7QUFDQTtBQUFBO0FBQUE7QUFDQTtBQUFBO0FBQUE7QUFDQTtBQUFBO0FBQUE7QUFDQTtBQUFBO0FBQUE7QUFDQTtBQUFBO0FBQUE7QUFDQTtBQUFBO0FBQUE7QUFDQTtBQUFBO0FBRUE7QUFDQTtBQUNBO0FBQ0E7QUFBQTtBQUFBO0FBQ0E7QUFDQTtBQUNBO0FBQUE7QUFFQTtBQUNBO0FBQUE7QUFBQTtBQUNBO0FBQ0E7QUFBQTtBQUFBO0FBQUE7QUFHQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFFQTtBQUNBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUNBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUNBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBOyJ9

