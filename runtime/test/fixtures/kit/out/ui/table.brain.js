import { sig, drv, tpl, insert, attr, cls, on, list, when } from "__RT__/brain.js";
import { dev } from "__RT__/brain-dev.js";
const $t0 = tpl("<button type=\"button\"><!> <span aria-hidden=\"true\"><!></span></button>");
const $t1 = tpl("<!><!>");
const $t2 = tpl("<th scope=\"col\"><!></th>");
const $t3 = tpl("<tr><td class=\"msg bad\" role=\"alert\"><!></td></tr>");
const $t4 = tpl("<tr><td class=\"msg\">Loading…</td></tr>");
const $t5 = tpl("<tr class=\"pad\" aria-hidden=\"true\"></tr>");
const $t6 = tpl("<td><!></td>");
const $t7 = tpl("<tr><!></tr>");
const $t8 = tpl("<tr><td class=\"msg\"><!></td></tr>");
const $t9 = tpl("<!> <!> <!>");
const $t10 = tpl("<div data-b=\"yi1ja4\" class=\"wrap\"><table><caption><!></caption> <thead><tr><!></tr></thead> <tbody><!></tbody></table></div>");
function Table($p = {}) {
  const VIRTUAL_FROM = 200;
  const OVERSCAN = 10;
  const sortKey = sig('', "sortKey");
  const dir = sig(1, "dir");
  const top = sig(0, "top");
  const sorted = drv(() => {
    if (!sortKey.v) {
      return $p.rows;
    }
    const k = sortKey.v;
    const d = dir.v;
    return [...$p.rows].sort((a, b) => (a[k] > b[k] ? d : a[k] < b[k] ? -d : 0));
  });
  const virtual = drv(() => sorted.v.length > VIRTUAL_FROM);
  const first = drv(() => (virtual.v ? Math.max(0, Math.floor(top.v / ($p.rowHeight ?? 40)) - OVERSCAN) : 0));
  const last = drv(() => (virtual.v ? Math.min(sorted.v.length, first.v + Math.ceil(($p.height ?? 480) / ($p.rowHeight ?? 40)) + 2 * OVERSCAN) : sorted.v.length));
  const visible = drv(() => sorted.v.slice(first.v, last.v));
  const sortBy = (k) => {
    if (sortKey.v === k) {
      dir.v = -dir.v;
    } else {
      sortKey.v = k;
      dir.v = 1;
    }
  };
  const ariaSort = (k) => (sortKey.v === k ? (dir.v === 1 ? 'ascending' : 'descending') : 'none');
  const arrow = (k) => (sortKey.v === k ? (dir.v === 1 ? '▲' : '▼') : '');
  const onScroll = (e) => {
    top.v = e.target.scrollTop;
  };
  const $r37 = $t10();
  const $e1 = $r37;
  const $e2 = $r37.firstChild;
  const $e3 = $e2.firstChild.firstChild;
  const $e4 = $e2.firstChild.nextSibling.nextSibling.firstChild.firstChild;
  const $e14 = $e2.firstChild.nextSibling.nextSibling.nextSibling.nextSibling.firstChild;
  cls($e1, "virtual", () => virtual.v);
  attr($e1, "style:max-height", () => virtual.v ? ($p.height ?? 480) + 'px' : null);
  on($e1, "scroll", onScroll);
  attr($e2, "aria-busy", () => ($p.loading ?? false) ? 'true' : null);
  attr($e2, "aria-rowcount", () => sorted.v.length + 1);
  insert(null, () => $p.caption, $e3);
  list(null, () => $p.columns, (c) => c.key, (c) => {
    const $r13 = $t2();
    const $e5 = $r13;
    const $e6 = $r13.firstChild;
    attr($e5, "aria-sort", () => ariaSort(c.v.key));
    when(null, () => ($p.sortable ?? true) && c.v.sort !== false, () => {
      const $r10 = $t0();
      const $e7 = $r10;
      const $e8 = $r10.firstChild;
      const $e9 = $e8.nextSibling.nextSibling.firstChild;
      on($e7, "click", () => { sortBy(c.v.key); });
      insert(null, () => c.v.label, $e8);
      insert(null, () => arrow(c.v.key), $e9);
      return $r10;
    }, () => {
      const $r12 = $t1();
      const $e11 = $r12.firstChild;
      insert(null, () => c.v.label, $e11);
      return $r12;
    }, $e6);
    return $r13;
  }, $e4);
  when(null, () => $p.error, () => {
    const $r17 = $t3();
    const $e15 = $r17.firstChild;
    const $e16 = $e15.firstChild;
    attr($e15, "colspan", () => $p.columns.length);
    insert(null, () => $p.error, $e16);
    return $r17;
  }, () => {
    const $r18 = $t1();
    when(null, () => ($p.loading ?? false), () => {
      const $r20 = $t4();
      const $e19 = $r20.firstChild;
      attr($e19, "colspan", () => $p.columns.length);
      return $r20;
    }, () => {
      const $r36 = $t9();
      const $e21 = $r36.firstChild;
      const $e24 = $e21.nextSibling.nextSibling;
      const $e33 = $e24.nextSibling.nextSibling;
      when(null, () => first.v > 0, () => {
        const $r23 = $t5();
        const $e22 = $r23;
        attr($e22, "style:height", () => first.v * ($p.rowHeight ?? 40) + 'px');
        return $r23;
      }, null, $e21);
      list(null, () => visible.v, (r) => r[($p.rowKey ?? 'id')], (r) => {
        const $r29 = $t7();
        const $e25 = $r29;
        const $e26 = $r29.firstChild;
        attr($e25, "style:height", () => ($p.rowHeight ?? 40) + 'px');
        list(null, () => $p.columns, (c) => c.key, (c) => {
          const $r28 = $t6();
          const $e27 = $r28.firstChild;
          insert(null, () => r.v[c.v.key], $e27);
          return $r28;
        }, $e26);
        return $r29;
      }, $e24, () => {
        const $r32 = $t8();
        const $e30 = $r32.firstChild;
        const $e31 = $e30.firstChild;
        attr($e30, "colspan", () => $p.columns.length);
        insert(null, () => ($p.empty ?? 'No data yet.'), $e31);
        return $r32;
      });
      when(null, () => last.v < sorted.v.length, () => {
        const $r35 = $t5();
        const $e34 = $r35;
        attr($e34, "style:height", () => (sorted.v.length - last.v) * ($p.rowHeight ?? 40) + 'px');
        return $r35;
      }, null, $e33);
      return $r36;
    }, $r18.firstChild);
    return $r18;
  }, $e14);
  return $r37;
}
Table.h = "yi1ja4";
export default dev(Table);
//# sourceMappingURL=data:application/json;base64,eyJ2ZXJzaW9uIjozLCJmaWxlIjoidGFibGUuYnJhaW4uanMiLCJzb3VyY2VzIjpbInRhYmxlLmJyYWluIl0sInNvdXJjZXNDb250ZW50IjpbIjxtZXRhIHByb3BzPVwicm93czpsaXN0IGNvbHVtbnM6bGlzdCBjYXB0aW9uOnN0ciByb3dLZXk6c3RyPSdpZCcgc29ydGFibGU6Ym9vbD10cnVlIGxvYWRpbmc6Ym9vbD1mYWxzZSBlcnJvcjpzdHI/IGVtcHR5OnN0cj0nTm8gZGF0YSB5ZXQuJyByb3dIZWlnaHQ6bnVtPTQwIGhlaWdodDpudW09NDgwXCI+XG48c2NyaXB0PlxuICAvLyBjb2x1bW5zOiBbeyBrZXk6ICduYW1lJywgbGFiZWw6ICdOYW1lJywgc29ydD86IGZhbHNlIH1dIMK3IG1vcmUgdGhhbiAyMDAgcm93cyDihpIgb25seSB0aGUgdmlzaWJsZSB3aW5kb3cgaXMgcmVuZGVyZWRcbiAgY29uc3QgVklSVFVBTF9GUk9NID0gMjAwXG4gIGNvbnN0IE9WRVJTQ0FOID0gMTBcbiAgbGV0IHNvcnRLZXkgPSBzaWcoJycpXG4gIGxldCBkaXIgPSBzaWcoMSlcbiAgbGV0IHRvcCA9IHNpZygwKVxuICBjb25zdCBzb3J0ZWQgPSBkcnYoKCkgPT4ge1xuICAgIGlmICghc29ydEtleSkgcmV0dXJuIHJvd3NcbiAgICBjb25zdCBrID0gc29ydEtleVxuICAgIGNvbnN0IGQgPSBkaXJcbiAgICByZXR1cm4gWy4uLnJvd3NdLnNvcnQoKGEsIGIpID0+IChhW2tdID4gYltrXSA/IGQgOiBhW2tdIDwgYltrXSA/IC1kIDogMCkpXG4gIH0pXG4gIGNvbnN0IHZpcnR1YWwgPSBkcnYoKCkgPT4gc29ydGVkLmxlbmd0aCA+IFZJUlRVQUxfRlJPTSlcbiAgY29uc3QgZmlyc3QgPSBkcnYoKCkgPT4gKHZpcnR1YWwgPyBNYXRoLm1heCgwLCBNYXRoLmZsb29yKHRvcCAvIHJvd0hlaWdodCkgLSBPVkVSU0NBTikgOiAwKSlcbiAgY29uc3QgbGFzdCA9IGRydigoKSA9PiAodmlydHVhbCA/IE1hdGgubWluKHNvcnRlZC5sZW5ndGgsIGZpcnN0ICsgTWF0aC5jZWlsKGhlaWdodCAvIHJvd0hlaWdodCkgKyAyICogT1ZFUlNDQU4pIDogc29ydGVkLmxlbmd0aCkpXG4gIGNvbnN0IHZpc2libGUgPSBkcnYoKCkgPT4gc29ydGVkLnNsaWNlKGZpcnN0LCBsYXN0KSlcbiAgY29uc3Qgc29ydEJ5ID0gKGspID0+IHtcbiAgICBpZiAoc29ydEtleSA9PT0gaykgZGlyID0gLWRpclxuICAgIGVsc2UgeyBzb3J0S2V5ID0gazsgZGlyID0gMSB9XG4gIH1cbiAgY29uc3QgYXJpYVNvcnQgPSAoaykgPT4gKHNvcnRLZXkgPT09IGsgPyAoZGlyID09PSAxID8gJ2FzY2VuZGluZycgOiAnZGVzY2VuZGluZycpIDogJ25vbmUnKVxuICBjb25zdCBhcnJvdyA9IChrKSA9PiAoc29ydEtleSA9PT0gayA/IChkaXIgPT09IDEgPyAn4payJyA6ICfilrwnKSA6ICcnKVxuICBjb25zdCBvblNjcm9sbCA9IChlKSA9PiB7IHRvcCA9IGUudGFyZ2V0LnNjcm9sbFRvcCB9XG48L3NjcmlwdD5cbjxkaXYgY2xhc3M9XCJ3cmFwXCIgY2xhc3M6dmlydHVhbD17dmlydHVhbH0gc3R5bGU6bWF4LWhlaWdodD17dmlydHVhbCA/IGhlaWdodCArICdweCcgOiBudWxsfSBvbjpzY3JvbGw9e29uU2Nyb2xsfT5cbiAgPHRhYmxlIGFyaWEtYnVzeT17bG9hZGluZyA/ICd0cnVlJyA6IG51bGx9IGFyaWEtcm93Y291bnQ9e3NvcnRlZC5sZW5ndGggKyAxfT5cbiAgICA8Y2FwdGlvbj57Y2FwdGlvbn08L2NhcHRpb24+XG4gICAgPHRoZWFkPlxuICAgICAgPHRyPlxuICAgICAgICB7I2VhY2ggY29sdW1ucyBhcyBjIChjLmtleSl9XG4gICAgICAgICAgPHRoIHNjb3BlPVwiY29sXCIgYXJpYS1zb3J0PXthcmlhU29ydChjLmtleSl9PlxuICAgICAgICAgICAgeyNpZiBzb3J0YWJsZSAmJiBjLnNvcnQgIT09IGZhbHNlfVxuICAgICAgICAgICAgICA8YnV0dG9uIHR5cGU9XCJidXR0b25cIiBvbjpjbGljaz17c29ydEJ5KGMua2V5KX0+e2MubGFiZWx9IDxzcGFuIGFyaWEtaGlkZGVuPVwidHJ1ZVwiPnthcnJvdyhjLmtleSl9PC9zcGFuPjwvYnV0dG9uPlxuICAgICAgICAgICAgezplbHNlfVxuICAgICAgICAgICAgICB7Yy5sYWJlbH1cbiAgICAgICAgICAgIHsvaWZ9XG4gICAgICAgICAgPC90aD5cbiAgICAgICAgey9lYWNofVxuICAgICAgPC90cj5cbiAgICA8L3RoZWFkPlxuICAgIDx0Ym9keT5cbiAgICAgIHsjaWYgZXJyb3J9XG4gICAgICAgIDx0cj48dGQgY2xhc3M9XCJtc2cgYmFkXCIgY29sc3Bhbj17Y29sdW1ucy5sZW5ndGh9IHJvbGU9XCJhbGVydFwiPntlcnJvcn08L3RkPjwvdHI+XG4gICAgICB7OmVsc2UgaWYgbG9hZGluZ31cbiAgICAgICAgPHRyPjx0ZCBjbGFzcz1cIm1zZ1wiIGNvbHNwYW49e2NvbHVtbnMubGVuZ3RofT5Mb2FkaW5n4oCmPC90ZD48L3RyPlxuICAgICAgezplbHNlfVxuICAgICAgICB7I2lmIGZpcnN0ID4gMH08dHIgY2xhc3M9XCJwYWRcIiBhcmlhLWhpZGRlbj1cInRydWVcIiBzdHlsZTpoZWlnaHQ9e2ZpcnN0ICogcm93SGVpZ2h0ICsgJ3B4J30+PC90cj57L2lmfVxuICAgICAgICB7I2VhY2ggdmlzaWJsZSBhcyByIChyW3Jvd0tleV0pfVxuICAgICAgICAgIDx0ciBzdHlsZTpoZWlnaHQ9e3Jvd0hlaWdodCArICdweCd9PlxuICAgICAgICAgICAgeyNlYWNoIGNvbHVtbnMgYXMgYyAoYy5rZXkpfTx0ZD57cltjLmtleV19PC90ZD57L2VhY2h9XG4gICAgICAgICAgPC90cj5cbiAgICAgICAgezplbXB0eX1cbiAgICAgICAgICA8dHI+PHRkIGNsYXNzPVwibXNnXCIgY29sc3Bhbj17Y29sdW1ucy5sZW5ndGh9PntlbXB0eX08L3RkPjwvdHI+XG4gICAgICAgIHsvZWFjaH1cbiAgICAgICAgeyNpZiBsYXN0IDwgc29ydGVkLmxlbmd0aH08dHIgY2xhc3M9XCJwYWRcIiBhcmlhLWhpZGRlbj1cInRydWVcIiBzdHlsZTpoZWlnaHQ9eyhzb3J0ZWQubGVuZ3RoIC0gbGFzdCkgKiByb3dIZWlnaHQgKyAncHgnfT48L3RyPnsvaWZ9XG4gICAgICB7L2lmfVxuICAgIDwvdGJvZHk+XG4gIDwvdGFibGU+XG48L2Rpdj5cbjxzdHlsZT5cbiAgLndyYXAgeyBvdmVyZmxvdzogYXV0bzsgYm9yZGVyOiAxcHggc29saWQgdmFyKC0tZ2xhc3MtZWRnZSk7IGJvcmRlci1yYWRpdXM6IHZhcigtLXJhZGl1cyk7IGJhY2tncm91bmQ6IHZhcigtLWdsYXNzKTsgYm94LXNoYWRvdzogdmFyKC0tc2hhZG93KTsgfVxuICB0YWJsZSB7IHdpZHRoOiAxMDAlOyBib3JkZXItY29sbGFwc2U6IGNvbGxhcHNlOyBmb250OiAxNHB4LzEuNCB2YXIoLS1mb250KTsgY29sb3I6IHZhcigtLWZnKTsgZm9udC12YXJpYW50LW51bWVyaWM6IHRhYnVsYXItbnVtczsgfVxuICBjYXB0aW9uIHsgdGV4dC1hbGlnbjogbGVmdDsgZm9udC13ZWlnaHQ6IDY1MDsgcGFkZGluZzogdmFyKC0tc3BhY2UtMykgdmFyKC0tc3BhY2UtMykgdmFyKC0tc3BhY2UtMik7IH1cbiAgdGgsIHRkIHsgdGV4dC1hbGlnbjogbGVmdDsgcGFkZGluZzogdmFyKC0tc3BhY2UtMikgdmFyKC0tc3BhY2UtMyk7IGJvcmRlci1ib3R0b206IDFweCBzb2xpZCB2YXIoLS1saW5lKTsgfVxuICB0aGVhZCB0aCB7IHBvc2l0aW9uOiBzdGlja3k7IHRvcDogMDsgYmFja2dyb3VuZDogdmFyKC0tc3VyZmFjZSk7IGZvbnQtd2VpZ2h0OiA2MDA7IGNvbG9yOiB2YXIoLS1tdXRlZCk7IH1cbiAgdGggYnV0dG9uIHsgZm9udDogaW5oZXJpdDsgY29sb3I6IGluaGVyaXQ7IGJhY2tncm91bmQ6IG5vbmU7IGJvcmRlcjogMDsgcGFkZGluZzogMDsgY3Vyc29yOiBwb2ludGVyOyB9XG4gIHRoIGJ1dHRvbjpmb2N1cy12aXNpYmxlIHsgb3V0bGluZTogMnB4IHNvbGlkIHZhcigtLWFjY2VudCk7IG91dGxpbmUtb2Zmc2V0OiAycHg7IH1cbiAgLm1zZyB7IHRleHQtYWxpZ246IGNlbnRlcjsgY29sb3I6IHZhcigtLW11dGVkKTsgcGFkZGluZzogdmFyKC0tc3BhY2UtNSk7IH1cbiAgLm1zZy5iYWQgeyBjb2xvcjogdmFyKC0tYmFkKTsgfVxuICAucGFkIHRkIHsgcGFkZGluZzogMDsgYm9yZGVyOiAwOyB9XG48L3N0eWxlPlxuIl0sIm5hbWVzIjpbXSwibWFwcGluZ3MiOiI7Ozs7Ozs7Ozs7Ozs7O0FBR0E7QUFDQTtBQUNBO0FBQ0E7QUFDQTtBQUNBO0FBQ0E7QUFBQTtBQUFBO0FBQ0E7QUFDQTtBQUNBO0FBQUE7QUFFQTtBQUNBO0FBQ0E7QUFDQTtBQUNBO0FBQ0E7QUFBQTtBQUFBO0FBQ0E7QUFBQTtBQUFBO0FBQUE7QUFFQTtBQUNBO0FBQ0E7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBRUE7QUFBQTtBQUFBO0FBQ0E7QUFBQTtBQUNBO0FBR0E7QUFBQTtBQUFBO0FBQUE7QUFDQTtBQUNBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFDQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUVBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFPQTtBQUFBO0FBQUE7QUFBQTtBQUNBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFFQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUVBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUNBO0FBQUE7QUFBQTtBQUFBO0FBQ0E7QUFDQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBR0E7QUFBQTtBQUFBO0FBQUE7QUFFQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBOyJ9

