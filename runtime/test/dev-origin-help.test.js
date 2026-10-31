import { test } from 'node:test';
import assert from 'node:assert/strict';
import { existsSync, readFileSync } from 'node:fs';
import vm from 'node:vm';

const file = new URL('../../crates/brain-serve/src/origin_help.js', import.meta.url);

function sandbox({ fetch, WebSocket }) {
  const errors = [];
  const reports = [];
  const listeners = {};
  const context = {
    URL,
    Proxy,
    Reflect,
    Set,
    location: { href: 'https://shop.example.test/cart', origin: 'https://shop.example.test' },
    console: { error: m => errors.push(m) },
    report: r => reports.push(r),
    addEventListener: (name, fn) => { listeners[name] = fn; },
    fetch,
    WebSocket,
  };
  vm.runInNewContext(readFileSync(file, 'utf8'), context);
  return { context, errors, reports, listeners };
}
const settle = () => new Promise(resolve => setTimeout(resolve, 5));

test('a CORS block names both origins, the key and the fix, and keeps the original error', async () => {
  assert.ok(existsSync(file), 'origin_help.js exists');
  const failure = new TypeError('Failed to fetch');
  const calls = [];
  const fetch = (input, init) => {
    calls.push(init?.mode);
    return init?.mode === 'no-cors' ? Promise.resolve({ type: 'opaque' }) : Promise.reject(failure);
  };
  const { context, errors, reports } = sandbox({ fetch });
  await assert.rejects(context.fetch('https://api.example.test/items?token=hidden'), e => e === failure);
  await settle();
  assert.equal(errors.length, 1);
  for (const needle of ['B0747', 'https://api.example.test', 'https://shop.example.test', 'cors.origins', 'brain explain B0747']) {
    assert.ok(errors[0].includes(needle), `${needle} missing from ${errors[0]}`);
  }
  assert.ok(!errors[0].includes('hidden'), 'the query never reaches the console');
  assert.equal(reports[0].code, 'B0747');
  assert.deepEqual(calls, [undefined, 'no-cors']);
});

test('an unreachable address is told apart from a CORS refusal', async () => {
  const fetch = () => Promise.reject(new TypeError('Failed to fetch'));
  const { context, errors } = sandbox({ fetch });
  await assert.rejects(context.fetch('http://api.example.test/items'));
  await settle();
  assert.ok(errors[0].includes('did not answer') && errors[0].includes('connect-src'), errors[0]);
});

test('same-origin failures, aborts and successes stay silent and are not probed', async () => {
  let calls = 0;
  const abort = Object.assign(new Error('aborted'), { name: 'AbortError' });
  const answers = [Promise.reject(new TypeError('x')), Promise.reject(abort), Promise.resolve({ ok: true })];
  answers.slice(0, 2).forEach(p => p.catch(() => {}));
  const { context, errors } = sandbox({ fetch: () => answers[calls++] });
  await assert.rejects(context.fetch('/api/items'));
  await assert.rejects(context.fetch('https://api.example.test/a'));
  assert.deepEqual(await context.fetch('https://api.example.test/b'), { ok: true });
  await settle();
  assert.equal(calls, 3);
  assert.deepEqual(errors, []);
});

test('the same blocked origin is explained once', async () => {
  const fetch = () => Promise.reject(new TypeError('x'));
  const { context, errors } = sandbox({ fetch });
  await assert.rejects(context.fetch('https://api.example.test/a'));
  await assert.rejects(context.fetch('https://api.example.test/b'));
  await settle();
  assert.equal(errors.length, 1);
});

test('a live socket refused before opening names B0743; other sockets are left alone', () => {
  class Socket {
    constructor(address) { this.address = address; this.on = {}; }
    addEventListener(name, fn) { this.on[name] = fn; }
  }
  const { context, errors } = sandbox({ fetch: () => Promise.resolve({}), WebSocket: Socket });
  const opened = new context.WebSocket('wss://shop.example.test/_brain/socket');
  assert.ok(opened instanceof Socket);
  opened.on.open();
  opened.on.error();
  assert.deepEqual(errors, []);
  const refused = new context.WebSocket('wss://shop.example.test/_brain/socket');
  refused.on.error();
  assert.ok(errors[0].includes('B0743') && errors[0].includes('backend.sockets.allowedOrigins') && errors[0].includes('https://shop.example.test'), errors[0]);
  const other = new context.WebSocket('wss://shop.example.test/@brain/hmr');
  assert.equal(other.on.error, undefined);
});

test('a connect-src violation names the blocked origin and B0747', () => {
  const { errors, listeners } = sandbox({ fetch: () => Promise.resolve({}) });
  listeners.securitypolicyviolation({ effectiveDirective: 'connect-src', blockedURI: 'http://api.example.test/a?token=hidden' });
  listeners.securitypolicyviolation({ effectiveDirective: 'img-src', blockedURI: 'http://img.example.test/a' });
  assert.equal(errors.length, 1);
  assert.ok(errors[0].includes('B0747') && errors[0].includes('connect-src') && errors[0].includes('http://api.example.test') && !errors[0].includes('hidden'), errors[0]);
});
