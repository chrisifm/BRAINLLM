#!/usr/bin/env node
// Project-owned verification over real HTTP, with exact raw bytes and no ambient browser/backend authority.
import assert from 'node:assert/strict'
import { generateKeyPairSync, randomUUID, sign } from 'node:crypto'
import { existsSync, mkdirSync, readFileSync, readdirSync, rmSync } from 'node:fs'
import { createServer, request } from 'node:http'
import { join } from 'node:path'
import { binary, brain, createDatabase, freePort, gate, own, ready, sleep, stop, write } from './lib.mjs'

const origin = 'https://example.test'
const verifier = `<meta server>
import { verify as verifySignature } from 'node:crypto'
import { existsSync, writeFileSync, appendFileSync } from 'node:fs'
export default async function verify(ctx) {
  if (Object.keys(ctx).sort().join(',') !== 'env,headers,rawBody,rawBodyEncoding') throw new Error('Unexpected verifier capability')
  if (ctx.env.BRAIN_ROUTES_API_KEY || ctx.env.BRAIN_ROUTES_SYSTEM_KEY) throw new Error('Unexpected verifier key')
  const event = ctx.headers['x-event-id']
  const mode = ctx.headers['x-event-mode']
  const signature = ctx.headers['x-event-signature']
  if (ctx.headers['x-key-url'] !== ctx.env.CUSTOM_VERIFY_KEY_URL) return false
  if (typeof event !== 'string' || !/^[a-z0-9-]{1,80}$/.test(event) || typeof signature !== 'string' || signature.length > 1024) return false
  const bytes = Buffer.from(ctx.rawBody, ctx.rawBodyEncoding)
  const response = await fetch(ctx.env.CUSTOM_VERIFY_KEY_URL, { signal: AbortSignal.timeout(2000), redirect: 'error' })
  if (!response.ok) return false
  const publicKey = await response.text()
  if (publicKey.length > 8192) return false
  const message = Buffer.concat([Buffer.from(event + '\\n' + mode + '\\n'), bytes])
  if (!verifySignature('sha256', message, publicKey, Buffer.from(signature, 'base64'))) return false
  if (mode === 'invalid') return 'true'
  if (mode === 'false') return false
  if (mode === 'throw') throw new Error('private verifier failure')
  if (mode === 'crash') process.exit(37)
  if (mode === 'timeout') {
    await new Promise(resolve => setTimeout(resolve, 1600))
    appendFileSync(ctx.env.CUSTOM_VERIFY_DIR + '/late.txt', 'verifier escaped deadline\\n')
  }
  if (mode === 'busy') {
    Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 1600)
    appendFileSync(ctx.env.CUSTOM_VERIFY_DIR + '/late.txt', 'busy verifier escaped deadline\\n')
  }
  if (mode === 'joint') await new Promise(resolve => setTimeout(resolve, 300))
  if (mode === 'replay') {
    const path = ctx.env.CUSTOM_VERIFY_DIR + '/' + event
    if (existsSync(path)) return false
    try { writeFileSync(path, 'accepted', { flag: 'wx' }) } catch { return false }
  }
  return true
}
`
const handler = `<meta server>
import { appendFileSync } from 'node:fs'
export default async function handle(ctx) {
  appendFileSync(ctx.env.CUSTOM_VERIFY_DIR + '/calls.txt', ctx.headers['x-event-id'] + '\\n')
  if (ctx.headers['x-event-mode'] === 'large') return { rawLength: ctx.rawBody.length, encoding: ctx.rawBodyEncoding }
  if (ctx.headers['x-event-mode'] === 'joint') {
    await new Promise(resolve => setTimeout(resolve, 300))
    appendFileSync(ctx.env.CUSTOM_VERIFY_DIR + '/late.txt', 'handler escaped joint deadline\\n')
  }
  return { rawBody: ctx.rawBody, encoding: ctx.rawBodyEncoding, verified: ctx.verified, user: ctx.user, backend: typeof ctx.backend, headers: ctx.headers }
}
`

