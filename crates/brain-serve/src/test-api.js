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
const fmt = v => (typeof v === 'string' ? JSON.stringify(v) : (() => { try { return JSON.stringify(v) ?? String(v) } catch { return String(v) } })());
const q = sel => {
  const el = root.querySelector(sel);
  if (!el) throw new Error(`no element matches \`${sel}\` inside the mounted component`);
  return el;
};
const fail = msg => { throw new Error(msg) };

// `brain test --heap`: unmounts the last test's component, so what stays alive afterwards is a leak
globalThis.__brainDispose = () => { if (typeof dispose === 'function') dispose(); dispose = null; root && root.replaceChildren(); };

Object.assign(globalThis, {
  test: (name, fn) => tests.push({ name, fn }),
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
    await flush();
  },
  click: async sel => { q(sel).click(); await flush() },
  type: async (sel, text) => {
    const el = q(sel);
    el.focus();
    el.value = text;
    el.dispatchEvent(new Event('input', { bubbles: true }));
    await flush();
  },
  text: sel => q(sel).textContent.replace(/\s+/g, ' ').trim(),
  attr: (sel, name) => q(sel).getAttribute(name),
  hasClass: (sel, cls) => q(sel).classList.contains(cls),
  count: sel => root.querySelectorAll(sel).length,
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
    return { name: t.name, ok: true, ms: performance.now() - t0 };
  } catch (e) {
    return { name: t.name, ok: false, ms: performance.now() - t0, error: String(e?.message ?? e) };
  }
};
