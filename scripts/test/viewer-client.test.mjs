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
    if (command.code.includes('const result=await window.__brainLoad')) return json({ ok: true, result: { failed: loadFailure, id: 'fixture-document.1', names: ['Declared action'] } })
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

// Both Claude Viewer and Codex Viewer may be connected at once, each with its own browser profiles. A profile named in
// BRAIN_VIEWER_PROFILE belongs to one of them; the other refuses the open explicitly (no tab is created), so the
// adapter moves to the next connected service instead of failing at stage open.
function twoServices({ codexRefuses = true, claudeRefuses = false, loseCodexOpen = false } = {}) {
  const claude = bridge(), seen = []
  const fetcher = async (url, options) => {
    const codex = url.startsWith('http://127.0.0.1:4315/')
    if (url.endsWith('/health')) return json({ ...health, service: codex ? 'codex-viewer' : 'claude-viewer' })
    const command = JSON.parse(options.body); seen.push([codex ? 'codex' : 'claude', command.action, command.profile])
    if (codex) {
      if (loseCodexOpen) throw Error('connection reset after the request was sent')
      if (codexRefuses) return new Response(JSON.stringify({ ok: false, error: 'private-profile-detail' }), { status: 422 })
      return claude.fetch(url.replace(':4315', ':4313'), { ...options, headers: { ...options.headers, 'X-CV-Session': options.headers['X-XV-Session'], 'X-CV-Label': options.headers['X-XV-Label'] } })
    }
    if (claudeRefuses && command.action === 'open') return new Response(JSON.stringify({ ok: false, error: 'private-limit-detail' }), { status: 422 })
    return claude.fetch(url, options)
  }
  return { fetcher, seen }
}
test('an explicit open refusal from one connected Viewer moves to the next service; no tab was opened there', async () => {
  const mock = twoServices(), result = await viewerClient({ ...suite, profile: 'WORK' }, mock.fetcher)
  assert.equal(result.ok, true)
  assert.equal(result.adapter, 'claude-viewer')
  assert.deepEqual(mock.seen.filter(s => s[1] === 'open'), [['codex', 'open', 'WORK'], ['claude', 'open', 'WORK']])
  assert.ok(!JSON.stringify(result).includes('private-profile-detail'))
})
test('BRAIN_VIEWER_SERVICE pins the service; an unknown service name is rejected before any browser call', async () => {
  const mock = twoServices(), pinned = await viewerClient({ ...suite, profile: 'WORK', service: 'claude-viewer' }, mock.fetcher)
  assert.equal(pinned.ok, true)
  assert.ok(mock.seen.every(s => s[0] === 'claude'))
  const bad = await viewerClient({ ...suite, service: 'other-viewer' }, mock.fetcher)
  assert.equal(bad.code, 'VIEWER_SERVICE_INVALID')
  assert.equal(bad.fallbackEligible, false)
})
test('every connected Viewer refusing the open is an actionable failure with nothing to clean up', async () => {
  const mock = twoServices({ claudeRefuses: true }), result = await viewerClient({ ...suite, profile: 'WORK' }, mock.fetcher)
  assert.equal(result.ok, false)
  assert.equal(result.code, 'VIEWER_OPEN_REFUSED')
  assert.equal(result.stage, 'open')
  assert.equal(result.cleanup, 'none; no tab was opened')
  assert.match(result.fix, /BRAIN_VIEWER_PROFILE/)
  assert.ok(!/private-(profile|limit)-detail/.test(JSON.stringify(result)))
})
test('a lost open response is never retried on another service', async () => {
  const mock = twoServices({ loseCodexOpen: true }), result = await viewerClient({ ...suite, profile: 'WORK' }, mock.fetcher)
  assert.equal(result.ok, false)
  assert.equal(result.code, 'VIEWER_EXECUTION_FAILED')
  assert.ok(mock.seen.every(s => s[0] === 'codex'))
})

