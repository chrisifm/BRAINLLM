import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { spawn } from 'node:child_process';
import { createInterface } from 'node:readline';
import { createServer } from 'node:net';
const binary = process.env.BRAIN_BIN && resolve(process.env.BRAIN_BIN);
const pause = milliseconds => new Promise(accept => setTimeout(accept, milliseconds));
async function until(read, milliseconds = 15000) {
  const deadline = Date.now() + milliseconds;
  while (Date.now() < deadline) { const value = read(); if (value) return value; await pause(25); }
  throw Error('bounded quality fixture deadline');
}
function app() {
  const root = mkdtempSync(join(tmpdir(), 'brain-quality-cli-'));
  mkdirSync(join(root, 'views')); mkdirSync(join(root, 'public'));
  writeFileSync(join(root, 'tokens.cssbrain'), '');
  writeFileSync(join(root, 'brain.json'), '{"name":"quality-fixture","docs":false}');
  writeFileSync(join(root, 'app.brain'), '<html lang="en"><head><meta charset="utf-8"></head><body><slot /></body></html>');
  writeFileSync(join(root, 'public/photo.svg'), '<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"></svg>');
  return root;
}
const head = '<meta seo title="Catalog" description="Browse a public catalog of useful example products."><h1>Catalog</h1>';
function run(root, args) {
  assert.ok(binary, 'BRAIN_BIN must select the exact integrated binary');
  return new Promise((accept, reject) => {
    const child = spawn(binary, args, { cwd: root, stdio: ['ignore', 'pipe', 'pipe'] });
    let stdout = '', stderr = '', exceeded = false;
    const timer = setTimeout(() => { exceeded = true; child.kill('SIGKILL'); }, 20000);
    child.stdout.on('data', data => { stdout += data; if (stdout.length > 2 * 1024 * 1024) child.kill('SIGKILL'); });
    child.stderr.on('data', data => { stderr += data; if (stderr.length > 65536) child.kill('SIGKILL'); });
    child.once('error', error => { clearTimeout(timer); reject(error); });
    child.once('close', status => { clearTimeout(timer); if (exceeded) reject(Error('owned CLI deadline')); else accept({ status, stdout, stderr }); });
  });
}
async function qualityMcp(root) {
  const child = spawn(binary, ['mcp'], { cwd: root, stdio: ['pipe', 'pipe', 'pipe'] });
  const lines = createInterface({ input: child.stdout });
  const responses = new Map(); let bytes = 0;
  lines.on('line', line => { bytes += line.length; if (bytes > 2 * 1024 * 1024) child.kill('SIGKILL'); else { const value = JSON.parse(line); responses.set(value.id, value); } });
  child.stderr.on('data', () => {});
  const closed = new Promise(accept => child.once('close', accept));
  try {
    child.stdin.write(`${JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'initialize', params: { protocolVersion: '2025-06-18' } })}\n`);
    await until(() => responses.get(1));
    child.stdin.write(`${JSON.stringify({ jsonrpc: '2.0', id: 2, method: 'tools/call', params: { name: 'brain_quality', arguments: { refresh: true } } })}\n`);
    const response = await until(() => responses.get(2));
    assert.equal(response.result.isError, false, JSON.stringify(response));
    return response.result.structuredContent ?? JSON.parse(response.result.content[0].text);
  } finally { child.stdin.end(); const timer = setTimeout(() => child.kill('SIGKILL'), 1000); await closed; clearTimeout(timer); lines.close(); }
}

