import { sig, fx, tpl, insert, attr, on, list } from "__RT__/brain.js";
import { dev } from "__RT__/brain-dev.js";
const $t0 = tpl("<button type=\"button\" role=\"menuitem\" tabindex=\"-1\"><!></button>");
const $t1 = tpl("<div data-b=\"5rtv19\" class=\"menu\"><button type=\"button\" aria-haspopup=\"menu\"><!></button> <div class=\"pop\" role=\"menu\"><!></div></div>");
function Menu($p = {}) {
  const open = sig(false, "open");
  const pop = sig(null, "pop");
  const btn = sig(null, "btn");
  const enabled = () => [...pop.v.querySelectorAll('[role=menuitem]:not([disabled])')];
  const choose = (it) => {
    open.v = false;
    btn.v.focus();
    $p["on:select"]?.(it);
  };
  const keys = (e) => {
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
  };
  fx(() => {
    if (!open.v || !pop.v) {
      return;
    }
    const h = requestAnimationFrame(() => {
      const all = enabled();
      if (all.length) {
        all[0].focus();
      }
    });
    const out = (e) => {
      if (!pop.v.contains(e.target) && !btn.v.contains(e.target)) {
        open.v = false;
      }
    };
    pop.v.ownerDocument.addEventListener('pointerdown', out);
    return () => {
      cancelAnimationFrame(h);
      pop.v.ownerDocument.removeEventListener('pointerdown', out);
    };
  });
  const $r8 = $t1();
  const $e1 = $r8.firstChild;
  const $e2 = $e1.firstChild;
  const $e3 = $e1.nextSibling.nextSibling;
  const $e4 = $e3.firstChild;
  btn.v = $e1;
  attr($e1, "aria-expanded", () => open.v ? 'true' : 'false');
  on($e1, "click", () => { open.v = !open.v; });
  insert(null, () => $p.label, $e2);
  attr($e3, "aria-label", () => $p.label);
  pop.v = $e3;
  attr($e3, "hidden", () => !open.v);
  on($e3, "keydown", keys);
  list(null, () => $p.items, (it) => it.id, (it) => {
    const $r7 = $t0();
    const $e5 = $r7;
    const $e6 = $r7.firstChild;
    attr($e5, "disabled", () => it.v.disabled);
    on($e5, "click", () => { choose(it.v); });
    insert(null, () => it.v.label, $e6);
    return $r7;
  }, $e4);
  return $r8;
}
Menu.h = "5rtv19";
export default dev(Menu);
//# sourceMappingURL=data:application/json;base64,eyJ2ZXJzaW9uIjozLCJmaWxlIjoibWVudS5icmFpbi5qcyIsInNvdXJjZXMiOlsibWVudS5icmFpbiJdLCJzb3VyY2VzQ29udGVudCI6WyI8bWV0YSBwcm9wcz1cImxhYmVsOnN0ciBpdGVtczpsaXN0XCIgZW1pdHM9XCJzZWxlY3Q6b2JqXCI+XG48c2NyaXB0PlxuICAvLyBpdGVtczogW3sgaWQ6ICdlZGl0JywgbGFiZWw6ICdFZGl0JywgZGlzYWJsZWQ/OiB0cnVlIH1dXG4gIGxldCBvcGVuID0gc2lnKGZhbHNlKVxuICBsZXQgcG9wID0gcmVmKClcbiAgbGV0IGJ0biA9IHJlZigpXG4gIGNvbnN0IGVuYWJsZWQgPSAoKSA9PiBbLi4ucG9wLnF1ZXJ5U2VsZWN0b3JBbGwoJ1tyb2xlPW1lbnVpdGVtXTpub3QoW2Rpc2FibGVkXSknKV1cbiAgY29uc3QgY2hvb3NlID0gKGl0KSA9PiB7XG4gICAgb3BlbiA9IGZhbHNlXG4gICAgYnRuLmZvY3VzKClcbiAgICBlbWl0KCdzZWxlY3QnLCBpdClcbiAgfVxuICBjb25zdCBrZXlzID0gKGUpID0+IHtcbiAgICBjb25zdCBhbGwgPSBlbmFibGVkKClcbiAgICBjb25zdCBpID0gYWxsLmluZGV4T2YoZS50YXJnZXQpXG4gICAgaWYgKGUua2V5ID09PSAnRXNjYXBlJykge1xuICAgICAgb3BlbiA9IGZhbHNlXG4gICAgICBidG4uZm9jdXMoKVxuICAgIH0gZWxzZSBpZiAoZS5rZXkgPT09ICdBcnJvd0Rvd24nICYmIGFsbC5sZW5ndGgpIHtcbiAgICAgIGUucHJldmVudERlZmF1bHQoKVxuICAgICAgYWxsWyhpICsgMSkgJSBhbGwubGVuZ3RoXS5mb2N1cygpXG4gICAgfSBlbHNlIGlmIChlLmtleSA9PT0gJ0Fycm93VXAnICYmIGFsbC5sZW5ndGgpIHtcbiAgICAgIGUucHJldmVudERlZmF1bHQoKVxuICAgICAgYWxsWyhpIC0gMSArIGFsbC5sZW5ndGgpICUgYWxsLmxlbmd0aF0uZm9jdXMoKVxuICAgIH1cbiAgfVxuICBmeCgoKSA9PiB7XG4gICAgaWYgKCFvcGVuIHx8ICFwb3ApIHJldHVyblxuICAgIC8vIG5leHQgZnJhbWU6IHRoZSBwb3B1cCBpcyB2aXNpYmxlIChoaWRkZW4gcmVtb3ZlZCkgb25seSBhZnRlciB0aGlzIHVwZGF0ZSBpcyBwYWludGVkXG4gICAgY29uc3QgaCA9IHJlcXVlc3RBbmltYXRpb25GcmFtZSgoKSA9PiB7XG4gICAgICBjb25zdCBhbGwgPSBlbmFibGVkKClcbiAgICAgIGlmIChhbGwubGVuZ3RoKSBhbGxbMF0uZm9jdXMoKVxuICAgIH0pXG4gICAgY29uc3Qgb3V0ID0gKGUpID0+IHtcbiAgICAgIGlmICghcG9wLmNvbnRhaW5zKGUudGFyZ2V0KSAmJiAhYnRuLmNvbnRhaW5zKGUudGFyZ2V0KSkgb3BlbiA9IGZhbHNlXG4gICAgfVxuICAgIHBvcC5vd25lckRvY3VtZW50LmFkZEV2ZW50TGlzdGVuZXIoJ3BvaW50ZXJkb3duJywgb3V0KVxuICAgIHJldHVybiAoKSA9PiB7XG4gICAgICBjYW5jZWxBbmltYXRpb25GcmFtZShoKVxuICAgICAgcG9wLm93bmVyRG9jdW1lbnQucmVtb3ZlRXZlbnRMaXN0ZW5lcigncG9pbnRlcmRvd24nLCBvdXQpXG4gICAgfVxuICB9KVxuPC9zY3JpcHQ+XG48ZGl2IGNsYXNzPVwibWVudVwiPlxuICA8YnV0dG9uIHR5cGU9XCJidXR0b25cIiBiaW5kOnJlZj17YnRufSBhcmlhLWhhc3BvcHVwPVwibWVudVwiIGFyaWEtZXhwYW5kZWQ9e29wZW4gPyAndHJ1ZScgOiAnZmFsc2UnfSBvbjpjbGljaz17b3BlbiA9ICFvcGVufT57bGFiZWx9PC9idXR0b24+XG4gIDxkaXYgY2xhc3M9XCJwb3BcIiByb2xlPVwibWVudVwiIGFyaWEtbGFiZWw9e2xhYmVsfSBiaW5kOnJlZj17cG9wfSBoaWRkZW49eyFvcGVufSBvbjprZXlkb3duPXtrZXlzfT5cbiAgICB7I2VhY2ggaXRlbXMgYXMgaXQgKGl0LmlkKX1cbiAgICAgIDxidXR0b24gdHlwZT1cImJ1dHRvblwiIHJvbGU9XCJtZW51aXRlbVwiIHRhYmluZGV4PVwiLTFcIiBkaXNhYmxlZD17aXQuZGlzYWJsZWR9IG9uOmNsaWNrPXtjaG9vc2UoaXQpfT57aXQubGFiZWx9PC9idXR0b24+XG4gICAgey9lYWNofVxuICA8L2Rpdj5cbjwvZGl2PlxuPHN0eWxlPlxuICAubWVudSB7IHBvc2l0aW9uOiByZWxhdGl2ZTsgZGlzcGxheTogaW5saW5lLWJsb2NrOyB9XG4gIC5tZW51ID4gYnV0dG9uIHsgZm9udDogNjAwIDE0cHgvMS4yIHZhcigtLWZvbnQpOyBwYWRkaW5nOiB2YXIoLS1zcGFjZS0yKSB2YXIoLS1zcGFjZS0zKTsgYm9yZGVyOiAxcHggc29saWQgdmFyKC0tZ2xhc3MtZWRnZSk7IGJvcmRlci1yYWRpdXM6IHZhcigtLXJhZGl1cy1zbSk7IGNvbG9yOiB2YXIoLS1mZyk7IGJhY2tncm91bmQ6IHZhcigtLWdsYXNzKTsgY3Vyc29yOiBwb2ludGVyOyB9XG4gIC5wb3Age1xuICAgIHBvc2l0aW9uOiBhYnNvbHV0ZTsgdG9wOiBjYWxjKDEwMCUgKyA2cHgpOyBsZWZ0OiAwOyBtaW4td2lkdGg6IDE4MHB4OyBkaXNwbGF5OiBncmlkOyBwYWRkaW5nOiB2YXIoLS1zcGFjZS0xKTsgei1pbmRleDogNTA7XG4gICAgYm9yZGVyOiAxcHggc29saWQgdmFyKC0tZ2xhc3MtZWRnZSk7IGJvcmRlci1yYWRpdXM6IHZhcigtLXJhZGl1cyk7IGJhY2tncm91bmQ6IHZhcigtLWdsYXNzKTsgYm94LXNoYWRvdzogdmFyKC0tc2hhZG93KTsgYmFja2Ryb3AtZmlsdGVyOiBibHVyKDIwcHgpIHNhdHVyYXRlKDEuNCk7XG4gIH1cbiAgLnBvcFtoaWRkZW5dIHsgZGlzcGxheTogbm9uZTsgfVxuICBbcm9sZT1tZW51aXRlbV0geyB0ZXh0LWFsaWduOiBsZWZ0OyBmb250OiAxNHB4LzEuMyB2YXIoLS1mb250KTsgcGFkZGluZzogdmFyKC0tc3BhY2UtMikgdmFyKC0tc3BhY2UtMyk7IGJvcmRlcjogMDsgYm9yZGVyLXJhZGl1czogdmFyKC0tcmFkaXVzLXNtKTsgY29sb3I6IHZhcigtLWZnKTsgYmFja2dyb3VuZDogdHJhbnNwYXJlbnQ7IGN1cnNvcjogcG9pbnRlcjsgfVxuICBbcm9sZT1tZW51aXRlbV06aG92ZXIsIFtyb2xlPW1lbnVpdGVtXTpmb2N1cy12aXNpYmxlIHsgYmFja2dyb3VuZDogdmFyKC0tc3VyZmFjZSk7IG91dGxpbmU6IG5vbmU7IH1cbiAgW3JvbGU9bWVudWl0ZW1dOmRpc2FibGVkIHsgY29sb3I6IHZhcigtLW11dGVkKTsgY3Vyc29yOiBub3QtYWxsb3dlZDsgfVxuICBidXR0b246Zm9jdXMtdmlzaWJsZSB7IG91dGxpbmU6IDJweCBzb2xpZCB2YXIoLS1hY2NlbnQpOyBvdXRsaW5lLW9mZnNldDogMnB4OyB9XG48L3N0eWxlPlxuIl0sIm5hbWVzIjpbXSwibWFwcGluZ3MiOiI7Ozs7O0FBR0E7QUFDQTtBQUNBO0FBQ0E7QUFDQTtBQUNBO0FBQ0E7QUFDQTtBQUFBO0FBRUE7QUFDQTtBQUNBO0FBQ0E7QUFDQTtBQUNBO0FBQUE7QUFBQTtBQUVBO0FBQ0E7QUFBQTtBQUFBO0FBRUE7QUFDQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBR0E7QUFDQTtBQUFBO0FBQUE7QUFFQTtBQUNBO0FBQ0E7QUFBQTtBQUFBO0FBQUE7QUFFQTtBQUNBO0FBQUE7QUFBQTtBQUFBO0FBRUE7QUFDQTtBQUNBO0FBQ0E7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUtBO0FBQUE7QUFBQTtBQUFBO0FBQ0E7QUFBQTtBQUFBO0FBQUE7QUFDQTtBQUFBO0FBQUE7QUFBQTtBQUNBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTsifQ==

