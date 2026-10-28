import { tpl, insert, attr, on, when } from "__RT__/brain.js";
import { dev } from "__RT__/brain-dev.js";
const $t0 = tpl("<p class=\"error\" role=\"alert\"><!></p>");
const $t1 = tpl("<div data-b=\"rw6vzg\" class=\"check\"><label><input type=\"checkbox\"> <span><!></span></label> <!></div>");
function Checkbox($p = {}) {
  const $r6 = $t1();
  const $e1 = $r6.firstChild.firstChild;
  const $e2 = $e1.nextSibling.nextSibling.firstChild;
  const $e3 = $r6.firstChild.nextSibling.nextSibling;
  attr($e1, "name", () => $p.name);
  attr($e1, "disabled", () => ($p.disabled ?? false));
  attr($e1, "aria-invalid", () => $p.error ? 'true' : null);
  attr($e1, "checked", () => $p.checked);
  on($e1, "change", (e) => { $p["bind:checked"]?.(e.target.checked); });
  insert(null, () => $p.label, $e2);
  when(null, () => $p.error, () => {
    const $r5 = $t0();
    const $e4 = $r5.firstChild;
    insert(null, () => $p.error, $e4);
    return $r5;
  }, null, $e3);
  return $r6;
}
Checkbox.h = "rw6vzg";
export default dev(Checkbox);
//# sourceMappingURL=data:application/json;base64,eyJ2ZXJzaW9uIjozLCJmaWxlIjoiY2hlY2tib3guYnJhaW4uanMiLCJzb3VyY2VzIjpbImNoZWNrYm94LmJyYWluIl0sInNvdXJjZXNDb250ZW50IjpbIjxtZXRhIHByb3BzPVwibGFiZWw6c3RyIGNoZWNrZWQ6Ym9vbH4gbmFtZTpzdHI/IGRpc2FibGVkOmJvb2w9ZmFsc2UgZXJyb3I6c3RyP1wiPlxuPGRpdiBjbGFzcz1cImNoZWNrXCI+XG4gIDxsYWJlbD5cbiAgICA8aW5wdXQgdHlwZT1cImNoZWNrYm94XCIgbmFtZT17bmFtZX0gZGlzYWJsZWQ9e2Rpc2FibGVkfSBhcmlhLWludmFsaWQ9e2Vycm9yID8gJ3RydWUnIDogbnVsbH0gYmluZDpjaGVja2VkPXtjaGVja2VkfT5cbiAgICA8c3Bhbj57bGFiZWx9PC9zcGFuPlxuICA8L2xhYmVsPlxuICB7I2lmIGVycm9yfTxwIGNsYXNzPVwiZXJyb3JcIiByb2xlPVwiYWxlcnRcIj57ZXJyb3J9PC9wPnsvaWZ9XG48L2Rpdj5cbjxzdHlsZT5cbiAgbGFiZWwgeyBkaXNwbGF5OiBpbmxpbmUtZmxleDsgYWxpZ24taXRlbXM6IGNlbnRlcjsgZ2FwOiB2YXIoLS1zcGFjZS0yKTsgZm9udDogMTVweC8xLjQgdmFyKC0tZm9udCk7IGNvbG9yOiB2YXIoLS1mZyk7IGN1cnNvcjogcG9pbnRlcjsgfVxuICBpbnB1dCB7IHdpZHRoOiAxOHB4OyBoZWlnaHQ6IDE4cHg7IGFjY2VudC1jb2xvcjogdmFyKC0tYWNjZW50KTsgfVxuICAuZXJyb3IgeyBmb250LXNpemU6IDEzcHg7IGNvbG9yOiB2YXIoLS1iYWQpOyBtYXJnaW46IHZhcigtLXNwYWNlLTEpIDAgMDsgfVxuPC9zdHlsZT5cbiJdLCJuYW1lcyI6W10sIm1hcHBpbmdzIjoiOzs7Ozs7Ozs7QUFHQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQ0E7QUFFQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTsifQ==

