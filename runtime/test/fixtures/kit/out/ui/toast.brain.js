import { sig, fx, tpl, insert, attr, cls, on, list } from "__RT__/brain.js";
import { dev } from "__RT__/brain-dev.js";
const $t0 = tpl("<div class=\"toast\"><span><!></span> <button type=\"button\">×</button></div>");
const $t1 = tpl("<div data-b=\"xa8yj8\" class=\"toasts\" role=\"status\" aria-live=\"polite\"><!></div>");
function Toast($p = {}) {
  const items = sig([], "items");
  let next = 0;
  const dismiss = (id) => {
    items.v = items.v.filter((x) => x.id !== id);
  };
  fx(() => {
    let timers = [];
    const off = $p.from.on((m) => {
      const t = typeof m === 'object' && m ? m : { text: String(m) };
      next = next + 1;
      const id = next;
      items.v = [...items.v, { id, text: t.text, kind: t.kind || 'info' }].slice(-($p.max ?? 3));
      const h = setTimeout(() => {
        timers = timers.filter((x) => x !== h);
        dismiss(id);
      }, ($p.ms ?? 3000));
      timers = [...timers, h];
    });
    return () => {
      off();
      for (const h of timers) {
        clearTimeout(h);
      }
    };
  });
  const $r6 = $t1();
  const $e1 = $r6.firstChild;
  list(null, () => items.v, (t) => t.id, (t) => {
    const $r5 = $t0();
    const $e2 = $r5;
    const $e3 = $r5.firstChild.firstChild;
    const $e4 = $r5.firstChild.nextSibling.nextSibling;
    cls($e2, "ok", () => t.v.kind === 'ok');
    cls($e2, "bad", () => t.v.kind === 'error');
    insert(null, () => t.v.text, $e3);
    attr($e4, "aria-label", () => ($p.dismissLabel ?? 'Dismiss'));
    on($e4, "click", () => { dismiss(t.v.id); });
    return $r5;
  }, $e1);
  return $r6;
}
Toast.h = "xa8yj8";
export default dev(Toast);
//# sourceMappingURL=data:application/json;base64,eyJ2ZXJzaW9uIjozLCJmaWxlIjoidG9hc3QuYnJhaW4uanMiLCJzb3VyY2VzIjpbInRvYXN0LmJyYWluIl0sInNvdXJjZXNDb250ZW50IjpbIjxtZXRhIHByb3BzPVwiZnJvbTpvYmogbXM6bnVtPTMwMDAgbWF4Om51bT0zIGRpc21pc3NMYWJlbDpzdHI9J0Rpc21pc3MnXCI+XG48c2NyaXB0PlxuICAvLyBmcm9tOiBhIGJ1cyBmcm9tIGNoYW5uZWxzLyAoYGV4cG9ydCBkZWZhdWx0IGJ1cygndG9hc3QnKWApOyBzZW5kIGEgc3RyaW5nIG9yIHsgdGV4dCwga2luZDogJ29rJyB8ICdlcnJvcicgfVxuICBsZXQgaXRlbXMgPSBzaWcoW10pXG4gIGxldCBuZXh0ID0gMFxuICBjb25zdCBkaXNtaXNzID0gKGlkKSA9PiB7IGl0ZW1zID0gaXRlbXMuZmlsdGVyKCh4KSA9PiB4LmlkICE9PSBpZCkgfVxuICBmeCgoKSA9PiB7XG4gICAgbGV0IHRpbWVycyA9IFtdXG4gICAgY29uc3Qgb2ZmID0gZnJvbS5vbigobSkgPT4ge1xuICAgICAgY29uc3QgdCA9IHR5cGVvZiBtID09PSAnb2JqZWN0JyAmJiBtID8gbSA6IHsgdGV4dDogU3RyaW5nKG0pIH1cbiAgICAgIG5leHQgPSBuZXh0ICsgMVxuICAgICAgY29uc3QgaWQgPSBuZXh0XG4gICAgICBpdGVtcyA9IFsuLi5pdGVtcywgeyBpZCwgdGV4dDogdC50ZXh0LCBraW5kOiB0LmtpbmQgfHwgJ2luZm8nIH1dLnNsaWNlKC1tYXgpXG4gICAgICBjb25zdCBoID0gc2V0VGltZW91dCgoKSA9PiB7XG4gICAgICAgIHRpbWVycyA9IHRpbWVycy5maWx0ZXIoKHgpID0+IHggIT09IGgpXG4gICAgICAgIGRpc21pc3MoaWQpXG4gICAgICB9LCBtcylcbiAgICAgIHRpbWVycyA9IFsuLi50aW1lcnMsIGhdXG4gICAgfSlcbiAgICByZXR1cm4gKCkgPT4ge1xuICAgICAgb2ZmKClcbiAgICAgIGZvciAoY29uc3QgaCBvZiB0aW1lcnMpIGNsZWFyVGltZW91dChoKVxuICAgIH1cbiAgfSlcbjwvc2NyaXB0PlxuPGRpdiBjbGFzcz1cInRvYXN0c1wiIHJvbGU9XCJzdGF0dXNcIiBhcmlhLWxpdmU9XCJwb2xpdGVcIj5cbiAgeyNlYWNoIGl0ZW1zIGFzIHQgKHQuaWQpfVxuICAgIDxkaXYgY2xhc3M9XCJ0b2FzdFwiIGNsYXNzOm9rPXt0LmtpbmQgPT09ICdvayd9IGNsYXNzOmJhZD17dC5raW5kID09PSAnZXJyb3InfT5cbiAgICAgIDxzcGFuPnt0LnRleHR9PC9zcGFuPlxuICAgICAgPGJ1dHRvbiB0eXBlPVwiYnV0dG9uXCIgYXJpYS1sYWJlbD17ZGlzbWlzc0xhYmVsfSBvbjpjbGljaz17ZGlzbWlzcyh0LmlkKX0+w5c8L2J1dHRvbj5cbiAgICA8L2Rpdj5cbiAgey9lYWNofVxuPC9kaXY+XG48c3R5bGU+XG4gIC50b2FzdHMgeyBwb3NpdGlvbjogZml4ZWQ7IHJpZ2h0OiAxNnB4OyBib3R0b206IDE2cHg7IGRpc3BsYXk6IGdyaWQ7IGdhcDogdmFyKC0tc3BhY2UtMik7IHotaW5kZXg6IDEwMDA7IH1cbiAgLnRvYXN0IHtcbiAgICBkaXNwbGF5OiBmbGV4OyBhbGlnbi1pdGVtczogY2VudGVyOyBnYXA6IHZhcigtLXNwYWNlLTMpOyBwYWRkaW5nOiB2YXIoLS1zcGFjZS0zKSB2YXIoLS1zcGFjZS00KTsgbWluLXdpZHRoOiAyNDBweDtcbiAgICBmb250OiAxNHB4LzEuNCB2YXIoLS1mb250KTsgY29sb3I6IHZhcigtLWZnKTsgYmFja2dyb3VuZDogdmFyKC0tZ2xhc3MpOyBib3JkZXI6IDFweCBzb2xpZCB2YXIoLS1nbGFzcy1lZGdlKTtcbiAgICBib3JkZXItcmFkaXVzOiB2YXIoLS1yYWRpdXMpOyBib3gtc2hhZG93OiB2YXIoLS1zaGFkb3cpOyBiYWNrZHJvcC1maWx0ZXI6IGJsdXIoMjBweCkgc2F0dXJhdGUoMS40KTtcbiAgfVxuICAudG9hc3Qub2sgeyBib3JkZXItY29sb3I6IHZhcigtLW9rKTsgfVxuICAudG9hc3QuYmFkIHsgYm9yZGVyLWNvbG9yOiB2YXIoLS1iYWQpOyB9XG4gIC50b2FzdCBzcGFuIHsgZmxleDogMTsgfVxuICBidXR0b24geyBmb250LXNpemU6IDE4cHg7IGxpbmUtaGVpZ2h0OiAxOyBib3JkZXI6IDA7IGNvbG9yOiB2YXIoLS1tdXRlZCk7IGJhY2tncm91bmQ6IHRyYW5zcGFyZW50OyBjdXJzb3I6IHBvaW50ZXI7IH1cbjwvc3R5bGU+XG4iXSwibmFtZXMiOltdLCJtYXBwaW5ncyI6Ijs7Ozs7QUFHQTtBQUNBO0FBQ0E7QUFBQTtBQUFBO0FBQ0E7QUFDQTtBQUNBO0FBQ0E7QUFDQTtBQUNBO0FBQ0E7QUFDQTtBQUNBO0FBQ0E7QUFBQTtBQUVBO0FBQUE7QUFFQTtBQUNBO0FBQ0E7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFLQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQ0E7QUFBQTtBQUNBO0FBQ0E7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTsifQ==

