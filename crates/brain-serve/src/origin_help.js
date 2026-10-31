// Dev only, appended to the HMR client: the browser reports CORS and connect-src failures as an opaque TypeError and
// hides a refused WebSocket, so this names the likely cause and the fix. Calls keep their original promise and error.
;(() => {
  const told = new Set();
  const tell = (key, code, message) => {
    if (told.has(key)) return;
    told.add(key);
    console.error(message);
    report({ message, type: 'origin', code });
  };
  const originOf = address => { try { return new URL(address, location.href).origin; } catch { return null; } };
  const nativeFetch = globalThis.fetch;
  const explain = (input, error) => {
    if (error?.name === 'AbortError') return;
    const address = input?.url ?? String(input);
    const target = originOf(address);
    if (!target || target === 'null' || target === location.origin || told.has(target)) return;
    nativeFetch.call(globalThis, address, { mode: 'no-cors', method: 'HEAD', credentials: 'omit' }).then(
      () => tell(target, 'B0747', `B0747: the browser blocked the call to ${target} from ${location.origin}: the server answers, but its CORS headers do not allow this page. Fix: if it is this app's backend, add "${location.origin}" to cors.origins in server/platform.jsbrain; otherwise call it from a server function. Never a wildcard. Run brain explain B0747`),
      () => tell(target, 'B0747', `B0747: the call to ${target} from ${location.origin} failed: that server did not answer, or connect-src blocked it. Fix: check that it is running and uses https, or use a relative path through your own server. Run brain explain B0747`),
    );
  };
  globalThis.fetch = function (input, init) {
    const call = nativeFetch.call(this ?? globalThis, input, init);
    call.catch(error => explain(input, error));
    return call;
  };
  addEventListener('securitypolicyviolation', event => {
    if (event.effectiveDirective !== 'connect-src') return;
    const target = originOf(event.blockedURI) ?? 'the blocked address';
    tell(`csp ${target}`, 'B0747', `B0747: connect-src blocked the call to ${target} from ${location.origin}. Fix: use a relative path through your own server, or the https address. Run brain explain B0747`);
  });
  const NativeSocket = globalThis.WebSocket;
  if (!NativeSocket) return;
  globalThis.WebSocket = new Proxy(NativeSocket, {
    construct(Target, args, newTarget) {
      const socket = Reflect.construct(Target, args, newTarget);
      let path = '';
      try { path = new URL(args[0], location.href).pathname; } catch {}
      if (path.endsWith('/_brain/socket')) {
        let opened = false;
        socket.addEventListener('open', () => { opened = true; });
        socket.addEventListener('error', () => {
          if (!opened) tell('socket', 'B0743', `B0743: the live socket closed before opening from ${location.origin}, and the browser hides why. Likely fix: add "${location.origin}" to brain.json backend.sockets.allowedOrigins and backend.origins; the dev server log has the seen and allowed origins. Never a wildcard. Run brain explain B0743`);
        });
      }
      return socket;
    },
  });
})();