test('native warnings, quality per-view recommendations, build facts and MCP use actual bounded evidence', { timeout: 60000 }, async () => {
  const root = app();
  try {
    writeFileSync(join(root, 'views/home.brain'), `${head}<img src="/photo.svg"><p>private-body-value</p>`);
    writeFileSync(join(root, 'views/other.brain'), '<h1>Other</h1>');
    const result = await run(root, ['quality', 'refresh', '--json']);
    assert.equal(result.status, 0, result.stderr || result.stdout);
    const report = JSON.parse(result.stdout);
    assert.equal(report.perViews.coverage.completeAudit, false);
    const home = report.perViews.views.find(row => row.viewFile === 'views/home.brain');
    assert.ok(home.recommendations.some(row => row.category === 'images'));
    const other = report.perViews.views.find(row => row.viewFile === 'views/other.brain');
    assert.ok(other.recommendations.some(row => row.category === 'page-content'));
    assert.equal(home.visualAssessment, 'not-run');
    assert.ok(!result.stdout.includes('private-body-value'));
    const viaMcp = await qualityMcp(root);
    assert.ok(viaMcp.perViews.views.find(row => row.viewFile === 'views/home.brain').recommendations.some(row => row.category === 'images'));
    writeFileSync(join(root, 'views/home.brain'), `${head}<img src="/photo.svg" alt="Example diagram" width="10" height="10">`);
    writeFileSync(join(root, 'views/other.brain'), '<meta seo title="Other" description="Read another public example page with useful information."><h1>Other</h1>');
    const built = await run(root, ['build', '--json', '--progress', 'off']);
    assert.equal(built.status, 0, built.stderr || built.stdout);
    const stored = JSON.parse(readFileSync(join(root, '.brain/quality/build.json'), 'utf8'));
    assert.equal(stored.routeFacts.source, 'verified-build-report');
    assert.ok(stored.routeFacts.routes.some(row => row.route === '/' && row.emittedBytes.htmlGz > 0));
    writeFileSync(join(root, 'views/home.brain'), `<script>const rows=[1,2]; for (const first of rows) { for (const second of rows) { log(first,second) } }</script>${head}`);
    const checked = await run(root, ['check', '--json']);
    const warning = JSON.parse(checked.stdout).find(row => row.code === 'B1303');
    assert.equal(warning?.level, 'warning'); assert.deepEqual(warning.edits ?? [], []);
    const catalog = await run(root, ['manifest', 'practices', '--request', '{"codePrefix":"B13","limit":100}', '--json']);
    for (const code of ['B1301', 'B1303', 'B1305', 'B1312', 'B1313']) assert.ok(JSON.parse(catalog.stdout).practices.some(row => row.code === code));
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test('the existing development save feed emits per-view image review advice without another watcher or browser', { timeout: 60000 }, async () => {
  const root = app(); writeFileSync(join(root, 'views/home.brain'), `${head}<p>Initial content.</p>`);
  const reservation = createServer(); await new Promise(accept => reservation.listen(0, '127.0.0.1', accept));
  const port = reservation.address().port; await new Promise(accept => reservation.close(accept));
  const child = spawn(binary, ['run', '--port', String(port), '--no-inspect', '--progress', 'off'], { cwd: root, detached: true, stdio: ['ignore', 'pipe', 'pipe'] });
  let stdout = '', stderr = ''; const records = [];
  child.stdout.on('data', data => { stdout += data; if (stdout.length > 1024 * 1024) child.kill('SIGKILL'); });
  child.stderr.on('data', data => { stderr += data; if (stderr.length > 65536) child.kill('SIGKILL'); });
  const lines = createInterface({ input: child.stdout });
  lines.on('line', line => { if (line.startsWith('impact ')) { try { records.push(JSON.parse(line.slice(7))); } catch {} } });
  const closed = new Promise(accept => child.once('close', accept));
  try {
    await until(() => records.some(row => row.baseline), 20000);
    writeFileSync(join(root, 'views/home.brain'), `${head}<img src="/photo.svg" width="10" height="10">`);
    const record = await until(() => records.find(row => row.viewAdvice?.newFindingCodes?.includes('B0401')), 20000);
    assert.ok(record.viewAdvice.affectedViews.includes('views/home.brain'), JSON.stringify(record));
    assert.ok(record.viewAdvice.recommendations.some(row => row.category === 'images'));
    assert.equal(record.viewAdvice.visualAssessment, 'not-run');
  } finally {
    try { process.kill(-child.pid, 'SIGTERM'); } catch {}
    const timer = setTimeout(() => { try { process.kill(-child.pid, 'SIGKILL'); } catch {} }, 1000);
    await closed; clearTimeout(timer); lines.close(); rmSync(root, { recursive: true, force: true });
  }
});

test('actual native checking skips wrapped escapes, method shadows, complete identifier uses and ordering barriers', { timeout: 60000 }, async () => {
  const root = app(); mkdirSync(join(root, 'api'));
  writeFileSync(join(root, 'api/items.jsbrain'), "export default api('/records/:id', { get: { out: 'str' } })");
  const cases = [
    ['B1312', "import items from '../api/items.jsbrain'\nconst configure=box=>{box.api.get=id=>'local'};configure({api:items});const collect=async(ids)=>{for(const id of ids){await items.get(id)}}"],
    ['B1303', 'const rows=[1,2];const box={rows};box.rows.pop();for(const first of rows){for(const second of rows){log(first,second)}}'],
    ['B1313', "const custom=selector=>({style:{},offsetWidth:1});const box={document};box.document.querySelector=custom;const panel=document.querySelector('#panel');const rows=[1,2];for(const row of rows){panel.style.width=row+'px';const width=panel.offsetWidth}"],
    ['B1313', "const panel=document.querySelector('#panel');const rows=[1,2];const consume=value=>panel.offsetWidth;for(const row of rows){consume(panel.style.width=row+'px');const width=panel.offsetWidth}"],
    ['B1313', "const panel=document.querySelector('#panel');const rows=[1,2];const consume=()=>panel.offsetWidth;for(const row of rows){const value=(panel.style.width=row+'px',consume());const width=panel.offsetWidth}"],
    ['B1312', "import items from '../api/items.jsbrain'\nconst loader={async collect(items){for(const id of ['a']){await items.get(id)}}}"],
    ['B1313', "const loader={collect(document){const panel=document.querySelector('#panel');const rows=[1,2];for(const row of rows){panel.style.width=row+'px';const width=panel.offsetWidth}}}"],
    ['B1301', 'const collect=async(items)=>{let cursor=0;for(const item of items){cursor=await read({cursor})}}'],
    ['B1303', 'const rows=[1,2];const take=(value=rows.pop())=>value;take();for(const first of rows){for(const second of rows){log(first,second)}}'],
    ['B1303', 'const rows=[1,2];const {value=rows.pop()}={};for(const first of rows){for(const second of rows){log(first,second)}}'],
  ];
  try {
    for (const [code, script] of cases) {
      writeFileSync(join(root, 'views/home.brain'), `<script>${script}</script>${head}`);
      const result = await run(root, ['check', '--json']);
      assert.ok(!JSON.parse(result.stdout).some(row => row.code === code), `${code}: ${result.stdout}`);
    }
    for (const expression of ['shared.map(value=>({row,value}))[0].row', 'shared.map(value=>({[row]:value}))[0]']) {
      writeFileSync(join(root, 'views/home.brain'), `<script>const shared=[1,2];const rows=[1,2]</script>${head}{#each rows as row (row)}<p>{${expression}}</p>{/each}`);
      const result = await run(root, ['check', '--json']); assert.ok(!JSON.parse(result.stdout).some(row => row.code === 'B1305'), result.stdout);
    }
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test('real MCP quality reports an incomplete view closure when the shared dependency edge budget clips a chain', { timeout: 60000 }, async () => {
  const root = app(); mkdirSync(join(root, 'comp'));
  try {
    for (let index = 0; index < 150; index++) {
      const next = String(index + 1).padStart(3, '0');
      writeFileSync(join(root, `comp/c${String(index).padStart(3, '0')}.brain`), (index < 149 ? `<script>import Child from './c${next}.brain'</script>` : '') + '<p>Fixture</p>');
    }
    writeFileSync(join(root, 'views/home.brain'), `<script>import Child from '../comp/c000.brain'</script>${head}<Child />`);
    const report = await qualityMcp(root);
    assert.equal(report.perViews.coverage.graphEdges, 4096);
    assert.ok(report.perViews.coverage.skippedEdgesOrDocuments > 0);
    assert.equal(report.perViews.views.find(row => row.viewFile === 'views/home.brain').closureComplete, false);
  } finally { rmSync(root, { recursive: true, force: true }); }
});
