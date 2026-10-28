import { tpl, insert, attr, on, list, when } from "__RT__/brain.js";
import { dev } from "__RT__/brain-dev.js";
import { alternates, suggest, dismiss } from "__RT__/brain-i18n.js";
const $t0 = tpl("<li><a><!></a></li>");
const $t1 = tpl("<p class=\"hint\"><a><!></a> <button type=\"button\" aria-label=\"Dismiss\">×</button></p>");
const $t2 = tpl("<nav data-b=\"9dvubz\" class=\"lang\"><ul><!></ul> <!></nav>");
const $t3 = tpl("<!><!>");
function LangSwitch($p = {}) {
  const langs = alternates();
  const hint = suggest();
  const close = (e) => {
    dismiss();
    e.target.parentNode.setAttribute('hidden', '');
  };
  const $r14 = $t3();
  const $e1 = $r14.firstChild;
  when(null, () => langs.length, () => {
    const $r13 = $t2();
    const $e2 = $r13;
    const $e3 = $r13.firstChild.firstChild;
    const $e7 = $r13.firstChild.nextSibling.nextSibling;
    attr($e2, "aria-label", () => ($p.label ?? 'Language'));
    list(null, () => langs, (l) => l.locale, (l) => {
      const $r6 = $t0();
      const $e4 = $r6.firstChild;
      const $e5 = $e4.firstChild;
      attr($e4, "href", () => l.v.href);
      attr($e4, "hreflang", () => l.v.locale);
      attr($e4, "lang", () => l.v.locale);
      attr($e4, "aria-current", () => l.v.current ? 'page' : null);
      insert(null, () => l.v.name, $e5);
      return $r6;
    }, $e3);
    when(null, () => hint, () => {
      const $r12 = $t1();
      const $e8 = $r12;
      const $e9 = $r12.firstChild;
      const $e10 = $e9.firstChild;
      const $e11 = $e9.nextSibling.nextSibling;
      attr($e8, "lang", hint.locale);
      attr($e9, "href", hint.href);
      attr($e9, "hreflang", hint.locale);
      on($e9, "click", () => { dismiss(); });
      insert(null, hint.name, $e10);
      on($e11, "click", close);
      return $r12;
    }, null, $e7);
    return $r13;
  }, null, $e1);
  return $r14;
}
LangSwitch.h = "9dvubz";
export default dev(LangSwitch);
//# sourceMappingURL=data:application/json;base64,eyJ2ZXJzaW9uIjozLCJmaWxlIjoibGFuZy1zd2l0Y2guYnJhaW4uanMiLCJzb3VyY2VzIjpbImxhbmctc3dpdGNoLmJyYWluIl0sInNvdXJjZXNDb250ZW50IjpbIjxtZXRhIHByb3BzPVwibGFiZWw6c3RyPSdMYW5ndWFnZSdcIj5cbjxzY3JpcHQ+XG4gIC8vIGxpbmtzIHRvIHRoaXMgc2FtZSBwYWdlIGluIGV2ZXJ5IGxhbmd1YWdlIG9mIHRoZSBzaXRlIChub25lIHdpdGggYSBzaW5nbGUgbGFuZ3VhZ2UpOyBlYWNoIG5hbWUgaW4gaXRzZWxmXG4gIGNvbnN0IGxhbmdzID0gYWx0ZXJuYXRlcygpXG4gIC8vIHRoZSBicm93c2VyIHByZWZlcnMgYW5vdGhlciBsYW5ndWFnZSB0aGUgc2l0ZSBoYXM6IG9mZmVyZWQgb25jZSwgbmV2ZXIgYW4gYXV0b21hdGljIHJlZGlyZWN0LiBBIHVpLyBjb21wb25lbnQgaGFzXG4gIC8vIG5vIHN0YXRlIChCMDExNik6IGNsb3NpbmcgaGlkZXMgdGhlIGhpbnQncyBvd24gZWxlbWVudCBhbmQgcmVtZW1iZXJzIHRoZSBhbnN3ZXIuXG4gIGNvbnN0IGhpbnQgPSBzdWdnZXN0KClcbiAgY29uc3QgY2xvc2UgPSAoZSkgPT4ge1xuICAgIGRpc21pc3MoKVxuICAgIGUudGFyZ2V0LnBhcmVudE5vZGUuc2V0QXR0cmlidXRlKCdoaWRkZW4nLCAnJylcbiAgfVxuPC9zY3JpcHQ+XG57I2lmIGxhbmdzLmxlbmd0aH1cbiAgPG5hdiBjbGFzcz1cImxhbmdcIiBhcmlhLWxhYmVsPXtsYWJlbH0+XG4gICAgPHVsPlxuICAgICAgeyNlYWNoIGxhbmdzIGFzIGwgKGwubG9jYWxlKX1cbiAgICAgICAgPGxpPjxhIGhyZWY9e2wuaHJlZn0gaHJlZmxhbmc9e2wubG9jYWxlfSBsYW5nPXtsLmxvY2FsZX0gYXJpYS1jdXJyZW50PXtsLmN1cnJlbnQgPyAncGFnZScgOiBudWxsfT57bC5uYW1lfTwvYT48L2xpPlxuICAgICAgey9lYWNofVxuICAgIDwvdWw+XG4gICAgeyNpZiBoaW50fVxuICAgICAgPHAgY2xhc3M9XCJoaW50XCIgbGFuZz17aGludC5sb2NhbGV9PlxuICAgICAgICA8YSBocmVmPXtoaW50LmhyZWZ9IGhyZWZsYW5nPXtoaW50LmxvY2FsZX0gb246Y2xpY2s9e2Rpc21pc3MoKX0+e2hpbnQubmFtZX08L2E+XG4gICAgICAgIDxidXR0b24gdHlwZT1cImJ1dHRvblwiIGFyaWEtbGFiZWw9XCJEaXNtaXNzXCIgb246Y2xpY2s9e2Nsb3NlfT7DlzwvYnV0dG9uPlxuICAgICAgPC9wPlxuICAgIHsvaWZ9XG4gIDwvbmF2Plxuey9pZn1cbjxzdHlsZT5cbiAgLmxhbmcgeyBkaXNwbGF5OiBmbGV4OyBhbGlnbi1pdGVtczogY2VudGVyOyBnYXA6IHZhcigtLXNwYWNlLTMpOyBmb250OiAxNHB4LzEuMyB2YXIoLS1mb250KTsgfVxuICB1bCB7IGRpc3BsYXk6IGZsZXg7IGdhcDogdmFyKC0tc3BhY2UtMSk7IGxpc3Qtc3R5bGU6IG5vbmU7IG1hcmdpbjogMDsgcGFkZGluZzogMDsgfVxuICBhIHsgY29sb3I6IHZhcigtLW11dGVkKTsgdGV4dC1kZWNvcmF0aW9uOiBub25lOyBwYWRkaW5nOiB2YXIoLS1zcGFjZS0xKSB2YXIoLS1zcGFjZS0yKTsgYm9yZGVyLXJhZGl1czogdmFyKC0tcmFkaXVzLXNtKTsgfVxuICBhW2FyaWEtY3VycmVudD0ncGFnZSddIHsgY29sb3I6IHZhcigtLWZnKTsgYmFja2dyb3VuZDogdmFyKC0tc3VyZmFjZSk7IH1cbiAgYTpmb2N1cy12aXNpYmxlLCBidXR0b246Zm9jdXMtdmlzaWJsZSB7IG91dGxpbmU6IDJweCBzb2xpZCB2YXIoLS1hY2NlbnQpOyBvdXRsaW5lLW9mZnNldDogMnB4OyB9XG4gIC5oaW50IHsgZGlzcGxheTogZmxleDsgYWxpZ24taXRlbXM6IGNlbnRlcjsgZ2FwOiB2YXIoLS1zcGFjZS0xKTsgbWFyZ2luOiAwOyBwYWRkaW5nOiB2YXIoLS1zcGFjZS0xKSB2YXIoLS1zcGFjZS0yKTsgYm9yZGVyOiAxcHggc29saWQgdmFyKC0tZ2xhc3MtZWRnZSk7IGJvcmRlci1yYWRpdXM6IHZhcigtLXJhZGl1cy1zbSk7IGJhY2tncm91bmQ6IHZhcigtLWdsYXNzKTsgfVxuICAuaGludCBhIHsgY29sb3I6IHZhcigtLWFjY2VudCk7IH1cbiAgLmhpbnRbaGlkZGVuXSB7IGRpc3BsYXk6IG5vbmU7IH0gLyogY2xvc2VkOiBkaXNwbGF5OmZsZXggYWJvdmUgd291bGQgb3RoZXJ3aXNlIHdpbiBvdmVyIFtoaWRkZW5dICovXG4gIGJ1dHRvbiB7IGJvcmRlcjogMDsgYmFja2dyb3VuZDogdHJhbnNwYXJlbnQ7IGNvbG9yOiB2YXIoLS1tdXRlZCk7IGZvbnQtc2l6ZTogMTZweDsgY3Vyc29yOiBwb2ludGVyOyB9XG48L3N0eWxlPlxuIl0sIm5hbWVzIjpbXSwibWFwcGluZ3MiOiI7Ozs7Ozs7O0FBR0E7QUFHQTtBQUNBO0FBQ0E7QUFDQTtBQUFBO0FBQUE7QUFBQTtBQUdBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFDQTtBQUVBO0FBQUE7QUFBQTtBQUFBO0FBQ0E7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFHQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFDQTtBQUNBO0FBQUE7QUFBQTtBQUFBO0FBQ0E7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBOyJ9

