// Embedded loopback-only Viewer transport. No profiles/tabs inventory, remote URLs or arbitrary script input.
import { randomUUID } from 'node:crypto'
const SERVICES = [{ url:'http://127.0.0.1:4315',name:'codex-viewer',header:'X-XV' },{ url:'http://127.0.0.1:4313',name:'claude-viewer',header:'X-CV' }]
const MAX_RESPONSE = 2 * 1024 * 1024
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms))
const safeName = value => typeof value === 'string' && value.length <= 160 && !/[\x00-\x1f]/.test(value) && !/(?:bearer\s|password\s*[=:]|token\s*[=:]|sk-[A-Za-z0-9]{16})/i.test(value) ? value : 'Declared scenario'
function baseURL(value) {
  const url = new URL(value)
  if (url.protocol !== 'http:' || !['127.0.0.1', 'localhost', '[::1]'].includes(url.hostname) || !url.port || url.username || url.password || url.search || url.hash || url.pathname !== '/') throw Error('VIEWER_LOCAL_BASE_REQUIRED')
  return url.origin
}
function sourceFile(value) {
  if (typeof value !== 'string' || value.length > 240 || !value.endsWith('.test.brain') || value.startsWith('/') || value.includes('\\') || value.split('/').some(p => !p || p === '.' || p === '..' || p.startsWith('.')) || /[\x00-\x1f?#]/.test(value)) throw Error('VIEWER_TEST_PATH_INVALID')
  return value
}
async function jsonRequest(fetcher, service, path, options, timeout) {
  const controller = new AbortController(), timer = setTimeout(() => controller.abort(), timeout)
  try {
    const response = await fetcher(service + path, { ...options, redirect: 'error', signal: controller.signal })
    const reader = response.body.getReader(); let length = 0; const chunks = []
    try {
      for (;;) { const { done, value } = await reader.read(); if (done) break; length += value.byteLength; if (length > MAX_RESPONSE) throw Error('VIEWER_RESPONSE_LIMIT'); chunks.push(value) }
    } finally { await reader.cancel().catch(() => {}) }
    const value=JSON.parse(Buffer.concat(chunks).toString('utf8'))
    if (!response.ok && !(value && value.ok===false)) throw Error('VIEWER_HTTP_FAILURE')
    return value
  } finally { clearTimeout(timer) }
}
export async function viewerClient(input, fetcher = globalThis.fetch) {
  if (!input || !['probe', 'run'].includes(input.op)) return { ok: false, code: 'VIEWER_INVALID_OPERATION' }
  let health, service, observed
  for (const candidate of SERVICES) {
    let value
    try { value=await jsonRequest(fetcher,candidate.url,'/health',{},1500) } catch { continue }
    if (value?.service!==candidate.name) continue
    const connected=value?.bridge?.connected===true||Number.isInteger(value?.bridge?.connected)&&value.bridge.connected>0
    if (!observed || connected) observed={health:value,service:candidate}
    if (connected && value?.bridge?.userScripts===true) {health=value;service=candidate;break}
  }
  if (!service && observed) ({health,service}=observed)
  if (!service) return {ok:true,available:false,engine:'viewer',reason:'service-unavailable',fallbackEligible:true}
  const connected = health?.bridge?.connected === true || Number.isInteger(health?.bridge?.connected) && health.bridge.connected > 0
  const scripts = health?.bridge?.userScripts === true
  const version = typeof health.version === 'string' && /^\d+\.\d+\.\d+(?:[-.][\w]+)*$/.test(health.version) ? health.version : null
  const projection = { engine: 'viewer', adapter:service.name, serviceVersion: version, connected, userScripts: scripts }
  if (!connected) return { ok: true, available: false, ...projection, reason: 'extension-not-connected', fallbackEligible: true }
  if (!scripts) return { ok: false, available: false, ...projection, code: 'VIEWER_USER_SCRIPTS_REQUIRED', fallbackEligible: false }
  const profile=input.profile==null?null:typeof input.profile==='string'&&input.profile.length<=128&&!/[\x00-\x1f\x7f]/.test(input.profile)?input.profile:null
  if(input.profile!=null&&!profile)return {ok:false,...projection,code:'VIEWER_PROFILE_INVALID',fallbackEligible:false}
  const requiresProfile=Number.isInteger(health?.bridge?.connected)&&health.bridge.connected>1&&!profile
  if (input.op === 'probe') return { ok: true, available: true, ...projection, requiresProfile, capabilities: { pageWorld: 'MAIN', scenarioAPI: '__brainLoad/__brainStep', accessibility: 'DOM-label-heuristic; not Chrome AX tree', forcedGC: false } }
  if(requiresProfile)return {ok:false,...projection,code:'VIEWER_PROFILE_REQUIRED',fix:'Set private BRAIN_VIEWER_PROFILE to the intended local extension profile; tests open a new isolated window and never claim another session window',fallbackEligible:false}
  let base, files
  try {
    base = baseURL(input.base)
    if (!Array.isArray(input.files) || !input.files.length || input.files.length > 32) throw Error('VIEWER_FILE_LIMIT')
    files = input.files.map(sourceFile)
  } catch { return { ok: false, ...projection, code: 'VIEWER_INVALID_LOCAL_SUITE' } }
  const duration = Number.isFinite(input.timeoutMs) ? Math.max(2000, Math.min(180000, input.timeoutMs)) : 60000
  const deadline = Date.now() + duration
  const session = 'brain-e2e-' + randomUUID(), headers = { 'content-type': 'application/json', [service.header+'-Session']: session, [service.header+'-Label']: 'BRAINLLM E2E' }
  const reports = [], owned = new Set()
  async function call(command, cleanup = false) {
    const end = cleanup ? Date.now() + 5000 : Math.min(deadline, Date.now() + 30000)
    const callId = randomUUID(), timeout = Math.max(2, Math.ceil((end - Date.now()) / 1000))
    if (!cleanup && end <= Date.now()) throw Error('VIEWER_SUITE_DEADLINE')
    let out
    try { out = await jsonRequest(fetcher, service.url, '/api/bridge/call', { method: 'POST', headers, body: JSON.stringify({ ...command, ...(profile?{profile}:{}), callId, timeout }) }, Math.max(1, end - Date.now())) }
    catch { out = { pending: true } }
    // Recover this exact id; never retry an open action or enumerate tabs after a lost response.
    while (out?.pending && Date.now() < end) {
      try { out = await jsonRequest(fetcher, service.url, '/api/bridge/result/' + callId + '?wait=1', { headers }, Math.min(1500, Math.max(1, end - Date.now()))) }
      catch { if (Date.now() < end) await sleep(Math.min(100, end - Date.now())) }
    }
    if (!out?.ok || out.pending) throw Error('VIEWER_BRIDGE_CALL_FAILED')
    return out.result
  }
  const guard = `if(location.origin!==${JSON.stringify(base)}||location.pathname!=='/@brain/test.html')throw Error('VIEWER_OWNED_PAGE_MOVED');`
  const run = (id, body) => { if (!owned.has(id)) throw Error('VIEWER_TAB_NOT_OWNED'); return call({ action: 'run', target: String(id), world: 'MAIN', code: guard + body }) }
  let stage = 'open', cleanupFailed = false
  try {
    for (const file of files) {
      stage = 'open'
      const opened = await call({ action: 'open', url: base + '/@brain/test.html', active: false })
      if (!Number.isSafeInteger(opened?.id) || opened.id < 1) throw Error('VIEWER_NO_OWNED_TAB')
      const id = opened.id; owned.add(id)
      try {
        stage = 'ready'
        await run(id, "const end=Date.now()+10000;while(typeof window.__brainLoad!=='function'){if(Date.now()>end)throw Error('VIEWER_TEST_API_NOT_READY');await new Promise(r=>setTimeout(r,20))}return true;")
        stage = 'load'
        const loaded = await run(id, `const result=await window.__brainLoad(${JSON.stringify('/' + file)});return {failed:typeof result?.error==='string',names:Array.isArray(result?.names)?result.names.slice(0,257):[]};`)
        if (loaded?.failed) { reports.push({ file, error: 'Declared test module failed to load; inspect compiler feedback.', tests: [], a11y: [], errors: [], leaks: [], engine: 'viewer', a11yMode: 'DOM-label-heuristic', heapMode: 'not-measured' }) } else {
        if (!Array.isArray(loaded?.names) || loaded.names.length > 256) throw Error('VIEWER_SCENARIO_LIMIT')
        const report = { file, tests: [], a11y: [], errors: [], leaks: [], engine: 'viewer', a11yMode: 'DOM-label-heuristic', heapMode: 'not-measured' }
        await run(id, "await import('/@brain/brain-err.js').then(m=>m.capture({log:false}));return true;")
        for (let i = 0; i < loaded.names.length; i++) {
          stage = 'scenario'
          const result = await run(id, `const result=await window.__brainStep(${i});return {name:result?.name,ok:result?.ok===true,ms:result?.ms};`)
          report.tests.push({ name: safeName(result?.name), ok: result?.ok === true, ms: Number.isFinite(result?.ms) ? Math.min(180000, Math.max(0, result.ms)) : 0, ...(result?.ok === true ? {} : { error: 'Declared scenario assertion failed; inspect its source and compiler feedback.' }) })
          // DOM labels only: never claims Accessibility.getFullAXTree or computed accessibility parity.
          const issues = await run(id, `const rules={button:['B0406','control without accessible text'],link:['B0406','control without accessible text'],textbox:['B0403','form control without an accessible name'],checkbox:['B0403','form control without an accessible name'],radio:['B0403','form control without an accessible name'],combobox:['B0403','form control without an accessible name'],img:['B0401','image without alternative text']};const out=[];const elements=document.querySelectorAll('#root button,#root a[href],#root input,#root textarea,#root select,#root img,#root [role]');let visited=0;for(const el of elements){if(++visited>4096)break;if(el.hidden||el.getAttribute('aria-hidden')==='true')continue;const tag=el.localName;const role=el.getAttribute('role')||(tag==='a'?'link':tag==='input'?(['checkbox','radio'].includes(el.type)?el.type:el.type==='hidden'?'':el.type==='button'||el.type==='submit'?'button':'textbox'):tag==='textarea'?'textbox':tag==='select'?'combobox':tag);const rule=rules[role];if(!rule)continue;if(tag==='img'&&el.hasAttribute('alt')&&el.alt==='')continue;const labelled=(el.getAttribute('aria-labelledby')||'').split(/\\s+/).filter(Boolean).map(id=>document.getElementById(id)?.textContent||'').join(' ');const label=el.getAttribute('aria-label')||labelled||el.alt||Array.from(el.labels||[]).map(l=>l.textContent).join(' ')||(['button','link'].includes(role)?el.textContent:'');if(!label.trim())out.push({code:rule[0],role,node:tag,msg:rule[1]});if(out.length>=128)break}return {items:out,truncated:visited>4096||out.length>=128};`)
          for (const issue of issues?.items || []) if (!report.a11y.some(old => old.code === issue.code && old.role === issue.role && old.node === issue.node)) report.a11y.push({ code: issue.code, role: issue.role, node: issue.node, test: safeName(result?.name), msg: issue.msg, fix: issue.code === 'B0401' ? 'add alt (or alt="" for a decorative image)' : 'add a visible or associated accessible label' })
          if (issues?.truncated) report.auditTruncated = true
        }
        stage = 'runtime-feedback'
        const errors = await run(id, "const m=await import('/@brain/brain-err.js');return m.errors.slice(0,128).map(e=>({type:typeof e.type==='string'?e.type:'runtime',line:Number.isSafeInteger(e.line)?e.line:null,n:Number.isSafeInteger(e.n)?e.n:1}));")
        report.errors = Array.isArray(errors) ? errors.map(e => ({ type: safeName(e.type), line: e.line, n: e.n, msg: 'Runtime failure captured in the owned test page; inspect compiler feedback.' })) : []
        await run(id, "globalThis.__brainDispose?.();return true;")
        reports.push(report)
        }
      } finally {
        try { await call({ action: 'close', target: String(id) }, true); owned.delete(id) } catch { cleanupFailed = true; stage = 'close' }
      }
      if (cleanupFailed) throw Error('VIEWER_OWNED_TAB_CLEANUP_FAILED')
    }
    return { ok: true, ...projection, reports, capabilities: { scenarios: 'same declared native test API', accessibility: 'DOM-label-heuristic; not Chrome AX tree', forcedGC: false }, cleanup: 'owned-tabs-closed' }
  } catch {
    return { ok: false, ...projection, code: cleanupFailed ? 'VIEWER_OWNED_TAB_CLEANUP_FAILED' : 'VIEWER_EXECUTION_FAILED', stage, completedReports: reports, cleanup: owned.size ? 'unknown; owned tab close failed or open result lost' : stage === 'open' ? 'unknown if open response was lost' : 'owned-tabs-closed', fallbackEligible: false }
  }
}
export async function viewerMain() {
  let size = 0, chunks = []
  for await (const chunk of process.stdin) { size += chunk.length; if (size > 65536) { console.log(JSON.stringify({ ok: false, code: 'VIEWER_INPUT_LIMIT' })); return } chunks.push(chunk) }
  let output
  try { output = await viewerClient(JSON.parse(Buffer.concat(chunks).toString('utf8'))) }
  catch { output = { ok: false, code: 'VIEWER_CLIENT_FAILED' } }
  const serialized = JSON.stringify(output)
  console.log(serialized.length <= MAX_RESPONSE ? serialized : JSON.stringify({ ok: false, code: 'VIEWER_OUTPUT_LIMIT' }))
}
