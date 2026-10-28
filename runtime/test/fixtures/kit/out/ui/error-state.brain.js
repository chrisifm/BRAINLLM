import { fx, tpl, insert, attr, on, when } from "__RT__/brain.js";
import { dev } from "__RT__/brain-dev.js";
const $t0 = tpl("<a>Sign in</a>");
const $t1 = tpl("<button type=\"button\">Retry</button>");
const $t2 = tpl("<small>Reference: <!></small>");
const $t3 = tpl("<div data-b=\"oylky1\" class=\"error-state\" role=\"alert\"><p><!></p> <!> <!> <!></div>");
const $t4 = tpl("<!><!>");
function ErrorState($p = {}) {
  const text = (e) => {
    if (!e) {
      return '';
    }
    if (e.kind === 'offline') {
      return 'You are offline.';
    }
    if (e.kind === 'network') {
      return 'The connection was lost.';
    }
    if (e.kind === 'timeout') {
      return 'The server took too long to answer.';
    }
    if (e.kind === 'schema') {
      return 'The server sent something unexpected.';
    }
    if (e.status === 401) {
      return 'Sign in to continue.';
    }
    if (e.status === 403) {
      return e.message && e.message !== 'Forbidden' ? 'You do not have permission: ' + e.message : 'You do not have permission to see this.';
    }
    if (e.status === 404) {
      return 'Not found.';
    }
    if (e.status === 422) {
      return 'Check the highlighted fields.';
    }
    if (e.status === 429) {
      return 'Too many requests. Trying again in ' + (e.retryAfter || 30) + ' seconds.';
    }
    return 'Something went wrong on our side.';
  };
  const canRetry = (e) => ! !$p.retry && ! !e && (e.kind !== 'http' || e.status >= 500);
  const signIn = () => ($p.login ?? '/login') + (globalThis.location ? '?next=' + encodeURIComponent(location.pathname + location.search) : '');
  fx(() => {
    if (!$p.retry || !$p.error || ($p.error.kind !== 'offline' && $p.error.kind !== 'network') || !globalThis.addEventListener) {
      return;
    }
    const again = () => $p.retry();
    globalThis.addEventListener('online', again);
    return () => globalThis.removeEventListener('online', again);
  });
  fx(() => {
    if (!$p.retry || !$p.error || $p.error.status !== 429) {
      return;
    }
    const t = setTimeout(() => $p.retry(), ($p.error.retryAfter || 30) * 1000);
    return () => clearTimeout(t);
  });
  const $r13 = $t4();
  const $e1 = $r13.firstChild;
  when(null, () => $p.error && $p.error.kind !== 'aborted', () => {
    const $r12 = $t3();
    const $e2 = $r12.firstChild.firstChild;
    const $e3 = $r12.firstChild.nextSibling.nextSibling;
    const $e6 = $e3.nextSibling.nextSibling;
    const $e9 = $e6.nextSibling.nextSibling;
    insert(null, () => text($p.error), $e2);
    when(null, () => $p.error && $p.error.status === 401, () => {
      const $r5 = $t0();
      const $e4 = $r5;
      attr($e4, "href", () => signIn());
      return $r5;
    }, null, $e3);
    when(null, () => canRetry($p.error), () => {
      const $r8 = $t1();
      const $e7 = $r8;
      on($e7, "click", () => { $p.retry(); });
      return $r8;
    }, null, $e6);
    when(null, () => $p.error && $p.error.id && $p.error.status >= 500, () => {
      const $r11 = $t2();
      const $e10 = $r11.firstChild.nextSibling;
      insert(null, () => $p.error.id, $e10);
      return $r11;
    }, null, $e9);
    return $r12;
  }, null, $e1);
  return $r13;
}
ErrorState.h = "oylky1";
export default dev(ErrorState);
//# sourceMappingURL=data:application/json;base64,eyJ2ZXJzaW9uIjozLCJmaWxlIjoiZXJyb3Itc3RhdGUuYnJhaW4uanMiLCJzb3VyY2VzIjpbImVycm9yLXN0YXRlLmJyYWluIl0sInNvdXJjZXNDb250ZW50IjpbIjxtZXRhIHByb3BzPVwiZXJyb3I6YW55IHJldHJ5OmZuPyBsb2dpbjpzdHI9Jy9sb2dpbidcIj5cbjxzY3JpcHQ+XG4gIC8vIE9uZSBtZXNzYWdlIGFuZCBvbmUgYWN0aW9uIHBlciBlcnJvciBraW5kIChJTlRFUk5BTFMgwqc0MC4yKSwgZm9yIHs6Y2F0Y2ggZX0gYW5kIGZhaWxlZCByZXNvdXJjZXMuIE9mZmxpbmUgYW5kXG4gIC8vIG5ldHdvcmsgZXJyb3JzIHJldHJ5IGJ5IHRoZW1zZWx2ZXMgd2hlbiB0aGUgY29ubmVjdGlvbiBjb21lcyBiYWNrOyA0MjkgcmV0cmllcyBhZnRlciBpdHMgUmV0cnktQWZ0ZXIuIEEgY2FuY2VsbGVkXG4gIC8vIHJlcXVlc3QgKGtpbmQgJ2Fib3J0ZWQnKSBzaG93cyBub3RoaW5nLlxuICBjb25zdCB0ZXh0ID0gKGUpID0+IHtcbiAgICBpZiAoIWUpIHJldHVybiAnJ1xuICAgIGlmIChlLmtpbmQgPT09ICdvZmZsaW5lJykgcmV0dXJuICdZb3UgYXJlIG9mZmxpbmUuJ1xuICAgIGlmIChlLmtpbmQgPT09ICduZXR3b3JrJykgcmV0dXJuICdUaGUgY29ubmVjdGlvbiB3YXMgbG9zdC4nXG4gICAgaWYgKGUua2luZCA9PT0gJ3RpbWVvdXQnKSByZXR1cm4gJ1RoZSBzZXJ2ZXIgdG9vayB0b28gbG9uZyB0byBhbnN3ZXIuJ1xuICAgIGlmIChlLmtpbmQgPT09ICdzY2hlbWEnKSByZXR1cm4gJ1RoZSBzZXJ2ZXIgc2VudCBzb21ldGhpbmcgdW5leHBlY3RlZC4nXG4gICAgaWYgKGUuc3RhdHVzID09PSA0MDEpIHJldHVybiAnU2lnbiBpbiB0byBjb250aW51ZS4nXG4gICAgaWYgKGUuc3RhdHVzID09PSA0MDMpIHJldHVybiBlLm1lc3NhZ2UgJiYgZS5tZXNzYWdlICE9PSAnRm9yYmlkZGVuJyA/ICdZb3UgZG8gbm90IGhhdmUgcGVybWlzc2lvbjogJyArIGUubWVzc2FnZSA6ICdZb3UgZG8gbm90IGhhdmUgcGVybWlzc2lvbiB0byBzZWUgdGhpcy4nXG4gICAgaWYgKGUuc3RhdHVzID09PSA0MDQpIHJldHVybiAnTm90IGZvdW5kLidcbiAgICBpZiAoZS5zdGF0dXMgPT09IDQyMikgcmV0dXJuICdDaGVjayB0aGUgaGlnaGxpZ2h0ZWQgZmllbGRzLidcbiAgICBpZiAoZS5zdGF0dXMgPT09IDQyOSkgcmV0dXJuICdUb28gbWFueSByZXF1ZXN0cy4gVHJ5aW5nIGFnYWluIGluICcgKyAoZS5yZXRyeUFmdGVyIHx8IDMwKSArICcgc2Vjb25kcy4nXG4gICAgcmV0dXJuICdTb21ldGhpbmcgd2VudCB3cm9uZyBvbiBvdXIgc2lkZS4nXG4gIH1cbiAgY29uc3QgY2FuUmV0cnkgPSAoZSkgPT4gISFyZXRyeSAmJiAhIWUgJiYgKGUua2luZCAhPT0gJ2h0dHAnIHx8IGUuc3RhdHVzID49IDUwMClcbiAgY29uc3Qgc2lnbkluID0gKCkgPT4gbG9naW4gKyAoZ2xvYmFsVGhpcy5sb2NhdGlvbiA/ICc/bmV4dD0nICsgZW5jb2RlVVJJQ29tcG9uZW50KGxvY2F0aW9uLnBhdGhuYW1lICsgbG9jYXRpb24uc2VhcmNoKSA6ICcnKVxuICBmeCgoKSA9PiB7XG4gICAgaWYgKCFyZXRyeSB8fCAhZXJyb3IgfHwgKGVycm9yLmtpbmQgIT09ICdvZmZsaW5lJyAmJiBlcnJvci5raW5kICE9PSAnbmV0d29yaycpIHx8ICFnbG9iYWxUaGlzLmFkZEV2ZW50TGlzdGVuZXIpIHJldHVybiAvLyBubyB3aW5kb3cgZXZlbnRzIHdoaWxlIHRoZSBzZXJ2ZXIgcmVuZGVyc1xuICAgIGNvbnN0IGFnYWluID0gKCkgPT4gcmV0cnkoKVxuICAgIGdsb2JhbFRoaXMuYWRkRXZlbnRMaXN0ZW5lcignb25saW5lJywgYWdhaW4pXG4gICAgcmV0dXJuICgpID0+IGdsb2JhbFRoaXMucmVtb3ZlRXZlbnRMaXN0ZW5lcignb25saW5lJywgYWdhaW4pXG4gIH0pXG4gIGZ4KCgpID0+IHtcbiAgICBpZiAoIXJldHJ5IHx8ICFlcnJvciB8fCBlcnJvci5zdGF0dXMgIT09IDQyOSkgcmV0dXJuXG4gICAgY29uc3QgdCA9IHNldFRpbWVvdXQoKCkgPT4gcmV0cnkoKSwgKGVycm9yLnJldHJ5QWZ0ZXIgfHwgMzApICogMTAwMClcbiAgICByZXR1cm4gKCkgPT4gY2xlYXJUaW1lb3V0KHQpXG4gIH0pXG48L3NjcmlwdD5cbnsjaWYgZXJyb3IgJiYgZXJyb3Iua2luZCAhPT0gJ2Fib3J0ZWQnfVxuPGRpdiBjbGFzcz1cImVycm9yLXN0YXRlXCIgcm9sZT1cImFsZXJ0XCI+XG4gIDxwPnt0ZXh0KGVycm9yKX08L3A+XG4gIHsjaWYgZXJyb3IgJiYgZXJyb3Iuc3RhdHVzID09PSA0MDF9PGEgaHJlZj17c2lnbkluKCl9PlNpZ24gaW48L2E+ey9pZn1cbiAgeyNpZiBjYW5SZXRyeShlcnJvcil9PGJ1dHRvbiB0eXBlPVwiYnV0dG9uXCIgb246Y2xpY2s9e3JldHJ5KCl9PlJldHJ5PC9idXR0b24+ey9pZn1cbiAgeyNpZiBlcnJvciAmJiBlcnJvci5pZCAmJiBlcnJvci5zdGF0dXMgPj0gNTAwfTxzbWFsbD5SZWZlcmVuY2U6IHtlcnJvci5pZH08L3NtYWxsPnsvaWZ9XG48L2Rpdj5cbnsvaWZ9XG48c3R5bGU+XG4gIC5lcnJvci1zdGF0ZSB7XG4gICAgZGlzcGxheTogZ3JpZDsganVzdGlmeS1pdGVtczogc3RhcnQ7IGdhcDogdmFyKC0tc3BhY2UtMik7IHBhZGRpbmc6IHZhcigtLXNwYWNlLTQpOyBmb250OiAxNHB4LzEuNSB2YXIoLS1mb250KTtcbiAgICBjb2xvcjogdmFyKC0tZmcpOyBiYWNrZ3JvdW5kOiB2YXIoLS1nbGFzcyk7IGJvcmRlcjogMXB4IHNvbGlkIHZhcigtLWdsYXNzLWVkZ2UpOyBib3JkZXItcmFkaXVzOiB2YXIoLS1yYWRpdXMpO1xuICB9XG4gIHAgeyBtYXJnaW46IDA7IH1cbiAgYSB7IGNvbG9yOiB2YXIoLS1hY2NlbnQpOyB9XG4gIGJ1dHRvbiB7XG4gICAgZm9udDogaW5oZXJpdDsgcGFkZGluZzogdmFyKC0tc3BhY2UtMSkgdmFyKC0tc3BhY2UtMyk7IGJvcmRlcjogMXB4IHNvbGlkIHZhcigtLWxpbmUpOyBib3JkZXItcmFkaXVzOiB2YXIoLS1yYWRpdXMtc20pO1xuICAgIGNvbG9yOiB2YXIoLS1mZyk7IGJhY2tncm91bmQ6IHZhcigtLXN1cmZhY2UpOyBjdXJzb3I6IHBvaW50ZXI7XG4gIH1cbiAgc21hbGwgeyBjb2xvcjogdmFyKC0tbXV0ZWQpOyB9XG48L3N0eWxlPlxuIl0sIm5hbWVzIjpbXSwibWFwcGluZ3MiOiI7Ozs7Ozs7O0FBS0E7QUFDQTtBQUFBO0FBQUE7QUFDQTtBQUFBO0FBQUE7QUFDQTtBQUFBO0FBQUE7QUFDQTtBQUFBO0FBQUE7QUFDQTtBQUFBO0FBQUE7QUFDQTtBQUFBO0FBQUE7QUFDQTtBQUFBO0FBQUE7QUFDQTtBQUFBO0FBQUE7QUFDQTtBQUFBO0FBQUE7QUFDQTtBQUFBO0FBQUE7QUFDQTtBQUFBO0FBRUE7QUFDQTtBQUNBO0FBQ0E7QUFBQTtBQUFBO0FBQ0E7QUFDQTtBQUNBO0FBQUE7QUFFQTtBQUNBO0FBQUE7QUFBQTtBQUNBO0FBQ0E7QUFBQTtBQUFBO0FBQUE7QUFHQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFFQTtBQUNBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUNBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUNBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTtBQUFBO0FBQUE7QUFBQTsifQ==

