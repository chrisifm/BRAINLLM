import { sig, drv, tpl, insert, attr, on, list as $list } from "__RT__/brain.js";
import { dev } from "__RT__/brain-dev.js";
const $t0 = tpl("<button type=\"button\" role=\"tab\"><!></button>");
const $t1 = tpl("<div data-b=\"a0l4ls\" class=\"tabs\"><div class=\"list\" role=\"tablist\"><!></div> <div class=\"panel\" role=\"tabpanel\" tabindex=\"0\"><!></div></div>");
function Tabs($p = {}) {
  const list = sig(null, "list");
  const current = drv(() => $p.tabs.find((t) => t.id === $p.value) || $p.tabs[0]);
  const keys = (e) => {
    const i = $p.tabs.findIndex((t) => t.id === $p.value);
    let j = -1;
    if (e.key === 'ArrowRight') {
      j = (i + 1) % $p.tabs.length;
    } else {
      if (e.key === 'ArrowLeft') {
        j = (i - 1 + $p.tabs.length) % $p.tabs.length;
      } else {
        if (e.key === 'Home') {
          j = 0;
        } else {
          if (e.key === 'End') {
            j = $p.tabs.length - 1;
          }
        }
      }
    }
    if (j < 0) {
      return;
    }
    e.preventDefault();
    $p["bind:value"]?.($p.tabs[j].id);
    list.v.querySelectorAll('[role=tab]')[j].focus();
  };
  const $r8 = $t1();
  const $e1 = $r8.firstChild;
  const $e2 = $e1.firstChild;
  const $e6 = $e1.nextSibling.nextSibling;
  const $e7 = $e6.firstChild;
  attr($e1, "aria-label", () => $p.label);
  list.v = $e1;
  on($e1, "keydown", keys);
  $list(null, () => $p.tabs, (t) => t.id, (t) => {
    const $r5 = $t0();
    const $e3 = $r5;
    const $e4 = $r5.firstChild;
    attr($e3, "aria-selected", () => t.v.id === $p.value ? 'true' : 'false');
    attr($e3, "tabindex", () => t.v.id === $p.value ? 0 : -1);
    on($e3, "click", () => { $p["bind:value"]?.(t.v.id); });
    insert(null, () => t.v.label, $e4);
    return $r5;
  }, $e2);
  attr($e6, "aria-label", () => current.v ? current.v.label : $p.label);
  insert(null, $p.children ? $p.children() : null, $e7);
  return $r8;
}
Tabs.h = "a0l4ls";
export default dev(Tabs);
//# sourceMappingURL=data:application/json;base64,eyJ2ZXJzaW9uIjozLCJmaWxlIjoidGFicy5icmFpbi5qcyIsInNvdXJjZXMiOlsidGFicy5icmFpbiJdLCJzb3VyY2VzQ29udGVudCI6WyI8bWV0YSBwcm9wcz1cInRhYnM6bGlzdCB2YWx1ZTpzdHJ+IGxhYmVsOnN0clwiPlxuPHNjcmlwdD5cbiAgLy8gdGFiczogW3sgaWQ6ICdhJywgbGFiZWw6ICdPdmVydmlldycgfV0gwrcgdGhlIHBhcmVudCByZW5kZXJzIHRoZSBwYW5lbCBvZiBgdmFsdWVgIGluIHRoZSBzbG90XG4gIGxldCBsaXN0ID0gcmVmKClcbiAgY29uc3QgY3VycmVudCA9IGRydigoKSA9PiB0YWJzLmZpbmQoKHQpID0+IHQuaWQgPT09IHZhbHVlKSB8fCB0YWJzWzBdKVxuICBjb25zdCBrZXlzID0gKGUpID0+IHtcbiAgICBjb25zdCBpID0gdGFicy5maW5kSW5kZXgoKHQpID0+IHQuaWQgPT09IHZhbHVlKVxuICAgIGxldCBqID0gLTFcbiAgICBpZiAoZS5rZXkgPT09ICdBcnJvd1JpZ2h0JykgaiA9IChpICsgMSkgJSB0YWJzLmxlbmd0aFxuICAgIGVsc2UgaWYgKGUua2V5ID09PSAnQXJyb3dMZWZ0JykgaiA9IChpIC0gMSArIHRhYnMubGVuZ3RoKSAlIHRhYnMubGVuZ3RoXG4gICAgZWxzZSBpZiAoZS5rZXkgPT09ICdIb21lJykgaiA9IDBcbiAgICBlbHNlIGlmIChlLmtleSA9PT0gJ0VuZCcpIGogPSB0YWJzLmxlbmd0aCAtIDFcbiAgICBpZiAoaiA8IDApIHJldHVyblxuICAgIGUucHJldmVudERlZmF1bHQoKVxuICAgIHZhbHVlID0gdGFic1tqXS5pZFxuICAgIGxpc3QucXVlcnlTZWxlY3RvckFsbCgnW3JvbGU9dGFiXScpW2pdLmZvY3VzKClcbiAgfVxuPC9zY3JpcHQ+XG48ZGl2IGNsYXNzPVwidGFic1wiPlxuICA8ZGl2IGNsYXNzPVwibGlzdFwiIHJvbGU9XCJ0YWJsaXN0XCIgYXJpYS1sYWJlbD17bGFiZWx9IGJpbmQ6cmVmPXtsaXN0fSBvbjprZXlkb3duPXtrZXlzfT5cbiAgICB7I2VhY2ggdGFicyBhcyB0ICh0LmlkKX1cbiAgICAgIDxidXR0b24gdHlwZT1cImJ1dHRvblwiIHJvbGU9XCJ0YWJcIiBhcmlhLXNlbGVjdGVkPXt0LmlkID09PSB2YWx1ZSA/ICd0cnVlJyA6ICdmYWxzZSd9IHRhYmluZGV4PXt0LmlkID09PSB2YWx1ZSA/IDAgOiAtMX0gb246Y2xpY2s9e3ZhbHVlID0gdC5pZH0+e3QubGFiZWx9PC9idXR0b24+XG4gICAgey9lYWNofVxuICA8L2Rpdj5cbiAgPGRpdiBjbGFzcz1cInBhbmVsXCIgcm9sZT1cInRhYnBhbmVsXCIgYXJpYS1sYWJlbD17Y3VycmVudCA/IGN1cnJlbnQubGFiZWwgOiBsYWJlbH0gdGFiaW5kZXg9XCIwXCI+PHNsb3QgLz48L2Rpdj5cbjwvZGl2PlxuPHN0eWxlPlxuICAubGlzdCB7IGRpc3BsYXk6IGZsZXg7IGdhcDogdmFyKC0tc3BhY2UtMSk7IHBhZGRpbmc6IHZhcigtLXNwYWNlLTEpOyBib3JkZXItcmFkaXVzOiB2YXIoLS1yYWRpdXMpOyBiYWNrZ3JvdW5kOiB2YXIoLS1nbGFzcyk7IGJvcmRlcjogMXB4IHNvbGlkIHZhcigtLWdsYXNzLWVkZ2UpOyB3aWR0aDogZml0LWNvbnRlbnQ7IH1cbiAgW3JvbGU9dGFiXSB7IGZvbnQ6IDYwMCAxNHB4LzEuMiB2YXIoLS1mb250KTsgcGFkZGluZzogdmFyKC0tc3BhY2UtMikgdmFyKC0tc3BhY2UtMyk7IGJvcmRlcjogMDsgYm9yZGVyLXJhZGl1czogdmFyKC0tcmFkaXVzLXNtKTsgY29sb3I6IHZhcigtLW11dGVkKTsgYmFja2dyb3VuZDogdHJhbnNwYXJlbnQ7IGN1cnNvcjogcG9pbnRlcjsgfVxuICBbcm9sZT10YWJdW2FyaWEtc2VsZWN0ZWQ9J3RydWUnXSB7IGNvbG9yOiB2YXIoLS1mZyk7IGJhY2tncm91bmQ6IHZhcigtLXN1cmZhY2UpOyBib3gtc2hhZG93OiB2YXIoLS1zaGFkb3cpOyB9XG4gIFtyb2xlPXRhYl06Zm9jdXMtdmlzaWJsZSwgLnBhbmVsOmZvY3VzLXZpc2libGUgeyBvdXRsaW5lOiAycHggc29saWQgdmFyKC0tYWNjZW50KTsgb3V0bGluZS1vZmZzZXQ6IDJweDsgfVxuICAucGFuZWwgeyBwYWRkaW5nOiB2YXIoLS1zcGFjZS00KSAwOyB9XG48L3N0eWxlPlxuIl0sIm5hbWVzIjpbXSwibWFwcGluZ3MiOiI7Ozs7O0FBR0E7QUFDQTtBQUNBO0FBQ0E7QUFDQTtBQUNBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUlBO0FBQUE7QUFBQTtBQUNBO0FBQ0E7QUFDQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUlBO0FBQUE7QUFBQTtBQUNBO0FBQUE7QUFBQTtBQUFBO0FBQ0E7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBR0E7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBOyJ9