await gate('custom-route-verifier', 6 * 60 * 1000, async directory => {
  // This is a local RSA-key adapter, not an assertion that a provider certificate chain has been validated.
  const keys=generateKeyPairSync('rsa',{modulusLength:2048})
  const publicKey=keys.publicKey.export({type:'spki',format:'pem'})
  let keyFetches=0,foreignFetches=0
  const keyServer=createServer((req,res)=>{keyFetches++;res.writeHead(200,{'Content-Type':'text/plain'});res.end(publicKey)})
  const foreignServer=createServer((req,res)=>{foreignFetches++;res.writeHead(200,{'Content-Type':'text/plain'});res.end(publicKey)})
  await new Promise(resolve=>keyServer.listen(0,'127.0.0.1',resolve))
  await new Promise(resolve=>foreignServer.listen(0,'127.0.0.1',resolve))
  try {
  const keyURL=`http://127.0.0.1:${keyServer.address().port}/keys/current`
  const foreignURL=`http://127.0.0.1:${foreignServer.address().port}/keys/foreign`
  const database = createDatabase('gate_custom_verifier')
  const env = { DATABASE_URL: database, PUBLIC_SITE_URL: origin, BRAIN_MAIL_RESEND_KEY: '', GOOGLE_CLIENT_ID: '', BRAIN_JWT_SECRET: '',
    CUSTOM_VERIFY_KEY_URL: keyURL, CUSTOM_VERIFY_DIR: join(directory,'verification'),
    BRAIN_ROUTES_SYSTEM_KEY: 'synthetic-system-key-must-never-reach-verification' }
  mkdirSync(env.CUSTOM_VERIFY_DIR)
  brain(['new','app','--json'],directory,env)
  const app = join(directory,'app')
  const envFile = readFileSync(join(app,'.brain_env'),'utf8')
  write(app,'.brain_env',envFile+'\nCUSTOM_VERIFY_KEY_URL?=\nCUSTOM_VERIFY_DIR?=\n')
  write(app,'server/routes/events/route.jsbrain',"<meta server>\nexport default { method: 'POST', path: '/events/custom', auth: 'public', body: 'raw', bodyLimit: '1mb', timeout: '500ms', headers: ['x-event-id', 'x-event-signature', 'x-key-url', 'x-event-mode', 'content-type', 'origin', 'accept'], verify: { scheme: 'custom' } }\n")
  write(app,'server/routes/events/handler.jsbrain',handler)
  for (const [name,path,method,timeout] of [['events_large','/events/large','POST','3s'],['events_delete','/events/delete','DELETE','500ms']]) {
    write(app,`server/routes/${name}/route.jsbrain`,`<meta server>\nexport default { method: '${method}', path: '${path}', auth: 'public', body: 'raw', bodyLimit: '1mb', timeout: '${timeout}', headers: ['x-event-id', 'x-event-signature', 'x-key-url', 'x-event-mode', 'content-type', 'origin', 'accept'], verify: { scheme: 'custom' } }\n`)
    write(app,`server/routes/${name}/handler.jsbrain`,handler)
    write(app,`server/routes/${name}/verify.jsbrain`,verifier)
  }
  write(app,'server/routes/ordinary/route.jsbrain',"<meta server>\nexport default { method: 'POST', path: '/events/ordinary', auth: 'public', body: 'raw', timeout: '500ms' }\n")
  write(app,'server/routes/ordinary/handler.jsbrain',"<meta server>\nimport { appendFileSync } from 'node:fs'\nexport default async function handle(ctx) { if (ctx.rawBody === 'timeout') { await new Promise(resolve => setTimeout(resolve, 1600)); appendFileSync(ctx.env.CUSTOM_VERIFY_DIR + '/late.txt', 'ordinary handler escaped deadline') } return { ordinary: true } }\n")
  // One unrelated long route forces the pool's startup maximum above both tested 500 ms route budgets.
  write(app,'server/routes/wide/route.jsbrain',"<meta server>\nexport default { path: '/events/wide', auth: 'public', timeout: '20s' }\n")
  write(app,'server/routes/wide/handler.jsbrain',"<meta server>\nexport default async function handle() { return { wide: true } }\n")
  brain(['backend','migrate','--json'],app,env)
  const nativePort = await freePort(), nativeURL = `http://127.0.0.1:${nativePort}`
  // Missing and non-callable companions fail startup; no unverified handler is ever served.
  for (const source of [null,'<meta server>\nexport default 7\n']) {
    if (source!==null) write(app,'server/routes/events/verify.jsbrain',source)
    const refused = own(binary,['backend','serve','--port',String(nativePort),'--env','production'],app,env)
    const until = Date.now()+15000
    while (refused.child.exitCode===null && refused.child.signalCode===null && Date.now()<until) await sleep(50)
    assert.ok(refused.child.exitCode!==null && refused.child.exitCode!==0,refused.output.text)
    assert.match(refused.output.text,/verify\.jsbrain/)
    await stop(refused.child)
  }
  write(app,'server/routes/events/verify.jsbrain',verifier)
  let native
  const backend = async () => {
    if (native) await stop(native.child)
    native=own(binary,['backend','serve','--port',String(nativePort),'--env','production'],app,env)
    await ready(nativeURL+'/health',native)
  }
  await backend()
  const auth = async (path,body) => {
    const response=await fetch(nativeURL+path,{method:'POST',headers:{Origin:origin,'Content-Type':'application/json',Accept:'application/json'},body:JSON.stringify(body),signal:AbortSignal.timeout(5000)})
    assert.equal(response.status,200,await response.clone().text())
    return response
  }
  const account={email:'verifier-user@example.test',password:'synthetic-verifier-account-password'}
  await auth('/_brain/auth/signup',account)
  const login=await auth('/_brain/auth/login',account)
  const cookie=login.headers.getSetCookie().find(value=>/^__Host-brain(?:-access)?=/.test(value))?.split(';')[0]
  assert.ok(cookie,'a real browser session is used to prove it does not grant verifier authority')
  const identity=await fetch(nativeURL+'/_brain/auth/me',{headers:{Cookie:cookie,Accept:'application/json'},signal:AbortSignal.timeout(5000)})
  assert.equal(identity.status,200,'the incidental cookie must actually authenticate on an ordinary session endpoint')
  await identity.body?.cancel()
  const count=()=>existsSync(join(env.CUSTOM_VERIFY_DIR,'calls.txt'))?readFileSync(join(env.CUSTOM_VERIFY_DIR,'calls.txt'),'utf8').split('\n').filter(Boolean).length:0
  const post = async (base,{mode='valid',body=Buffer.from('first\r\nsecond'),id=randomUUID(),signature=true,cookie:session,alter=false,wrongSignature=false,key=keyURL,path='/events/custom',method='POST'}={}) => {
    const signatureBytes=sign('sha256',Buffer.concat([Buffer.from(id+'\n'+mode+'\n'),body]),keys.privateKey)
    if(wrongSignature)signatureBytes[0]^=1
    const signatureValue=signatureBytes.toString('base64')
    const response=await fetch(base+path,{method,headers:{Accept:'application/json','Content-Type':'application/octet-stream','x-event-id':id,'x-event-mode':mode,
      ...(key?{'x-key-url':key}:{}),...(signature?{'x-event-signature':signatureValue}:{}),...(session?{Cookie:session}:{})},body:alter?Buffer.concat([body,Buffer.from('!')]):body,signal:AbortSignal.timeout(5000),redirect:'manual'})
    return {status:response.status,body:await response.json()}
  }
  const duplicated = (base,field) => new Promise((resolve,reject)=>{
    const id=randomUUID(),body=Buffer.from('duplicated header')
    const signature=sign('sha256',Buffer.concat([Buffer.from(id+'\nvalid\n'),body]),keys.privateKey).toString('base64')
    const headers=['Host',new URL(base).host,'Content-Type','application/octet-stream','Accept','application/json','Content-Length',String(body.length),
      'x-event-id',id,'x-event-mode','valid','x-event-signature',signature,'x-key-url',keyURL,field,field==='content-type'?'text/plain':'different-event']
    const call=request(new URL('/events/custom',base),{method:'POST',headers},response=>{
      response.resume();response.once('end',()=>resolve(response.statusCode))
    })
    call.setTimeout(5000,()=>call.destroy(new Error('Owned duplicate-header probe exceeded its deadline')))
    call.once('error',reject);call.end(body)
  })
  const verifySurface = async (label,base) => {
    for (const body of [Buffer.from('first\r\nsecond'),Buffer.from([0,255,128,13,10,1])]) {
      const accepted=await post(base,{body,cookie})
      assert.equal(accepted.status,200,label+JSON.stringify(accepted))
      assert.equal(accepted.body.rawBody,body.toString(body.includes(255)?'base64':'utf8'))
      assert.equal(accepted.body.encoding,body.includes(255)?'base64':'utf8')
      assert.deepEqual(accepted.body.verified,{scheme:'custom'})
      assert.equal(accepted.body.user,null)
      assert.equal(accepted.body.backend,'undefined')
      assert.equal(accepted.body.headers.cookie,undefined)
      assert.equal(accepted.body.headers.authorization,undefined)
      assert.equal(accepted.body.headers.origin,undefined,'the gateways must not manufacture a browser Origin')
    }
    assert.equal((await post(base,{path:'/events/delete',method:'DELETE'})).status,200,'raw signed DELETE follows the same verified mutation contract')
    const large=await post(base,{path:'/events/large',mode:'large',body:Buffer.alloc(1024*1024)})
    assert.equal(large.status,200,label+' the full raw body limit remains usable after JSON escaping: '+JSON.stringify(large))
    assert.deepEqual(large.body,{rawLength:1024*1024,encoding:'utf8'})
    for (const options of [{signature:false},{alter:true},{wrongSignature:true},{mode:'false'},{mode:'invalid'},{mode:'throw'},{mode:'crash'}]) {
      const before=count(),rejected=await post(base,options)
      assert.equal(rejected.status,options.mode==='throw'||options.mode==='crash'?503:401,label+JSON.stringify(rejected))
      assert.equal(count(),before,'unverified work must never reach the handler')
      assert.ok(!JSON.stringify(rejected).includes('private verifier failure'))
    }
    for (const key of [null,foreignURL]) {
      const before=count(),fetches=keyFetches,rejected=await post(base,{key})
      assert.equal(rejected.status,401)
      assert.equal(count(),before)
      assert.equal(keyFetches,fetches,'untrusted key URLs are refused before any key fetch')
      assert.equal(foreignFetches,0,'an untrusted request can never trigger a fetch to the foreign key server')
    }
    for (const field of ['x-event-id','x-event-signature','content-type','accept']) {
      const beforeDuplicate=count()
      assert.ok([400,401].includes(await duplicated(base,field)),'duplicate verification headers must fail before the handler: '+field)
      assert.equal(count(),beforeDuplicate)
    }
    const id=randomUUID()
    assert.equal((await post(base,{mode:'replay',id})).status,200)
    const before=count()
    assert.equal((await post(base,{mode:'replay',id})).status,401)
    assert.equal(count(),before,'replay rejection is owned by this application adapter')
    for (const mode of ['timeout','busy','joint']) {
      const begin=Date.now(),before=count(),timedOut=await post(base,{mode})
      assert.equal(timedOut.status,504,label+' '+mode+' '+JSON.stringify(timedOut))
      assert.equal(timedOut.body.error.code,'ROUTE_TIMEOUT')
      assert.ok(Date.now()-begin<1400,'verification and handler share one 500 ms deadline')
      if (mode!=='joint') assert.equal(count(),before,'timed-out verification never reaches the handler')
    }
    const ordinaryTimeout=await fetch(base+'/events/ordinary',{method:'POST',headers:{Origin:origin,Accept:'application/json','Content-Type':'application/octet-stream'},body:'timeout',signal:AbortSignal.timeout(5000)})
    assert.equal(ordinaryTimeout.status,504,'ordinary handlers also receive their actual remaining route budget')
    assert.equal((await ordinaryTimeout.json()).error.code,'ROUTE_TIMEOUT')
    await sleep(1700)
    assert.ok(!existsSync(join(env.CUSTOM_VERIFY_DIR,'late.txt')),'deadlines kill the actual sidecar work; an abandoned future is insufficient')
    assert.equal((await post(base)).status,200,'capacity recovers after verifier failures and actual process timeouts')
    const ordinary=await fetch(base+'/events/ordinary',{method:'POST',headers:{Accept:'application/json','Content-Type':'application/octet-stream'},body:'payload',signal:AbortSignal.timeout(5000)})
    assert.equal(ordinary.status,403,label+' ordinary public mutation keeps its Origin gate')
    assert.equal((await ordinary.json()).error.brainCode,'B0740')
    console.log(`custom-route-verifier: ${label} raw bytes, strict true, refusal, replay, cookie isolation, shared deadline and live sidecar termination verified`)
  }
  await verifySurface('native',nativeURL)
  const gatewayEnv={...env,BRAIN_BACKEND_URL:nativeURL}
  const devPort=await freePort(),dev=own(binary,['run','--port',String(devPort)],app,gatewayEnv)
  await ready(`http://127.0.0.1:${devPort}/`,dev,60000)
  await verifySurface('brain run',`http://127.0.0.1:${devPort}`);await stop(dev.child)
  write(app,'views/live.brain','<meta seo title="Live example" description="A server rendered example page for testing the native verification gateway.">\n<script>export async function load() { return { value: 1 } }</script>\n<main><h1>Live example</h1><p>{props.value}</p></main>\n')
  brain(['build'],app,gatewayEnv)
  for (const path of readdirSync(join(app,'dist/_b'),{recursive:true}).filter(path=>/\.(?:js|mjs)$/.test(path))) {
    const source=readFileSync(join(app,'dist/_b',path),'utf8')
    assert.ok(!source.includes('private verifier failure')&&!source.includes('Unexpected verifier capability'),'private verifier implementation must not reach browser chunks')
  }
  const nodePort=await freePort(),node=own(binary,['start','--port',String(nodePort),'--env','development'],app,gatewayEnv)
  await ready(`http://127.0.0.1:${nodePort}/`,node,60000)
  await verifySurface('Node gateway',`http://127.0.0.1:${nodePort}`);await stop(node.child)
  rmSync(join(app,'dist/server.mjs'))
  const staticPort=await freePort(),fixed=own(binary,['start','--port',String(staticPort),'--env','development'],app,gatewayEnv)
  await ready(`http://127.0.0.1:${staticPort}/`,fixed,60000)
  await verifySurface('Rust static gateway',`http://127.0.0.1:${staticPort}`);await stop(fixed.child)
  } finally {
    await Promise.all([keyServer,foreignServer].map(server=>new Promise(resolve=>{server.close(resolve);server.closeAllConnections()})))
  }
})
