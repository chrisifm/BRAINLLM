// SPEC §8 test API for `brain test`. The Rust runner drives it over CDP:
// __brainLoad(url) → { names } | { error }, then __brainStep(i) → { name, ok, ms, error? } for each test.
import { mount as rtMount } from '/@brain/brain.js';

const tests = [];
let root;
let dispose;
// Background Viewer tabs may suspend animation frames. Flush state without requiring foreground focus.
const flush = () => new Promise(resolve => {
  let frame, timer, done=false;
  const finish=()=>{if(done)return;done=true;clearTimeout(timer);if(frame!=null)cancelAnimationFrame(frame);resolve()};
  timer=setTimeout(finish,document.visibilityState==='hidden'?0:50);
  if(document.visibilityState!=='hidden')frame=requestAnimationFrame(()=>setTimeout(finish,0));
});
// A stubbed call is still answering, or its answer is still rendering: `mount`, `click` and `type` wait for both, so a
// test reads the result without timing it.
let answering = 0;
const settle = async () => {
  await flush();
  while (answering) { await new Promise(r => setTimeout(r, 5)); await flush() }
};
const fmt = v => (typeof v === 'string' ? JSON.stringify(v) : (() => { try { return JSON.stringify(v) ?? String(v) } catch { return String(v) } })());
// A component's ids are unique across instances (`email~2` from the second mount on, SPEC), so `#email` means this test's.
const scoped = sel => (/^#[\w-]+$/.test(sel) && !root.querySelector(sel) ? `[id^="${sel.slice(1)}~"]` : sel);
const q = sel => {
  const el = root.querySelector(scoped(sel));
  if (!el) throw new Error(`no element matches \`${sel}\` inside the mounted component`);
  return el;
};
const fail = msg => { throw new Error(msg) };

// `stub(target, answer)`: the next calls to a `server:` function ('save') or an endpoint ('GET /api/posts', with a
// `?query` to match it exactly) answer here instead of the network. Everything crosses `fetch`, so the runtime has no
// test code. The answer is a value (JSON, status 200), `reply(status, body)`, or a function of the call's arguments
// (of the parsed body for an endpoint) returning either. It lives for one test, and one nobody called fails it: a
// misspelled name would otherwise reach the real server and pass. An upload through XHR is not intercepted.
class Reply { constructor(status, body) { this.status = status; this.body = body } }
const stubs = [];
const realFetch = globalThis.fetch;
const jsonBody = init => { try { return typeof init?.body === 'string' ? JSON.parse(init.body) : init?.body } catch { return init.body } };
globalThis.fetch = async (input, init = {}) => {
  const raw = typeof input === 'string' ? input : input.url ?? String(input);
  const url = new URL(raw, location.href), method = (init.method || input.method || 'GET').toUpperCase();
  const rpc = /^(?:.*\/)?_r\/[^/]+\/([^/]+)$/.exec(url.pathname);
  const hit = stubs.find(s => (s.fn ? rpc && rpc[1] === s.fn : s.method === method && (s.path.includes('?') ? url.pathname + url.search : url.pathname) === s.path));
  if (!hit) return realFetch(input, init);
  const body = jsonBody(init);
  hit.record.calls.push(body);
  answering++;
  try {
    let answer = typeof hit.answer === 'function' ? await (hit.fn ? hit.answer(...(body || [])) : hit.answer(body)) : hit.answer;
    if (!(answer instanceof Reply)) answer = new Reply(200, answer);
    return new Response(answer.body === undefined ? '' : JSON.stringify(answer.body), { status: answer.status, headers: { 'content-type': 'application/json' } });
  } finally {
    setTimeout(() => answering--, 0);
  }
};

// `brain test --heap`: unmounts the last test's component, so what stays alive afterwards is a leak
globalThis.__brainDispose = () => { if (typeof dispose === 'function') dispose(); dispose = null; root && root.replaceChildren(); };

Object.assign(globalThis, {
  test: (name, fn) => tests.push({ name, fn }),
  stub: (target, answer) => {
    if (typeof target !== 'string' || !target) fail(`stub: the target is a server function name or 'GET /path', got ${fmt(target)}`);
    const http = /^(GET|POST|PUT|PATCH|DELETE) (\/\S*)$/.exec(target);
    if (!http && /[\s/]/.test(target)) fail(`stub: \`${target}\` is neither a server function name nor 'GET /path'`);
    const record = { calls: [] };
    stubs.unshift(http ? { target, method: http[1], path: http[2], answer, record } : { target, fn: target, answer, record });
    return record;
  },
  reply: (status, body) => new Reply(status, body),
  // `mount(Comp, props, { slot, slots, provide })`: `slot` is the default slot's text and `slots` maps a named slot
  // (`<slot name="hint" />`) to its text, so a component whose accessible text comes from its slot is tested (and
  // audited) with it. `provide` swaps services as in the runtime's mount.
  mount: async (Comp, props = {}, options = {}) => {
    const known = ['slot', 'slots', 'provide'], unknown = Object.keys(options).filter(k => !known.includes(k));
    if (unknown.length) fail(`mount: unknown option \`${unknown[0]}\`; use ${known.join(', ')}`);
    const slotText = (v, name) => {
      if (typeof v !== 'string' && typeof v !== 'number') fail(`mount: ${name} must be text (a string), got ${fmt(v)}`);
      return () => String(v);
    };
    const p = { ...props };
    if (options.slot != null) p.children = slotText(options.slot, '`slot`');
    for (const [name, v] of Object.entries(options.slots || {})) p['slot:' + name] = slotText(v, `\`slots.${name}\``);
    if (typeof dispose === 'function') dispose();
    root.replaceChildren();
    dispose = rtMount(Comp, root, p, options.provide ? { provide: options.provide } : undefined);
    await settle();
  },
  click: async sel => { q(sel).click(); await settle() },
  type: async (sel, text) => {
    const el = q(sel);
    el.focus();
    el.value = text;
    el.dispatchEvent(new Event('input', { bubbles: true }));
    await settle();
  },
  text: sel => q(sel).textContent.replace(/\s+/g, ' ').trim(),
  attr: (sel, name) => q(sel).getAttribute(name),
  hasClass: (sel, cls) => q(sel).classList.contains(cls),
  count: sel => root.querySelectorAll(scoped(sel)).length,
  expect: x => ({
    toBe: y => Object.is(x, y) || fail(`expected ${fmt(y)}, got ${fmt(x)}`),
    toEqual: y => JSON.stringify(x) === JSON.stringify(y) || fail(`expected ${fmt(y)}, got ${fmt(x)}`),
    toBeTruthy: () => !!x || fail(`expected a truthy value, got ${fmt(x)}`),
  }),
});

window.__brainLoad = async url => {
  root = document.getElementById('root');
  try {
    await import(url);
  } catch (e) {
    return { error: String(e?.message ?? e) };
  }
  return { names: tests.map(t => t.name) };
};

window.__brainStep = async i => {
  const t = tests[i];
  const t0 = performance.now();
  try {
    await t.fn();
    const idle = stubs.find(s => !s.record.calls.length);
    if (idle) fail(`stub('${idle.target}') was never called; fix: check the name, or remove the stub`);
    return { name: t.name, ok: true, ms: performance.now() - t0 };
  } catch (e) {
    return { name: t.name, ok: false, ms: performance.now() - t0, error: String(e?.message ?? e) };
  } finally {
    stubs.length = 0;
  }
};
