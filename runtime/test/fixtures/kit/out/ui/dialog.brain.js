import { sig, tpl, insert, attr, on } from "__RT__/brain.js";
import { dev } from "__RT__/brain-dev.js";
import { expose } from "__RT__/brain-ch.js";
const $t0 = tpl("<dialog data-b=\"yxsdcn\"><header><h2><!></h2> <button type=\"button\" class=\"x\">×</button></header> <div class=\"body\"><!></div> <footer><!></footer></dialog>");
let $n = 0;
function Dialog($p = {}) {
  const $u = "-" + (++$n).toString(36);
  const dlg = sig(null, "dlg");
  const open = () => {
    if (!dlg.v.open) {
      dlg.v.showModal();
    }
  };
  const close = () => {
    if (dlg.v.open) {
      dlg.v.close();
    }
  };
  expose($p, { open, close });
  const $r7 = $t0();
  const $e1 = $r7;
  const $e2 = $r7.firstChild.firstChild;
  const $e3 = $e2.firstChild;
  const $e4 = $e2.nextSibling.nextSibling;
  const $e5 = $r7.firstChild.nextSibling.nextSibling.firstChild;
  const $e6 = $r7.firstChild.nextSibling.nextSibling.nextSibling.nextSibling.firstChild;
  dlg.v = $e1;
  attr($e1, "aria-labelledby", "dlg-title" + $u);
  on($e1, "close", () => { $p["on:close"]?.(); });
  attr($e2, "id", "dlg-title" + $u);
  insert(null, () => $p.title, $e3);
  attr($e4, "aria-label", () => ($p.closeLabel ?? 'Close'));
  on($e4, "click", close);
  insert(null, $p.children ? $p.children() : null, $e5);
  insert(null, $p["slot:footer"] ? $p["slot:footer"]() : null, $e6);
  return $r7;
}
Dialog.h = "yxsdcn";
export default dev(Dialog);
//# sourceMappingURL=data:application/json;base64,eyJ2ZXJzaW9uIjozLCJmaWxlIjoiZGlhbG9nLmJyYWluLmpzIiwic291cmNlcyI6WyJkaWFsb2cuYnJhaW4iXSwic291cmNlc0NvbnRlbnQiOlsiPG1ldGEgcHJvcHM9XCJ0aXRsZTpzdHIgY2xvc2VMYWJlbDpzdHI9J0Nsb3NlJ1wiIGVtaXRzPVwiY2xvc2VcIiBleHBvc2U9XCJvcGVuIGNsb3NlXCI+XG48c2NyaXB0PlxuICBsZXQgZGxnID0gcmVmKClcbiAgLy8gbmF0aXZlIG1vZGFsIDxkaWFsb2c+OiBmb2N1cyBtb3ZlcyBpbnNpZGUsIHRoZSByZXN0IG9mIHRoZSBwYWdlIGlzIGluZXJ0LCBFc2NhcGUgY2xvc2VzXG4gIGNvbnN0IG9wZW4gPSAoKSA9PiB7IGlmICghZGxnLm9wZW4pIGRsZy5zaG93TW9kYWwoKSB9XG4gIGNvbnN0IGNsb3NlID0gKCkgPT4geyBpZiAoZGxnLm9wZW4pIGRsZy5jbG9zZSgpIH1cbjwvc2NyaXB0PlxuPGRpYWxvZyBiaW5kOnJlZj17ZGxnfSBhcmlhLWxhYmVsbGVkYnk9XCJkbGctdGl0bGVcIiBvbjpjbG9zZT17ZW1pdCgnY2xvc2UnKX0+XG4gIDxoZWFkZXI+XG4gICAgPGgyIGlkPVwiZGxnLXRpdGxlXCI+e3RpdGxlfTwvaDI+XG4gICAgPGJ1dHRvbiB0eXBlPVwiYnV0dG9uXCIgY2xhc3M9XCJ4XCIgYXJpYS1sYWJlbD17Y2xvc2VMYWJlbH0gb246Y2xpY2s9e2Nsb3NlfT7DlzwvYnV0dG9uPlxuICA8L2hlYWRlcj5cbiAgPGRpdiBjbGFzcz1cImJvZHlcIj48c2xvdCAvPjwvZGl2PlxuICA8Zm9vdGVyPjxzbG90IG5hbWU9XCJmb290ZXJcIiAvPjwvZm9vdGVyPlxuPC9kaWFsb2c+XG48c3R5bGU+XG4gIGRpYWxvZyB7XG4gICAgd2lkdGg6IG1pbig1MjBweCwgY2FsYygxMDB2dyAtIDMycHgpKTsgcGFkZGluZzogMDsgYm9yZGVyOiAxcHggc29saWQgdmFyKC0tZ2xhc3MtZWRnZSk7XG4gICAgYm9yZGVyLXJhZGl1czogdmFyKC0tcmFkaXVzLWxnKTsgY29sb3I6IHZhcigtLWZnKTsgYmFja2dyb3VuZDogdmFyKC0tZ2xhc3MpOyBib3gtc2hhZG93OiB2YXIoLS1zaGFkb3cpO1xuICAgIGJhY2tkcm9wLWZpbHRlcjogYmx1cigyNHB4KSBzYXR1cmF0ZSgxLjQpO1xuICB9XG4gIGRpYWxvZzo6YmFja2Ryb3AgeyBiYWNrZ3JvdW5kOiB2YXIoLS1zY3JpbSk7IGJhY2tkcm9wLWZpbHRlcjogYmx1cig0cHgpOyB9XG4gIGhlYWRlciB7IGRpc3BsYXk6IGZsZXg7IGFsaWduLWl0ZW1zOiBjZW50ZXI7IGp1c3RpZnktY29udGVudDogc3BhY2UtYmV0d2VlbjsgcGFkZGluZzogdmFyKC0tc3BhY2UtNCkgdmFyKC0tc3BhY2UtNCkgMDsgfVxuICBoMiB7IGZvbnQ6IDY1MCAxOHB4LzEuMyB2YXIoLS1mb250KTsgbWFyZ2luOiAwOyB9XG4gIC54IHsgZm9udC1zaXplOiAyMnB4OyBsaW5lLWhlaWdodDogMTsgd2lkdGg6IDM2cHg7IGhlaWdodDogMzZweDsgYm9yZGVyOiAwOyBib3JkZXItcmFkaXVzOiB2YXIoLS1yYWRpdXMtc20pOyBjb2xvcjogdmFyKC0tbXV0ZWQpOyBiYWNrZ3JvdW5kOiB2YXIoLS1zdXJmYWNlKTsgY3Vyc29yOiBwb2ludGVyOyB9XG4gIC5ib2R5IHsgcGFkZGluZzogdmFyKC0tc3BhY2UtNCk7IH1cbiAgZm9vdGVyIHsgZGlzcGxheTogZmxleDsganVzdGlmeS1jb250ZW50OiBmbGV4LWVuZDsgZ2FwOiB2YXIoLS1zcGFjZS0yKTsgcGFkZGluZzogMCB2YXIoLS1zcGFjZS00KSB2YXIoLS1zcGFjZS00KTsgfVxuICBmb290ZXI6ZW1wdHkgeyBkaXNwbGF5OiBub25lOyB9XG48L3N0eWxlPlxuIl0sIm5hbWVzIjpbXSwibWFwcGluZ3MiOiI7Ozs7Ozs7QUFFQTtBQUVBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFDQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUVBO0FBQUE7QUFBQTtBQUVBO0FBQUE7QUFDQTtBQUFBO0FBRUE7QUFDQTtBQUFBO0FBQUE7QUFBQTtBQUFBOyJ9

