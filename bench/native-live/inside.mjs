// Owned-container acceptance only. Inputs contain synthetic disposable database credentials.
import fs from 'node:fs'
import path from 'node:path'
import os from 'node:os'
import crypto from 'node:crypto'
import { spawn, spawnSync } from 'node:child_process'
import { frameMetadata } from './frame-metadata.mjs'
const ensure = (value, code) => { if (!value) throw Error(code) }
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms))
const root = fs.mkdtempSync(path.join(os.tmpdir(), 'brain-live-app-'))
const profileRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'brain-live-browser-'))
const children = [], contexts = [], pages = [], results = []
let cdp, chrome, done = false, diagnosticCodes = new Set(), setupDiagnostics = {}
const origin = 'http://127.0.0.1:4180', base = origin + '/app', binary = '/tmp/brain-live-binary'
function child(command, args, options = {}) {
  const p = spawn(command, args, { cwd: root, env: process.env, detached: true, stdio: ['ignore', 'pipe', 'pipe'], ...options })
  p.kept = ''; p.bytes = 0
  for (const stream of [p.stdout, p.stderr]) stream.on('data', chunk => {
    p.bytes += chunk.length
    if (p.bytes > 262144) { p.outputBoundHit=true; try { process.kill(-p.pid, 'SIGTERM') } catch {} }
    p.kept = (p.kept + chunk.toString()).slice(-32768)
    for (const code of chunk.toString().match(/\bB\d{4}\b/g) || []) diagnosticCodes.add(code)
  })
  p.on('error', () => { p.startFailed = true }); children.push(p); return p
}
async function stop(p) {
  if (!p?.pid) return
  try { process.kill(-p.pid, 'SIGTERM') } catch {}
  const end = Date.now() + 3000
  while (p.exitCode === null && p.signalCode === null && Date.now() < end) await sleep(30)
  try { process.kill(-p.pid, 'SIGKILL') } catch {}
  if (p.exitCode === null && p.signalCode === null) await Promise.race([new Promise(resolve => p.once('exit', resolve)), sleep(1000)])
}
async function ready(url, p) {
  const end = Date.now() + 20000
  while (Date.now() < end) {
    ensure(!p.startFailed && p.exitCode === null && p.signalCode === null, 'OWNED_SERVICE_EXITED')
    try { const response = await fetch(url, { signal: AbortSignal.timeout(700) }); if (response.ok) return } catch {}
    await sleep(100)
  }
  throw Error('OWNED_SERVICE_READY_DEADLINE')
}
class CDP {
  constructor(socket) {
    this.ws = socket; this.next = 1; this.pending = new Map(); this.listeners = []
    socket.addEventListener('message', event => {
      const value = JSON.parse(event.data)
      if (value.id) { const pending = this.pending.get(value.id); if (!pending) return; this.pending.delete(value.id); clearTimeout(pending.timer); value.error ? pending.reject(Error('CDP_COMMAND_' + pending.method.replace(/[^A-Za-z0-9]/g,'_').toUpperCase())) : pending.resolve(value.result); }
      else for (const listener of this.listeners) { try { listener(value) } catch { this.eventError = true } }
    })
  }
  call(method, params = {}, sessionId) {
    ensure(this.pending.size < 64, 'CDP_PENDING_LIMIT')
    const id = this.next++
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => { this.pending.delete(id); reject(Error('CDP_DEADLINE_' + method.replace(/[^A-Za-z0-9]/g,'_').toUpperCase())) }, 7000)
      this.pending.set(id, { resolve, reject, timer, method }); this.ws.send(JSON.stringify({ id, method, params, ...(sessionId ? { sessionId } : {}) }))
    })
  }
  close() { for (const pending of this.pending.values()) { clearTimeout(pending.timer); pending.reject(Error('CDP_DISPOSED')) }; this.pending.clear(); this.ws.close() }
}
async function evaluate(page, expression) {
  ensure(!cdp.eventError, 'BROWSER_PROTOCOL_EVENT_FAILED')
  const result = await cdp.call('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true }, page.session)
  ensure(!result.exceptionDetails, 'PAGE_EVALUATION_FAILED')
  return result.result.value
}
async function wait(page, expression, code) {
  const end = Date.now() + 12000
  while (Date.now() < end) { if (await evaluate(page, expression)) return; await sleep(75) }
  throw Error(code)
}
async function click(page, selector) { await evaluate(page, `document.querySelector(${JSON.stringify(selector)}).click(); true`) }
async function cookies(page, actor) {
  for (const name of ['__Host-brain', '__Host-brain-access', '__Host-brain-refresh']) await cdp.call('Network.deleteCookies', { name, url: origin + '/' }, page.session)
  if (actor) await cdp.call('Network.setCookies', { cookies: actor.cookies.map(({ name, value }) => ({ name, value, url: origin + '/', path: '/', secure: true, httpOnly: true, sameSite: 'Lax' })) }, page.session)
}
async function newPage(actor) {
  const context = (await cdp.call('Target.createBrowserContext')).browserContextId; contexts.push(context)
  const target = (await cdp.call('Target.createTarget', { url: 'about:blank', browserContextId: context })).targetId
  const session = (await cdp.call('Target.attachToTarget', { targetId: target, flatten: true })).sessionId
  const page = { context, target, session, frames: [], nativeIds: new Set(), failures: [], exceptions: [], documentStatus: null, created: 0, closed: 0, errors: 0 }; pages.push(page)
  cdp.listeners.push(event => {
    if (event.sessionId !== session) return
    if (event.method === 'Network.webSocketCreated' && event.params.url.endsWith('/app/_brain/socket')) { ensure(page.nativeIds.size < 32, 'BROWSER_SOCKET_RETENTION_BOUND'); page.nativeIds.add(event.params.requestId); page.created++ }
    if (event.method === 'Network.webSocketClosed' && page.nativeIds.has(event.params.requestId)) page.closed++
    if (event.method === 'Runtime.exceptionThrown') { page.errors++; const details=event.params.exceptionDetails; page.exceptions.push({kind:details.exception?.className||'Exception',line:details.lineNumber,column:details.columnNumber,frames:(details.stackTrace?.callFrames||[]).slice(0,4).map(frame=>({path:(()=>{try{return new URL(frame.url).pathname}catch{return 'inline'}})(),line:frame.lineNumber,column:frame.columnNumber})),classification:/^TypeError: account\.get is not a function/.test(details.exception?.description||'')?'ACCOUNT_GET_NOT_CALLABLE':/^TypeError: Cannot read properties of (?:null|undefined)/.test(details.exception?.description||'')?'NULLISH_PROPERTY_ACCESS':/^SyntaxError:/.test(details.exception?.description||'')?'INVALID_JAVASCRIPT_INPUT':'UNCLASSIFIED_EXCEPTION'}); if(page.exceptions.length>8)page.exceptions.shift() }
    if(event.method==='Network.responseReceived'&&event.params.type==='Document')page.documentStatus=event.params.response.status
    if(event.method==='Network.loadingFailed'){const kind=event.params.errorText;if(/^net::ERR_[A-Z_]+$/.test(kind)&&page.failures.length<8)page.failures.push(kind)}
    if (event.method !== 'Network.webSocketFrameReceived' || !page.nativeIds.has(event.params.requestId)) return
    const value = frameMetadata(event.params.response)
    if (!value) return
    ensure(page.frames.length < 128, 'BROWSER_FRAME_RETENTION_BOUND'); page.frames.push(value)
  })
  await cdp.call('Network.enable', {}, session); await cdp.call('Runtime.enable', {}, session)
  await cookies(page, actor); await cdp.call('Page.navigate', { url: base + '/' }, session)
  await wait(page, `!!document.querySelector('#activate')`, 'COMPILED_VIEW_DID_NOT_MOUNT')
  return page
}
async function activate(page, actor) {
  await click(page, '#activate')
  await wait(page, `document.querySelector('#principal')?.textContent === ${JSON.stringify(actor.id)} && document.querySelector('#live-connected')?.textContent === 'yes'`, 'AUTHENTICATED_LIVE_JOIN_DID_NOT_COMPLETE')
}
async function request(actor, method, url, input) {
  const response = await fetch(base + url, { method, headers: { Origin: origin, ...(input ? { 'Content-Type': 'application/json' } : {}), ...(actor ? { Cookie: actor.cookies.map(({name,value}) => name + '=' + value).join('; ') } : {}) }, ...(input ? { body: JSON.stringify(input) } : {}), signal: AbortSignal.timeout(6000) })
  const text = await response.text(); ensure(text.length < 65536, 'HTTP_RESPONSE_BOUND')
  let value; try { value = JSON.parse(text) } catch { throw Error('HTTP_JSON_RESPONSE_REQUIRED') }
  return { status: response.status, value, headers: response.headers }
}
async function actor(label) {
  const credentials = { email: `live-${label}-${crypto.randomUUID()}@example.test`, password: crypto.randomBytes(24).toString('base64url') }
  const signup = await request(null, 'POST', '/_brain/auth/signup', credentials); ensure(signup.status === 200, 'NATIVE_SIGNUP_FAILED')
  const login = await request(null, 'POST', '/_brain/auth/login', credentials); ensure(login.status === 200, 'NATIVE_LOGIN_FAILED')
  const cookies = login.headers.getSetCookie().map(header => { const pair = header.split(';')[0], at = pair.indexOf('='); return { name: pair.slice(0,at), value: pair.slice(at+1) } }).filter(cookie => cookie.name.startsWith('__Host-brain'))
  ensure(cookies.some(cookie => cookie.name === '__Host-brain-access'), 'REAL_JWT_ACCESS_COOKIE_REQUIRED')
  return { id: login.value.user.id, cookies }
}
async function create(actor, title) {
  const result = await request(actor, 'POST', '/api/posts', { title, secret: 'private-row-body-fixture' })
  ensure(result.status === 200, 'COMMITTED_CREATE_FAILED'); return result.value
}
const textHas = title => `Array.from(document.querySelectorAll('#records li')).some(node => node.textContent === ${JSON.stringify(title)})`
async function proof(name, callback) { await callback(); results.push({ name, status: 'PASS' }); console.log(JSON.stringify({ kind: 'case', name, status: 'PASS' })) }
const deadline = setTimeout(() => { console.error(JSON.stringify({ ok:false, code:'FULLCHAIN_TOTAL_DEADLINE' })); process.exit(124) }, 180000)
try {
  fs.copyFileSync('/binary/brain', binary); fs.chmodSync(binary, 0o755); fs.cpSync('/fixture', root, { recursive: true })
  fs.writeFileSync(path.join(root, 'brain.json'), JSON.stringify({ name:'native-live-acceptance',base:'/app/',backend:{origins:[origin],sockets:{allowedOrigins:[origin]}} }))
  const env = { ...process.env, PUBLIC_SITE_URL: origin, BRAIN_BACKEND_URL: 'http://127.0.0.1:5182', BRAIN_JWT_SECRET: crypto.randomBytes(48).toString('base64url'), BRAIN_JWT_ISSUER: origin, BRAIN_JWT_AUDIENCE: origin }
  const databaseURL=new URL(env.DATABASE_URL||'')
  ensure(['postgres:','postgresql:'].includes(databaseURL.protocol)&&databaseURL.hostname==='owned-pg'&&databaseURL.port==='5432'&&databaseURL.username==='brain'&&/^[a-f0-9]{48}$/.test(databaseURL.password)&&/^\/brain_live_[a-f0-9]{12}$/.test(databaseURL.pathname)&&!databaseURL.search&&!databaseURL.hash,'ONLY_OWNED_DISPOSABLE_DATABASE_ALLOWED')
  const migration = spawnSync(binary, ['backend','migrate','--env','development'], {cwd:root,env,encoding:'utf8',timeout:20000,maxBuffer:65536})
  setupDiagnostics.migrationStatus=migration.status;setupDiagnostics.migrationErrorKind=migration.error?.code;setupDiagnostics.migrationCategories=[...new Set(((migration.stderr||'')+(migration.stdout||'')).match(/\b(?:BDB_[A-Z_]+|DATABASE_URL|unavailable|refused|timeout|schema|fields|meta|contract|invalid)\b/g)||[])];
  ensure(migration.status === 0, 'EXPLICIT_NATIVE_MIGRATION_FAILED')
  const backend = child(binary, ['backend','serve','--port','5182','--env','development'], {env}); await ready('http://127.0.0.1:5182/health', backend)
  const frontEnv={...env};for(const key of ['DATABASE_URL','BRAIN_JWT_SECRET','BRAIN_JWT_ISSUER','BRAIN_JWT_AUDIENCE'])delete frontEnv[key]
  const frontend = child(binary, ['run','--port','4180','--host','127.0.0.1','--no-inspect','--quiet','--json'], {env:frontEnv}); await ready(base + '/', frontend)
  const alice = await actor('alice'), bob = await actor('bob')
  const initialA = await create(alice, 'private-alice-initial'), initialB = await create(bob, 'private-bob-initial')
  chrome = child('/usr/bin/chromium', ['--headless','--no-sandbox','--disable-dev-shm-usage','--no-first-run','--disable-background-networking','--remote-debugging-port=0','--user-data-dir='+profileRoot,'about:blank'], {env:{PATH:'/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin',HOME:profileRoot,TMPDIR:'/tmp',LANG:'C.UTF-8'}})
  const chromeEnd = Date.now()+12000; let endpoint
  while(Date.now()<chromeEnd){endpoint=chrome.kept.match(/DevTools listening on (ws:\/\/[^\s]+)/)?.[1];if(endpoint)break;await sleep(40)}
  ensure(endpoint,'OWNED_CHROME_DEBUG_ENDPOINT_MISSING')
  const socket=new WebSocket(endpoint);await new Promise((resolve,reject)=>{const timer=setTimeout(()=>reject(Error('CDP_CONNECT_DEADLINE')),5000);socket.addEventListener('open',()=>{clearTimeout(timer);resolve()},{once:true});socket.addEventListener('error',()=>{clearTimeout(timer);reject(Error('CDP_CONNECT_FAILED'))},{once:true})});cdp=new CDP(socket)
  const a=await newPage(alice), b=await newPage(bob), anonymous=await newPage(null)
  setupDiagnostics.sdkShape=await evaluate(a,`Promise.all([import('/app/api/account.jsbrain'),import('/app/api/posts.jsbrain')]).then(modules=>modules.map(module=>({kind:typeof module.default,get:typeof module.default?.get,methods:Object.keys(module.default||{}).filter(name=>['get','post','put','delete'].includes(name))})))`);ensure(setupDiagnostics.sdkShape.every(shape=>shape.get==='function'),'COMPILED_API_METHOD_MISSING')
  await proof('compiled owner-scoped HTTP handle joins native JWT sockets through CLI gateway', async()=>{ensure(a.created===0&&b.created===0,'NULL_SCOPE_MUST_NOT_CONNECT');await activate(a,alice);await activate(b,bob);await wait(a,textHas('private-alice-initial'),'OWNER_INITIAL_ROWS_MISSING');await wait(b,textHas('private-bob-initial'),'SECOND_OWNER_INITIAL_ROWS_MISSING');ensure(!await evaluate(a,textHas('private-bob-initial'))&&!await evaluate(b,textHas('private-alice-initial')),'OWNER_HTTP_ISOLATION_FAILED')})
  await proof('anonymous HTTP and browser Upgrade disclose no private row body', async()=>{const denied=await request(null,'GET','/api/posts');ensure(denied.status===401&&!JSON.stringify(denied.value).includes('private-'),'ANONYMOUS_HTTP_DISCLOSURE');const state=await evaluate(anonymous,`new Promise(resolve=>{const socket=new WebSocket(${JSON.stringify(origin.replace('http:','ws:')+'/app/_brain/socket')},'brain.v1');const timer=setTimeout(()=>{socket.close();resolve('timeout')},5000);socket.onopen=()=>{clearTimeout(timer);socket.close();resolve('open')};socket.onerror=()=>{clearTimeout(timer);resolve('denied')};socket.onclose=()=>{clearTimeout(timer);resolve('denied')}})`);ensure(state==='denied','ANONYMOUS_UPGRADE_MUST_FAIL');ensure(!anonymous.frames.length,'ANONYMOUS_SOCKET_FRAME_DISCLOSURE')})
  let record
  await proof('committed create update and delete refresh DOM without leaking identifiers across owners',async()=>{record=await create(alice,'private-alice-created');await wait(a,textHas('private-alice-created'),'CREATE_INVALIDATION_DID_NOT_REFRESH');const updated=await request(alice,'PUT','/api/posts/'+record.id,{title:'private-alice-updated',secret:'private-row-body-fixture'});ensure(updated.status===200,'COMMITTED_UPDATE_FAILED');await wait(a,textHas('private-alice-updated'),'UPDATE_INVALIDATION_DID_NOT_REFRESH');const denied=await request(bob,'GET','/api/posts/'+record.id);ensure(denied.status===403&&!JSON.stringify(denied.value).includes('private-'),'OTHER_OWNER_ROW_DISCLOSURE');ensure((await request(alice,'DELETE','/api/posts/'+record.id)).status===200,'COMMITTED_DELETE_FAILED');await wait(a,`!(${textHas('private-alice-updated')})`,'DELETE_INVALIDATION_DID_NOT_REFRESH');await sleep(400);ensure(!b.frames.some(frame=>frame.type==='invalidate'&&[record.id,initialA.id].includes(frame.id)),'OTHER_OWNER_METADATA_DISCLOSURE');for(const operation of ['create','update','delete'])ensure(a.frames.some(frame=>frame.type==='invalidate'&&frame.id===record.id&&frame.operation===operation),'COMMITTED_CRUD_METADATA_MISSING')})
  await proof('reconnect rejoins and resynchronizes a mutation missed while scoped off',async()=>{const count=a.created;await click(a,'#clear');await wait(a,`document.querySelector('#live-connected')?.textContent==='no'`,'SCOPE_CLEAR_DID_NOT_CLOSE');await create(alice,'private-alice-offline');await activate(a,alice);await wait(a,textHas('private-alice-offline'),'MISSED_MUTATION_NOT_RESYNCHRONIZED');ensure(a.created>count,'RECONNECT_DID_NOT_CREATE_FRESH_SOCKET');const again=a.created;await click(a,'#reconnect');await wait(a,`document.querySelector('#live-connected')?.textContent==='yes'`,'EXPLICIT_RECONNECT_DID_NOT_JOIN');await sleep(200);ensure(a.created>again,'EXPLICIT_RECONNECT_DID_NOT_REPLACE_SOCKET')})
  await proof('principal switch closes previous generation and clears private HTTP view state',async()=>{await click(a,'#clear');await wait(a,`document.querySelector('#principal')?.textContent==='none' && document.querySelectorAll('#records li[data-record]').length===0`,'PRIVATE_VIEW_STATE_NOT_CLEARED');await cookies(a,bob);await activate(a,bob);await wait(a,textHas('private-bob-initial'),'NEW_PRINCIPAL_NOT_RESYNCHRONIZED');ensure(!await evaluate(a,textHas('private-alice-initial'))&&!await evaluate(a,textHas('private-alice-offline')),'PREVIOUS_PRINCIPAL_STATE_DISCLOSURE');const created=await create(alice,'private-alice-after-switch');await sleep(400);ensure(!a.frames.slice(-8).some(frame=>frame.type==='invalidate'&&frame.id===created.id),'OLD_SCOPE_METADATA_DISCLOSURE')})
  await proof('revoked JWT sessions close on delivery and disposed owners release browser sockets',async()=>{ensure((await request(bob,'POST','/_brain/auth/logout')).status===200,'REAL_LOGOUT_FAILED');await create(alice,'private-alice-after-revocation');await wait(b,`document.querySelector('#live-connected')?.textContent==='no'`,'REVOKED_SESSION_SOCKET_STILL_CONNECTED');await cookies(anonymous,alice);await activate(anonymous,alice);const before=anonymous.closed,created=anonymous.created;await cdp.call('Page.navigate',{url:base+'/other'},anonymous.session);await wait(anonymous,`!!document.querySelector('#disposed')`,'OWNER_NAVIGATION_DID_NOT_COMPLETE');const end=Date.now()+3000;while(anonymous.closed<=before&&Date.now()<end)await sleep(40);ensure(anonymous.closed>before,'DISPOSED_OWNER_SOCKET_NOT_CLOSED');await sleep(300);ensure(anonymous.created===created,'DISPOSED_OWNER_RECONNECTED');ensure(a.frames.every(frame=>frame.type!=='invalidate'||Object.keys(frame).length===5),'ROW_BODY_IN_PROTOCOL')})
  const version=await cdp.call('Browser.getVersion');done=true
  console.log(JSON.stringify({schemaVersion:1,ok:true,browser:version.product,viewer:{used:false,reason:'multiple-connected-profiles-without-explicit-selection'},cases:results,framesObserved:pages.reduce((n,page)=>n+page.frames.length,0),cookiesCaptured:false,valuesCaptured:false}))
} catch(error){const pageStates=[];if(cdp)for(const page of pages){try{const value=await cdp.call('Runtime.evaluate',{expression:`({ready:document.readyState,hasActivate:!!document.querySelector('#activate'),hasError:!!document.querySelector('#http-error'),hasMain:!!document.querySelector('main')})`,returnByValue:true},page.session);pageStates.push({state:value.result.value,status:page.documentStatus,exceptions:page.exceptions,failedResources:page.failures})}catch{}}console.error(JSON.stringify({schemaVersion:1,ok:false,pageStates,code:typeof error.message==='string'&&/^[A-Z0-9_]+$/.test(error.message)?error.message:'FULLCHAIN_FAILED',compilerCodes:[...diagnosticCodes].slice(0,32),setupDiagnostics,childStates:children.map(p=>({kind:p.spawnargs[1]==='backend'?'backend':p.spawnargs[1]==='run'?'dev':'chrome',exit:p.exitCode,signal:p.signalCode,bytes:p.bytes,outputBoundHit:!!p.outputBoundHit,diagnosticKinds:[...new Set(p.kept.match(/\b(?:panicked|EAGAIN|ENOMEM|OOM|SIGSEGV|TypeError|thread|poisoned)\b/g)||[])],sourceLocations:[...p.kept.matchAll(/(crates\/[A-Za-z0-9_\/-]+\.rs):(\d+):(\d+)/g)].map(m=>({file:m[1],line:Number(m[2]),column:Number(m[3])})).slice(-8)})),cases:results,valuesCaptured:false}));process.exitCode=1}
finally{clearTimeout(deadline);let disposed=0;if(cdp){await Promise.allSettled(contexts.map(async context=>{await cdp.call('Target.disposeBrowserContext',{browserContextId:context});disposed++}));cdp.close()}for(const p of children.reverse())await stop(p);fs.rmSync(root,{recursive:true,force:true});fs.rmSync(profileRoot,{recursive:true,force:true});console.log(JSON.stringify({kind:'cleanup',ownedChildrenReaped:children.every(p=>p.exitCode!==null||p.signalCode!==null),browserContextsCreated:contexts.length,browserContextsDisposed:disposed,completed:done}))}
