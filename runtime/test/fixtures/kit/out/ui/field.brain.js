import { sig, fx, tpl, insert, attr, on } from "__RT__/brain.js";
import { dev } from "__RT__/brain-dev.js";
const $t0 = tpl("<div data-b=\"2c0oim\" class=\"field\"><label><span class=\"label\"><!></span> <!></label> <p class=\"help\"><!></p> <p class=\"error\" role=\"alert\"><!></p></div>");
let $n = 0;
function Field($p = {}) {
  const $u = "-" + (++$n).toString(36);
  const box = sig(null, "box");
  const hint = sig(null, "hint");
  const err = sig(null, "err");
  fx(() => {
    if (!box.v || !hint.v || !err.v) {
      return;
    }
    const c = box.v.querySelector('input, select, textarea');
    if (!c) {
      return;
    }
    const ids = [];
    if ($p.help) {
      ids.push(hint.v.id);
    }
    if ($p.error) {
      ids.push(err.v.id);
    }
    if (ids.length) {
      c.setAttribute('aria-describedby', ids.join(' '));
    } else {
      c.removeAttribute('aria-describedby');
    }
    if ($p.error) {
      c.setAttribute('aria-invalid', 'true');
    } else {
      c.removeAttribute('aria-invalid');
    }
  });
  const $r8 = $t0();
  const $e1 = $r8;
  const $e2 = $r8.firstChild.firstChild.firstChild;
  const $e3 = $r8.firstChild.firstChild.nextSibling.nextSibling;
  const $e4 = $r8.firstChild.nextSibling.nextSibling;
  const $e5 = $e4.firstChild;
  const $e6 = $e4.nextSibling.nextSibling;
  const $e7 = $e6.firstChild;
  box.v = $e1;
  insert(null, () => $p.label, $e2);
  insert(null, $p.children ? $p.children() : null, $e3);
  attr($e4, "id", "hint" + $u);
  hint.v = $e4;
  attr($e4, "hidden", () => !$p.help);
  insert(null, () => $p.help, $e5);
  attr($e6, "id", "err" + $u);
  err.v = $e6;
  attr($e6, "hidden", () => !$p.error);
  insert(null, () => $p.error, $e7);
  return $r8;
}
Field.h = "2c0oim";
export default dev(Field);
//# sourceMappingURL=data:application/json;base64,eyJ2ZXJzaW9uIjozLCJmaWxlIjoiZmllbGQuYnJhaW4uanMiLCJzb3VyY2VzIjpbImZpZWxkLmJyYWluIl0sInNvdXJjZXNDb250ZW50IjpbIjxtZXRhIHByb3BzPVwibGFiZWw6c3RyIGVycm9yOnN0cj8gaGVscDpzdHI/XCI+XG48c2NyaXB0PlxuICBsZXQgYm94ID0gcmVmKClcbiAgbGV0IGhpbnQgPSByZWYoKVxuICBsZXQgZXJyID0gcmVmKClcbiAgLy8gbGlua3MgdGhlIHNsb3R0ZWQgY29udHJvbCB0byBpdHMgaGVscCBhbmQgZXJyb3IgdGV4dCAoYXJpYS1kZXNjcmliZWRieSAvIGFyaWEtaW52YWxpZClcbiAgZngoKCkgPT4ge1xuICAgIGlmICghYm94IHx8ICFoaW50IHx8ICFlcnIpIHJldHVyblxuICAgIGNvbnN0IGMgPSBib3gucXVlcnlTZWxlY3RvcignaW5wdXQsIHNlbGVjdCwgdGV4dGFyZWEnKVxuICAgIGlmICghYykgcmV0dXJuXG4gICAgY29uc3QgaWRzID0gW11cbiAgICBpZiAoaGVscCkgaWRzLnB1c2goaGludC5pZClcbiAgICBpZiAoZXJyb3IpIGlkcy5wdXNoKGVyci5pZClcbiAgICBpZiAoaWRzLmxlbmd0aCkgYy5zZXRBdHRyaWJ1dGUoJ2FyaWEtZGVzY3JpYmVkYnknLCBpZHMuam9pbignICcpKVxuICAgIGVsc2UgYy5yZW1vdmVBdHRyaWJ1dGUoJ2FyaWEtZGVzY3JpYmVkYnknKVxuICAgIGlmIChlcnJvcikgYy5zZXRBdHRyaWJ1dGUoJ2FyaWEtaW52YWxpZCcsICd0cnVlJylcbiAgICBlbHNlIGMucmVtb3ZlQXR0cmlidXRlKCdhcmlhLWludmFsaWQnKVxuICB9KVxuPC9zY3JpcHQ+XG48ZGl2IGNsYXNzPVwiZmllbGRcIiBiaW5kOnJlZj17Ym94fT5cbiAgPGxhYmVsPlxuICAgIDxzcGFuIGNsYXNzPVwibGFiZWxcIj57bGFiZWx9PC9zcGFuPlxuICAgIDxzbG90IC8+XG4gIDwvbGFiZWw+XG4gIDxwIGlkPVwiaGludFwiIGNsYXNzPVwiaGVscFwiIGJpbmQ6cmVmPXtoaW50fSBoaWRkZW49eyFoZWxwfT57aGVscH08L3A+XG4gIDxwIGlkPVwiZXJyXCIgY2xhc3M9XCJlcnJvclwiIHJvbGU9XCJhbGVydFwiIGJpbmQ6cmVmPXtlcnJ9IGhpZGRlbj17IWVycm9yfT57ZXJyb3J9PC9wPlxuPC9kaXY+XG48c3R5bGU+XG4gIC5maWVsZCB7IGRpc3BsYXk6IGdyaWQ7IGdhcDogdmFyKC0tc3BhY2UtMSk7IH1cbiAgbGFiZWwgeyBkaXNwbGF5OiBncmlkOyBnYXA6IHZhcigtLXNwYWNlLTEpOyB9XG4gIC5sYWJlbCB7IGZvbnQ6IDYwMCAxNHB4LzEuMyB2YXIoLS1mb250KTsgY29sb3I6IHZhcigtLWZnKTsgfVxuICAuaGVscCB7IGZvbnQtc2l6ZTogMTNweDsgY29sb3I6IHZhcigtLW11dGVkKTsgbWFyZ2luOiAwOyB9XG4gIC5lcnJvciB7IGZvbnQtc2l6ZTogMTNweDsgY29sb3I6IHZhcigtLWJhZCk7IG1hcmdpbjogMDsgfVxuPC9zdHlsZT5cbiJdLCJuYW1lcyI6W10sIm1hcHBpbmdzIjoiOzs7Ozs7QUFFQTtBQUNBO0FBQ0E7QUFFQTtBQUNBO0FBQUE7QUFBQTtBQUNBO0FBQ0E7QUFBQTtBQUFBO0FBQ0E7QUFDQTtBQUFBO0FBQUE7QUFDQTtBQUFBO0FBQUE7QUFDQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBRUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUlBO0FBRUE7QUFDQTtBQUVBO0FBQUE7QUFBQTtBQUFBO0FBQ0E7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTsifQ==

