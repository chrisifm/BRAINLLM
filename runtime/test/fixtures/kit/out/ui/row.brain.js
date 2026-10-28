import { tpl, insert, attr, cls } from "__RT__/brain.js";
import { dev } from "__RT__/brain-dev.js";
const $t0 = tpl("<div data-b=\"iaock1\" class=\"row\"><!></div>");
function Row($p = {}) {
  const sp = (n) => (n == null || n === '' ? null : String(n) === '0' ? '0' : 'var(--space-' + n + ')');
  const jc = (j) => (j === 'between' ? 'space-between' : j === 'start' || j === 'end' ? 'flex-' + j : j);
  const $r3 = $t0();
  const $e1 = $r3;
  const $e2 = $r3.firstChild;
  cls($e1, "nowrap", () => !($p.wrap ?? true));
  attr($e1, "style:gap", () => sp(($p.gap ?? '3')));
  attr($e1, "style:align-items", () => ($p.align ?? 'center'));
  attr($e1, "style:justify-content", () => jc(($p.justify ?? 'start')));
  insert(null, $p.children ? $p.children() : null, $e2);
  return $r3;
}
Row.h = "iaock1";
export default dev(Row);
//# sourceMappingURL=data:application/json;base64,eyJ2ZXJzaW9uIjozLCJmaWxlIjoicm93LmJyYWluLmpzIiwic291cmNlcyI6WyJyb3cuYnJhaW4iXSwic291cmNlc0NvbnRlbnQiOlsiPG1ldGEgcHJvcHM9XCJnYXA6c3RyPSczJyBhbGlnbjpzdHI9J2NlbnRlcicganVzdGlmeTpzdHI9J3N0YXJ0JyB3cmFwOmJvb2w9dHJ1ZVwiPlxuPHNjcmlwdD5cbiAgLy8gYSBob3Jpem9udGFsIGZsb3cgdGhhdCB3cmFwcyBvbiBuYXJyb3cgc2NyZWVuczsganVzdGlmeTogc3RhcnQgfCBjZW50ZXIgfCBlbmQgfCBiZXR3ZWVuXG4gIC8vIHNwYWNpbmcgcHJvcHMgdGFrZSBhIHRva2VuIHN0ZXAgZnJvbSB0b2tlbnMuY3NzYnJhaW4gKDDigJM2IOKGkiAtLXNwYWNlLU4pLCBuZXZlciByYXcgQ1NTXG4gIGNvbnN0IHNwID0gKG4pID0+IChuID09IG51bGwgfHwgbiA9PT0gJycgPyBudWxsIDogU3RyaW5nKG4pID09PSAnMCcgPyAnMCcgOiAndmFyKC0tc3BhY2UtJyArIG4gKyAnKScpXG4gIGNvbnN0IGpjID0gKGopID0+IChqID09PSAnYmV0d2VlbicgPyAnc3BhY2UtYmV0d2VlbicgOiBqID09PSAnc3RhcnQnIHx8IGogPT09ICdlbmQnID8gJ2ZsZXgtJyArIGogOiBqKVxuPC9zY3JpcHQ+XG48ZGl2IGNsYXNzPVwicm93XCIgY2xhc3M6bm93cmFwPXshd3JhcH0gc3R5bGU6Z2FwPXtzcChnYXApfSBzdHlsZTphbGlnbi1pdGVtcz17YWxpZ259IHN0eWxlOmp1c3RpZnktY29udGVudD17amMoanVzdGlmeSl9PjxzbG90IC8+PC9kaXY+XG48c3R5bGU+XG4gIC5yb3cgeyBkaXNwbGF5OiBmbGV4OyBmbGV4LXdyYXA6IHdyYXA7IG1pbi13aWR0aDogMDsgfVxuICAucm93Lm5vd3JhcCB7IGZsZXgtd3JhcDogbm93cmFwOyB9XG48L3N0eWxlPlxuIl0sIm5hbWVzIjpbXSwibWFwcGluZ3MiOiI7Ozs7QUFJQTtBQUNBO0FBQUE7QUFBQTtBQUFBO0FBRUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBOyJ9

