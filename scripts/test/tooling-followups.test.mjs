import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, readdirSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { spawn } from 'node:child_process';
import { createServer } from 'node:http';
import { createInterface } from 'node:readline';

const binary = process.env.BRAIN_BIN && resolve(process.env.BRAIN_BIN);
function run(root, args, timeout = 15000) {
  assert.ok(binary, 'BRAIN_BIN must select the exact integrated binary');
  return new Promise((accept, reject) => {
    const child = spawn(binary, args, { cwd: root, stdio: ['ignore', 'pipe', 'pipe'] });
    let stdout = '', stderr = '', over = false;
    const timer = setTimeout(() => { over = true; child.kill('SIGKILL'); }, timeout);
    for (const [stream, name] of [[child.stdout, 'out'], [child.stderr, 'err']]) stream.on('data', data => {
      if (name === 'out') stdout += data; else stderr += data;
      if (stdout.length + stderr.length > 2 * 1024 * 1024) { over = true; child.kill('SIGKILL'); }
    });
    child.once('error', error => { clearTimeout(timer); reject(error); });
    child.once('close', (status, signal) => { clearTimeout(timer); if (over) reject(Error('bounded tooling fixture exceeded time/output limit')); else accept({ status, signal, stdout, stderr }); });
  });
}
function fixture() {
  const root = mkdtempSync(join(tmpdir(), 'brain-tooling-cli-'));
  writeFileSync(join(root, 'tokens.cssbrain'), '');
  writeFileSync(join(root, 'brain.json'), '{"name":"tooling-fixture","docs":false}');
  return root;
}
function parse(result, code = 0) {
  assert.equal(result.status, code, result.stderr || result.stdout);
  const value = JSON.parse(result.stdout);
  assert.equal(value.schemaVersion, 1);
  return value;
}

test('bench observes one anonymous URL with bounded bodies, no redirect and no retry', { timeout: 45000 }, async () => {
  const root = fixture();
  const requests = [];
  const server = createServer((req, res) => {
    requests.push({ path: req.url, cookie: req.headers.cookie, authorization: req.headers.authorization });
    if (req.url === '/redirect') { res.writeHead(302, { location: '/destination' }); res.end(); }
    else if (req.url === '/large') { res.writeHead(200, { 'content-length': 262145 }); res.end('x'.repeat(262145)); }
    else { res.writeHead(200, { 'content-type': 'text/plain' }); res.end('observed'); }
  });
  await new Promise(accept => server.listen(0, '127.0.0.1', accept));
  const base = `http://127.0.0.1:${server.address().port}`;
  try {
    const observed = parse(await run(root, ['bench', '--url', `${base}/page`, '--json']));
    assert.equal(observed.requested, 3); assert.equal(observed.completed, 3);
    assert.equal(observed.firstObserved.phase, 'firstObserved');
    assert.equal(observed.repeatObserved.count, 2);
    assert.ok(observed.observations.every(row => row.status === 200 && row.bodyBytes === 8 && row.totalMs >= row.ttfbMs));
    assert.deepEqual(requests.map(row => row.path), ['/page', '/page', '/page']);
    assert.ok(requests.every(row => row.cookie === undefined && row.authorization === undefined));
    assert.equal(parse(await run(root, ['bench', '--url', `${base}/redirect`, '--requests', '1', '--json']), 1).observations[0].status, 302);
    assert.equal(parse(await run(root, ['bench', '--url', `${base}/large`, '--requests', '1', '--json']), 1).observations[0].error, 'response-too-large');
    assert.equal(requests.length, 5); assert.ok(!requests.some(row => row.path === '/destination'));
    for (const args of [['--url', `${base}/?secret=value`], ['--url', `${base}/`, '--requests', '21'], ['--url', `${base}/`, '--unknown']]) {
      assert.equal((await run(root, ['bench', ...args, '--json'])).status, 2);
    }
    assert.equal(requests.length, 5, 'invalid inputs must not access the network');
  } finally { await new Promise(accept => server.close(accept)); rmSync(root, { recursive: true, force: true }); }
});

