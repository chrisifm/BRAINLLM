import { sig, drv, fx, tpl, insert, attr, on, list, when } from "__RT__/brain.js";
import { dev } from "__RT__/brain-dev.js";
import Field from "./field.brain.js";
const $t0 = tpl("<option value=\"\" disabled><!></option>");
const $t1 = tpl("<option><!></option>");
const $t2 = tpl("<select data-b=\"6aoe0t\"><!> <!></select>");
const $t3 = tpl("<!><!>");
function Select($p = {}) {
  const items = drv(() => $p.options.map((o) => (typeof o === 'object' ? o : { value: String(o), label: String(o) })));
  const sel = sig(null, "sel");
  fx(() => {
    const n = items.v.length;
    if (!sel.v || !n) {
      return;
    }
    sel.v.value = $p.value || '';
  });
  const $r11 = $t3();
  const $e1 = $r11.firstChild;
  insert(null, Field({ get "label"() { return $p.label; }, get "error"() { return $p.error; }, get "help"() { return $p.help; }, children: () => {
      const $r10 = $t2();
      const $e2 = $r10;
      const $e3 = $r10.firstChild;
      const $e6 = $e3.nextSibling.nextSibling;
      sel.v = $e2;
      attr($e2, "name", () => $p.name);
      attr($e2, "required", () => ($p.required ?? false));
      attr($e2, "disabled", () => ($p.disabled ?? false));
      attr($e2, "aria-label", () => $p.label);
      attr($e2, "value", () => $p.value);
      on($e2, "change", (e) => { $p["bind:value"]?.(e.target.value); });
      when(null, () => $p.placeholder, () => {
        const $r5 = $t0();
        const $e4 = $r5.firstChild;
        insert(null, () => $p.placeholder, $e4);
        return $r5;
      }, null, $e3);
      list(null, () => items.v, (o) => o.value, (o) => {
        const $r9 = $t1();
        const $e7 = $r9;
        const $e8 = $r9.firstChild;
        attr($e7, "value", () => o.v.value);
        insert(null, () => o.v.label, $e8);
        return $r9;
      }, $e6);
      return $r10;
    } }), $e1);
  return $r11;
}
Select.h = "6aoe0t";
export default dev(Select);
//# sourceMappingURL=data:application/json;base64,eyJ2ZXJzaW9uIjozLCJmaWxlIjoic2VsZWN0LmJyYWluLmpzIiwic291cmNlcyI6WyJzZWxlY3QuYnJhaW4iXSwic291cmNlc0NvbnRlbnQiOlsiPG1ldGEgcHJvcHM9XCJsYWJlbDpzdHIgdmFsdWU6c3RyfiBvcHRpb25zOmxpc3QgcGxhY2Vob2xkZXI6c3RyPyBuYW1lOnN0cj8gcmVxdWlyZWQ6Ym9vbD1mYWxzZSBkaXNhYmxlZDpib29sPWZhbHNlIGVycm9yOnN0cj8gaGVscDpzdHI/XCI+XG48c2NyaXB0PlxuICAvLyBvcHRpb25zOiBbJ2EnLCAnYiddIG9yIFt7IHZhbHVlOiAnYScsIGxhYmVsOiAnQScgfV1cbiAgY29uc3QgaXRlbXMgPSBkcnYoKCkgPT4gb3B0aW9ucy5tYXAoKG8pID0+ICh0eXBlb2YgbyA9PT0gJ29iamVjdCcgPyBvIDogeyB2YWx1ZTogU3RyaW5nKG8pLCBsYWJlbDogU3RyaW5nKG8pIH0pKSlcbiAgbGV0IHNlbCA9IHJlZigpXG4gIC8vIGEgPHNlbGVjdD4gdmFsdWUgb25seSBzdGlja3Mgb25jZSBpdHMgb3B0aW9ucyBleGlzdDogcmUtYXBwbHkgaXQgYWZ0ZXIgdGhleSByZW5kZXJcbiAgZngoKCkgPT4ge1xuICAgIGNvbnN0IG4gPSBpdGVtcy5sZW5ndGhcbiAgICBpZiAoIXNlbCB8fCAhbikgcmV0dXJuXG4gICAgc2VsLnZhbHVlID0gdmFsdWUgfHwgJydcbiAgfSlcbjwvc2NyaXB0PlxuPEZpZWxkIGxhYmVsPXtsYWJlbH0gZXJyb3I9e2Vycm9yfSBoZWxwPXtoZWxwfT5cbiAgPHNlbGVjdCBiaW5kOnJlZj17c2VsfSBuYW1lPXtuYW1lfSByZXF1aXJlZD17cmVxdWlyZWR9IGRpc2FibGVkPXtkaXNhYmxlZH0gYXJpYS1sYWJlbD17bGFiZWx9IGJpbmQ6dmFsdWU9e3ZhbHVlfT5cbiAgICB7I2lmIHBsYWNlaG9sZGVyfTxvcHRpb24gdmFsdWU9XCJcIiBkaXNhYmxlZD57cGxhY2Vob2xkZXJ9PC9vcHRpb24+ey9pZn1cbiAgICB7I2VhY2ggaXRlbXMgYXMgbyAoby52YWx1ZSl9XG4gICAgICA8b3B0aW9uIHZhbHVlPXtvLnZhbHVlfT57by5sYWJlbH08L29wdGlvbj5cbiAgICB7L2VhY2h9XG4gIDwvc2VsZWN0PlxuPC9GaWVsZD5cbjxzdHlsZT5cbiAgc2VsZWN0IHtcbiAgICB3aWR0aDogMTAwJTsgbWluLWhlaWdodDogNDBweDsgcGFkZGluZzogdmFyKC0tc3BhY2UtMikgdmFyKC0tc3BhY2UtMyk7XG4gICAgZm9udDogMTVweC8xLjQgdmFyKC0tZm9udCk7IGNvbG9yOiB2YXIoLS1mZyk7IGJhY2tncm91bmQ6IHZhcigtLXN1cmZhY2UpO1xuICAgIGJvcmRlcjogMXB4IHNvbGlkIHZhcigtLWxpbmUpOyBib3JkZXItcmFkaXVzOiB2YXIoLS1yYWRpdXMtc20pO1xuICB9XG4gIHNlbGVjdDpmb2N1cy12aXNpYmxlIHsgb3V0bGluZTogMnB4IHNvbGlkIHZhcigtLWFjY2VudCk7IG91dGxpbmUtb2Zmc2V0OiAxcHg7IH1cbiAgc2VsZWN0W2FyaWEtaW52YWxpZD0ndHJ1ZSddIHsgYm9yZGVyLWNvbG9yOiB2YXIoLS1iYWQpOyB9XG48L3N0eWxlPlxuIl0sIm5hbWVzIjpbXSwibWFwcGluZ3MiOiI7Ozs7Ozs7O0FBR0E7QUFDQTtBQUVBO0FBQ0E7QUFDQTtBQUFBO0FBQUE7QUFDQTtBQUFBO0FBQUE7QUFBQTtBQUdBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFDQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUNBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUNBO0FBQUE7QUFBQTtBQUFBO0FBQ0E7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7In0=