// Drive each boundary deterministically; the bridge never exposes unrelated tabs or profile details.
function navigationBridge(phase, { mismatch = false, loseReopen = false, repeat = false } = {}) {
  const calls=[], executions=[], closed=[], documents=new Map()
  let opened=0
  const fetch=async(url,options)=>{
    if(url.startsWith('http://127.0.0.1:4315/'))throw Error('unavailable fixture service')
    if(url.endsWith('/health'))return json(health)
    if(url.includes('/api/bridge/result/'))throw Error('lost exact recovery result')
    const command=JSON.parse(options.body);calls.push(command)
    const reply=value=>json({ok:true,result:value})
    if(command.action==='open'){
      opened++
      if(loseReopen&&opened===2)throw Error('lost recovery open response')
      const id=400+opened;documents.set(id,{id:`document-${opened}.1`,valid:true,first:opened===1})
      return reply({id})
    }
    const id=Number(command.target),doc=documents.get(id)
    assert.ok(doc,'only a known owned tab may be targeted')
    if(command.action==='close'){closed.push(id);return reply({closed:id})}
    assert.equal(command.action,'run')
    const code=command.code
    if(code.includes('const result=await window.__brainLoad'))return reply({id:doc.id,names:mismatch&&!doc.first?['changed']:['first','second']})
    if(code.includes('const result=await window.__brainStep')){
      const index=Number(/__brainStep\((\d+),/.exec(code)?.[1]);assert.ok(Number.isInteger(index))
      assert.ok(code.includes(JSON.stringify(doc.id)))
      if((doc.first||repeat)&&phase==='before'&&index===1){doc.valid=false;return reply({documentChanged:true,started:false})}
      executions.push(index)
      if(doc.first&&phase==='during'&&index===0){doc.valid=false;return reply({documentChanged:true,started:true})}
      return reply({ok:true,ms:1})
    }
    if(code.includes('__brainMatches'))return reply(doc.valid)
    if(code.includes('const rules=')){
      if(doc.first&&phase==='audit'){doc.valid=false;return reply({items:[{code:'B0406',role:'button',node:'button',msg:'interrupted observation'}],truncated:true})}
      return reply({items:[],truncated:false})
    }
    if(code.includes('m.errors.slice')){if(phase==='feedback')doc.valid=false;return reply([])}
    return reply(true)
  }
  return {fetch,calls,executions,closed,get opened(){return opened}}
}
for(const phase of ['before','during','audit','feedback'])test(`Viewer retains original test identity when navigation occurs at ${phase}`,async()=>{
  const mock=navigationBridge(phase),result=await viewerClient(suite,mock.fetch)
  assert.equal(result.ok,true,JSON.stringify(result))
  const report=result.reports[0]
  assert.deepEqual(report.tests.map(t=>t.name),['first','second'])
  const failed=phase==='feedback'?1:0
  assert.equal(report.tests[failed].ok,false)
  assert.match(report.tests[failed].error,/registered document.*navigation/)
  assert.equal(report.tests[1-failed].ok,true)
  assert.deepEqual(mock.executions,[0,1],'a failed or completed assertion is never replayed')
  assert.deepEqual(report.a11y,[],'interrupted observations are discarded')
  assert.equal(report.auditTruncated,undefined)
  assert.equal(mock.opened,phase==='feedback'?1:2)
  assert.deepEqual(mock.closed,phase==='feedback'?[401]:[401,402])
  if(phase==='feedback')assert.match(report.error,/not measured/)
})
test('Viewer refuses a changed registry after recovery without executing another assertion',async()=>{
  const mock=navigationBridge('during',{mismatch:true}),result=await viewerClient(suite,mock.fetch)
  assert.equal(result.ok,true)
  assert.match(result.reports[0].error,/same registered scenarios/)
  assert.deepEqual(mock.executions,[0])
  assert.deepEqual(mock.closed,[401,402])
})
test('a lost recovery open is never retried and retains the original failed scenario',async()=>{
  const mock=navigationBridge('during',{loseReopen:true}),result=await viewerClient(suite,mock.fetch)
  assert.equal(result.ok,false)
  assert.equal(result.stage,'reopen')
  assert.match(result.cleanup,/unknown.*open response/)
  assert.equal(mock.opened,2)
  assert.deepEqual(mock.closed,[401],'the previously closed tab is never closed again')
  assert.deepEqual(mock.executions,[0])
  assert.equal(result.completedReports[0].tests[0].name,'first')
  assert.equal(result.completedReports[0].tests[0].ok,false)
})

test('a second pre-step navigation makes remaining unexecuted scenarios explicit',async()=>{
  const mock=navigationBridge('before',{repeat:true}),result=await viewerClient(suite,mock.fetch)
  assert.equal(result.ok,true)
  assert.match(result.reports[0].error,/remaining scenarios and feedback were not measured/)
  assert.deepEqual(mock.executions,[0])
  assert.equal(mock.opened,2,'one recovery only for a not-yet-started assertion')
  assert.deepEqual(mock.closed,[401,402])
})
