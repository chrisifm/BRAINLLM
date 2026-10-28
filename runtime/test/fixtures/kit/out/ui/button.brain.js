import { tpl, insert, attr, cls, on, when } from "__RT__/brain.js";
import { dev } from "__RT__/brain-dev.js";
const $t0 = tpl("<span class=\"spin\" aria-hidden=\"true\"></span>");
const $t1 = tpl("<button data-b=\"yasbzl\"><!> <!></button>");
function Button($p = {}) {
  const $r5 = $t1();
  const $e1 = $r5;
  const $e2 = $r5.firstChild;
  const $e4 = $e2.nextSibling.nextSibling;
  attr($e1, "type", () => ($p.type ?? 'button'));
  cls($e1, "ghost", () => ($p.variant ?? 'primary') === 'ghost');
  cls($e1, "danger", () => ($p.variant ?? 'primary') === 'danger');
  attr($e1, "disabled", () => ($p.disabled ?? false) || ($p.busy ?? false));
  attr($e1, "aria-busy", () => ($p.busy ?? false) ? 'true' : null);
  attr($e1, "aria-label", () => $p.label);
  on($e1, "click", () => { $p["on:click"]?.(); });
  when(null, () => ($p.busy ?? false), () => {
    const $r3 = $t0();
    return $r3;
  }, null, $e2);
  insert(null, $p.children ? $p.children() : null, $e4);
  return $r5;
}
Button.h = "yasbzl";
export default dev(Button);
//# sourceMappingURL=data:application/json;base64,eyJ2ZXJzaW9uIjozLCJmaWxlIjoiYnV0dG9uLmJyYWluLmpzIiwic291cmNlcyI6WyJidXR0b24uYnJhaW4iXSwic291cmNlc0NvbnRlbnQiOlsiPG1ldGEgcHJvcHM9XCJ2YXJpYW50OnN0cj0ncHJpbWFyeScgdHlwZTpzdHI9J2J1dHRvbicgYnVzeTpib29sPWZhbHNlIGRpc2FibGVkOmJvb2w9ZmFsc2UgbGFiZWw6c3RyP1wiIGVtaXRzPVwiY2xpY2tcIj5cbjxidXR0b25cbiAgdHlwZT17dHlwZX1cbiAgY2xhc3M6Z2hvc3Q9e3ZhcmlhbnQgPT09ICdnaG9zdCd9XG4gIGNsYXNzOmRhbmdlcj17dmFyaWFudCA9PT0gJ2Rhbmdlcid9XG4gIGRpc2FibGVkPXtkaXNhYmxlZCB8fCBidXN5fVxuICBhcmlhLWJ1c3k9e2J1c3kgPyAndHJ1ZScgOiBudWxsfVxuICBhcmlhLWxhYmVsPXtsYWJlbH1cbiAgb246Y2xpY2s9e2VtaXQoJ2NsaWNrJyl9XG4+XG4gIHsjaWYgYnVzeX08c3BhbiBjbGFzcz1cInNwaW5cIiBhcmlhLWhpZGRlbj1cInRydWVcIj48L3NwYW4+ey9pZn1cbiAgPHNsb3QgLz5cbjwvYnV0dG9uPlxuPHN0eWxlPlxuICBidXR0b24ge1xuICAgIGRpc3BsYXk6IGlubGluZS1mbGV4OyBhbGlnbi1pdGVtczogY2VudGVyOyBnYXA6IHZhcigtLXNwYWNlLTIpO1xuICAgIHBhZGRpbmc6IHZhcigtLXNwYWNlLTIpIHZhcigtLXNwYWNlLTQpOyBtaW4taGVpZ2h0OiA0MHB4O1xuICAgIGZvbnQ6IDYwMCAxNXB4LzEuMiB2YXIoLS1mb250KTsgYm9yZGVyLXJhZGl1czogdmFyKC0tcmFkaXVzLXNtKTsgY3Vyc29yOiBwb2ludGVyO1xuICAgIGNvbG9yOiB2YXIoLS1vbi1hY2NlbnQpOyBiYWNrZ3JvdW5kOiB2YXIoLS1hY2NlbnQpOyBib3JkZXI6IDFweCBzb2xpZCB2YXIoLS1hY2NlbnQpO1xuICAgIGJveC1zaGFkb3c6IHZhcigtLXNoYWRvdyk7XG4gIH1cbiAgYnV0dG9uLmdob3N0IHsgY29sb3I6IHZhcigtLWZnKTsgYmFja2dyb3VuZDogdmFyKC0tZ2xhc3MpOyBib3JkZXItY29sb3I6IHZhcigtLWdsYXNzLWVkZ2UpOyB9XG4gIGJ1dHRvbi5kYW5nZXIgeyBjb2xvcjogdmFyKC0tb24tYWNjZW50KTsgYmFja2dyb3VuZDogdmFyKC0tYmFkKTsgYm9yZGVyLWNvbG9yOiB2YXIoLS1iYWQpOyB9XG4gIGJ1dHRvbjpkaXNhYmxlZCB7IGN1cnNvcjogbm90LWFsbG93ZWQ7IGZpbHRlcjogZ3JheXNjYWxlKC40KTsgfVxuICBidXR0b246Zm9jdXMtdmlzaWJsZSB7IG91dGxpbmU6IDJweCBzb2xpZCB2YXIoLS1hY2NlbnQpOyBvdXRsaW5lLW9mZnNldDogMnB4OyB9XG4gIC5zcGluIHsgd2lkdGg6IDE0cHg7IGhlaWdodDogMTRweDsgYm9yZGVyLXJhZGl1czogNTAlOyBib3JkZXI6IDJweCBzb2xpZCBjdXJyZW50Q29sb3I7IGJvcmRlci1yaWdodC1jb2xvcjogdHJhbnNwYXJlbnQ7IH1cbiAgQG1lZGlhIChwcmVmZXJzLXJlZHVjZWQtbW90aW9uOiBuby1wcmVmZXJlbmNlKSB7XG4gICAgLnNwaW4geyBhbmltYXRpb246IHNwaW4gLjdzIGxpbmVhciBpbmZpbml0ZTsgfVxuICAgIEBrZXlmcmFtZXMgc3BpbiB7IHRvIHsgdHJhbnNmb3JtOiByb3RhdGUoMzYwZGVnKTsgfSB9XG4gIH1cbjwvc3R5bGU+XG4iXSwibmFtZXMiOltdLCJtYXBwaW5ncyI6Ijs7Ozs7Ozs7O0FBQ0E7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFTQTtBQUFBO0FBQUE7QUFBQTtBQUNBO0FBQUE7QUFBQTtBQUFBO0FBQUE7In0=