test('adopt and practices are read-only while connect handshakes only its owned MCP child', { timeout: 30000 }, async () => {
  const root = fixture();
  try {
    mkdirSync(join(root, 'src/pages'), { recursive: true });
    writeFileSync(join(root, 'package.json'), '{"name":"private-package","dependencies":{"react":"private-version"},"scripts":{"audit":"touch private-marker"}}');
    writeFileSync(join(root, 'src/pages/item.tsx'), 'throw new Error("private-source")');
    writeFileSync(join(root, '.brain_env'), 'PRIVATE_VALUE=private-environment');
    const before = readdirSync(root).sort();
    const report = parse(await run(root, ['adopt', '--audit', '--json']));
    assert.deepEqual(report.frameworks, ['React']); assert.equal(report.sourceCount, 2);
    assert.ok(report.sourceFiles.includes('src/pages/item.tsx'));
    assert.equal(report.inspection.sourceBodiesRead, false); assert.equal(report.inspection.writes, false);
    assert.ok(!JSON.stringify(report).includes('private-'));
    assert.equal(readFileSync(join(root, 'src/pages/item.tsx'), 'utf8'), 'throw new Error("private-source")');
    const practices = parse(await run(root, ['manifest', 'practices', '--request', '{"family":"webmcp","limit":2}', '--json']));
    assert.equal(practices.practices.length, 2); assert.equal(practices.nextCursor, 2);
    assert.equal(practices.source, 'brain_lex::codes::CODES');
    const connected = parse(await run(root, ['connect', '--json']));
    assert.equal(connected.handshake.serverInfo.name, 'brain');
    assert.equal(connected.handshake.protocolVersion, '2025-06-18');
    assert.equal(connected.clientConnected, false); assert.equal(connected.configurationWritten, false);
    assert.equal(connected.probeProcessReaped, true);
    assert.deepEqual(readdirSync(root).sort(), before, 'none of these commands writes client settings or cache');
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test('optional WebMCP levels apply to check and doctor without weakening tool security', { timeout: 120000 }, async () => {
  const root = fixture();
  try {
    mkdirSync(join(root, 'views'));
    writeFileSync(join(root, 'app.brain'), '<html><head><meta charset="utf-8"></head><body><slot /></body></html>');
    const form = '<meta seo title="Catalog" description="Search the public product catalog with a simple text query."><h1>Catalog</h1><form role="search"><input type="search" name="q" aria-label="Search"><button type="submit">Search</button></form>';
    writeFileSync(join(root, 'views/home.brain'), form);
    for (const level of ['off', 'info', 'warning', 'error']) {
      writeFileSync(join(root, 'brain.json'), JSON.stringify({ name: 'tooling-fixture', docs: false, lint: { B1706: level, B1707: level, B1708: level, B1705: 'off' } }));
      const checked = await run(root, ['check', '--json']);
      const check = JSON.parse(checked.stdout);
      const diagnostic = check.find(row => row.code === 'B1706');
      if (level === 'off') assert.equal(diagnostic, undefined); else assert.equal(diagnostic.level, level === 'error' ? 'warning' : level);
      const result = await run(root, ['doctor', '--json'], 25000);
      const advice = JSON.parse(result.stdout).advice.find(row => row.code === 'B1708');
      if (level === 'off') assert.equal(advice, undefined); else assert.equal(advice.level, level === 'error' ? 'info' : level);
    }
    writeFileSync(join(root, 'views/home.brain'), form.replace('<form role="search">', '<form role="search" tool="search_catalog" tool-description="Search">'));
    for (const level of ['off', 'info', 'warning', 'error']) {
      writeFileSync(join(root, 'brain.json'), JSON.stringify({ name: 'tooling-fixture', docs: false, lint: { B1707: level } }));
      const weak = JSON.parse((await run(root, ['check', '--json'])).stdout).find(row => row.code === 'B1707');
      if (level === 'off') assert.equal(weak, undefined); else assert.equal(weak.level, level === 'error' ? 'warning' : level);
    }
    writeFileSync(join(root, 'views/home.brain'), '<meta tools="delete_product: Delete a product from the catalog (id:str!)"><script>const delete_product = ({ id }) => id</script><p>Catalog</p>');
    writeFileSync(join(root, 'brain.json'), JSON.stringify({ name: 'tooling-fixture', docs: false }));
    const baseline = JSON.parse((await run(root, ['check', '--json'])).stdout).find(row => row.code === 'B1705');
    assert.ok(baseline, 'consequential tool produces the existing confirmation diagnostic');
    writeFileSync(join(root, 'brain.json'), JSON.stringify({ name: 'tooling-fixture', docs: false, lint: { B1705: 'off', B1701: 'off' } }));
    const secure = JSON.parse((await run(root, ['check', '--json'])).stdout);
    assert.equal(secure.find(row => row.code === 'B1705')?.level, baseline.level, 'existing confirmation diagnostic cannot be lowered by lint off');
    writeFileSync(join(root, 'views/home.brain'), '<meta tools="missing_task: Add a task to the public list (text:str!)"><p>Catalog</p>');
    const contract = JSON.parse((await run(root, ['check', '--json'])).stdout);
    assert.equal(contract.find(row => row.code === 'B1701')?.level, 'error', 'tool contract errors cannot be lowered by lint off');
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test('full MCP exposes real read-only tooling and practices without writing project settings', { timeout: 60000 }, async () => {
  const root = fixture();
  const server = createServer((_req, res) => res.end('mcp-observed'));
  await new Promise(accept => server.listen(0, '127.0.0.1', accept));
  assert.ok(binary, 'BRAIN_BIN must select the integrated binary');
  const child = spawn(binary, ['mcp'], { cwd: root, stdio: ['pipe', 'pipe', 'pipe'] });
  const pending = new Map();
  let id = 0, captured = 0;
  const reader = createInterface({ input: child.stdout });
  reader.on('line', line => {
    captured += line.length;
    if (captured > 2 * 1024 * 1024) { child.kill('SIGKILL'); return; }
    const value = JSON.parse(line);
    const waiter = pending.get(value.id);
    if (waiter) { pending.delete(value.id); clearTimeout(waiter.timer); waiter.accept(value); }
  });
  child.stderr.on('data', () => {});
  const closed = new Promise(accept => child.once('close', accept));
  const request = (method, params) => new Promise((accept, reject) => {
    const current = ++id;
    const timer = setTimeout(() => { pending.delete(current); reject(Error(`MCP fixture timed out for ${method}`)); }, 10000);
    pending.set(current, { timer, accept });
    child.stdin.write(`${JSON.stringify({ jsonrpc: '2.0', id: current, method, params })}\n`);
  });
  const call = async (name, args) => {
    const value = await request('tools/call', { name, arguments: args });
    assert.ok(!value.error, JSON.stringify(value));
    assert.equal(value.result.isError, false, JSON.stringify(value));
    return value.result.structuredContent ?? JSON.parse(value.result.content[0].text);
  };
  try {
    const before = readdirSync(root).sort();
    assert.equal((await request('initialize', { protocolVersion: '2025-06-18' })).result.serverInfo.name, 'brain');
    const tools = (await request('tools/list', {})).result.tools.map(tool => tool.name);
    for (const name of ['brain_manifest', 'brain_bench', 'brain_adopt', 'brain_connect']) assert.ok(tools.includes(name));
    assert.equal((await call('brain_manifest', { operation: 'practices', request: { family: 'webmcp', limit: 2 } })).practices.length, 2);
    assert.equal((await call('brain_adopt', { audit: true })).inspection.writes, false);
    assert.equal((await call('brain_connect', {})).clientConnected, false);
    const observed = await call('brain_bench', { url: `http://127.0.0.1:${server.address().port}/only`, requests: 1 });
    assert.equal(observed.observations[0].status, 200);
    assert.equal((await request('tools/call', { name: 'brain_adopt', arguments: { audit: false } })).result.isError, true);
    assert.deepEqual(readdirSync(root).sort(), before);
  } finally {
    child.stdin.end();
    const timer = setTimeout(() => child.kill('SIGKILL'), 1000);
    await closed; clearTimeout(timer); reader.close();
    for (const waiter of pending.values()) clearTimeout(waiter.timer);
    await new Promise(accept => server.close(accept)); rmSync(root, { recursive: true, force: true });
  }
});
