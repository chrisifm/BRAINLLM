import { test } from 'node:test'
import assert from 'node:assert/strict'
import { viewerClient } from '../../crates/brain-cli/src/templates/viewer-client.mjs'
const json = value => new Response(JSON.stringify(value), { headers: { 'content-type': 'application/json' } })
const health = { service:'claude-viewer', version: '0.20.9', bridge: { connected: 1, userScripts: true }, profile: 'must-never-return', email: 'must-never-return@example.test' }
function bridge({ failScenario = false, failClose = false, failAPI = false, loadFailure = false } = {}) {
  const calls = []
  const fetch = async (url, options) => {
    if(url.startsWith('http://127.0.0.1:4315/')) throw Error('Codex service unavailable in this fixture')
    assert.ok(url.startsWith('http://127.0.0.1:4313/'))
    if (url.endsWith('/health')) return json(health)
    assert.ok(url.endsWith('/api/bridge/call'))
    const command = JSON.parse(options.body)
    assert.ok(options.headers['X-CV-Session'].startsWith('brain-e2e-'))
    assert.equal(options.headers['X-CV-Label'], 'BRAINLLM E2E')
    assert.match(command.callId, /^[a-f0-9-]{36}$/)
    assert.ok(['open', 'run', 'close'].includes(command.action))
    calls.push(command)
    if (command.action === 'open') return json({ ok: true, result: { id: 312, title: 'must-never-return', windowId: 888 } })
    assert.equal(command.target, '312')
    if (command.action === 'close') return json({ ok: !failClose, result: { closed: 312 } })
    assert.equal(command.world, 'MAIN')
    assert.match(command.code, /location\.origin/)
    assert.match(command.code, /location\.pathname/)
    if (failAPI) return json({ ok: false, error: 'private-browser-message' })
    if (command.code.includes('const result=await window.__brainLoad')) return json({ ok: true, result: { failed: loadFailure, names: ['Declared action'] } })
    if (command.code.includes('const result=await window.__brainStep')) return json({ ok: true, result: { name: 'Declared action', ok: !failScenario, ms: 2 } })
    if (command.code.includes('const rules=')) return json({ ok: true, result: { items: [], truncated: false } })
    if (command.code.includes('m.errors.slice')) return json({ ok: true, result: [] })
    return json({ ok: true, result: true })
  }
  return { fetch, calls }
}
const suite = { op: 'run', base: 'http://127.0.0.1:5182', files: ['comp/counter.test.brain'], timeoutMs: 2000 }

test('health projects capability without profile/account/title inventory', async () => {
  const report = await viewerClient({ op: 'probe' }, async () => json(health))
  assert.equal(report.available, true)
  assert.equal(report.serviceVersion, '0.20.9')
  assert.equal(report.connected, true)
  assert.equal(report.capabilities.forcedGC, false)
  assert.ok(!JSON.stringify(report).includes('must-never-return'))
})
test('same declared scenario API runs only in its newly owned numeric tab then closes it', async () => {
  const mock = bridge(), result = await viewerClient(suite, mock.fetch)
  assert.equal(result.ok, true)
  assert.equal(result.reports[0].tests[0].ok, true)
  assert.equal(result.cleanup, 'owned-tabs-closed')
  assert.equal(mock.calls.filter(c => c.action === 'open').length, 1)
  assert.equal(mock.calls.filter(c => c.action === 'close').length, 1)
  assert.equal(mock.calls[0].url, 'http://127.0.0.1:5182/@brain/test.html')
  assert.equal(mock.calls[0].active, false)
})
test('failed scenario is a failed result and still closes the owned tab', async () => {
  const mock = bridge({ failScenario: true }), result = await viewerClient(suite, mock.fetch)
  assert.equal(result.ok, true)
  assert.equal(result.reports[0].tests[0].ok, false)
  assert.equal(mock.calls.at(-1).action, 'close')
})
test('connected but broken bridge never becomes a successful headless fallback', async () => {
  const mock = bridge({ failAPI: true }), result = await viewerClient(suite, mock.fetch)
  assert.equal(result.ok, false)
  assert.equal(result.fallbackEligible, false)
  assert.equal(result.stage, 'ready')
  assert.equal(mock.calls.at(-1).action, 'close')
  assert.ok(!JSON.stringify(result).includes('private-browser-message'))
})
test('a load failure and close failure remain explicit instead of leaking into another file', async () => {
  const mock = bridge({ loadFailure: true, failClose: true }), result = await viewerClient({ ...suite, files: [...suite.files, 'comp/other.test.brain'] }, mock.fetch)
  assert.equal(result.ok, false)
  assert.equal(result.code, 'VIEWER_OWNED_TAB_CLEANUP_FAILED')
  assert.equal(mock.calls.filter(c => c.action === 'open').length, 1)
})
test('no connection permits explicit fallback; disabled user scripts is an actionable failure', async () => {
  const absent = await viewerClient({ op: 'probe' }, async () => { throw Error('offline') })
  assert.equal(absent.fallbackEligible, true)
  const unsupported = await viewerClient({ op: 'probe' }, async () => json({ service:'claude-viewer', bridge: { connected: 1, userScripts: false } }))
  assert.equal(unsupported.ok, false)
  assert.equal(unsupported.fallbackEligible, false)
})
test('remote origins, traversal, hidden files, oversized suites cannot open tabs', async () => {
  for (const input of [{ ...suite, base: 'https://external.example' }, { ...suite, files: ['../private.test.brain'] }, { ...suite, files: ['.brain/private.test.brain'] }, { ...suite, files: Array(33).fill('a.test.brain') }]) {
    const mock = bridge(), result = await viewerClient(input, mock.fetch)
    assert.equal(result.ok, false)
    assert.equal(mock.calls.length, 0)
  }
})

test('Codex Viewer uses its own loopback endpoint and session headers',async()=>{
  const base=bridge();let calls=0;
  const fetcher=async(url,options)=>{
    assert.ok(url.startsWith('http://127.0.0.1:4315/'));
    if(url.endsWith('/health'))return json({...health,service:'codex-viewer'});
    assert.ok(options.headers['X-XV-Session'].startsWith('brain-e2e-'));
    assert.equal(options.headers['X-XV-Label'],'BRAINLLM E2E');
    assert.equal(options.headers['X-CV-Session'],undefined);calls++;
    return base.fetch(url.replace(':4315',':4313'),{...options,headers:{...options.headers,'X-CV-Session':options.headers['X-XV-Session'],'X-CV-Label':options.headers['X-XV-Label']}});
  };
  const result=await viewerClient(suite,fetcher);assert.equal(result.ok,true);assert.equal(result.adapter,'codex-viewer');assert.ok(calls>0);
});

test('multiple connected profiles require private explicit selection before opening a tab',async()=>{
  let mutations=0;
  const fetcher=async(url)=>{if(url.endsWith('/health'))return json({...health,bridge:{connected:3,userScripts:true}});mutations++;throw Error('No mutation expected')};
  const result=await viewerClient(suite,fetcher);assert.equal(result.code,'VIEWER_PROFILE_REQUIRED');assert.equal(mutations,0);assert.equal(result.fallbackEligible,false);
});
