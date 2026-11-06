import { test } from 'node:test'
import assert from 'node:assert/strict'
import { EventEmitter } from 'node:events'
import { createServer, request as httpRequest, STATUS_CODES } from 'node:http'
import { request as httpsRequest } from 'node:https'
import { createServer as socketServer } from 'node:net'
import dns from 'node:dns'
import { startSentry } from '../brain-sentry.js'
import { consent } from '../brain-consent.js'
import { _rt, _err } from '../brain.js'
import { parseDSN, sanitizeEvent, createSentryTunnel } from '../server/sentry.mjs'

const asset = '/assets/01234567.js', id = '01234567-89ab-5def-8123-456789abcdef'
const assets = { [asset]: id }
const dsn = 'https://' + 'a'.repeat(32) + '@o1.ingest.sentry.io/123'
const value = () => ({ schemaVersion: 1, type: 'TypeError', frames: [{ asset, debugId: id, line: 3, column: 19 }] })
const tick = () => new Promise(ok => setImmediate(ok))
function browser() {
  const listeners = new Map(), sent = [], timers = new Set()
  const env = { location: { origin: 'https://example.com', href: 'https://example.com/private?secret=value' }, navigator: {}, AbortController,
    localStorage: { getItem() { return null }, setItem() {} }, Event,
    setTimeout(fn, ms) { const timer = setTimeout(fn, ms); timers.add(timer); return timer }, clearTimeout(timer) { timers.delete(timer); clearTimeout(timer) },
    fetch(url, options) { sent.push({ url, options }); return Promise.resolve({ status: 202 }) },
    addEventListener(type, fn) { if (!listeners.has(type)) listeners.set(type, new Set()); listeners.get(type).add(fn) },
    removeEventListener(type, fn) { listeners.get(type)?.delete(fn) },
    dispatchEvent(event) { for (const fn of listeners.get(event.type) || []) fn(event) },
  }
  env.window = env
  let capture, subscriptions = 0
  const state = startSentry({ base: '', assets: { [asset.slice(1)]: id } }, env, fn => { capture = fn; subscriptions++; return () => { capture = null; subscriptions-- } })
  const error = { name: 'TypeError', message: 'private input', stack: `TypeError: private input\n    at secret (https://example.com${asset}:3:19)\n    at foreign (https://other.example.com${asset}:3:19)\n    at query (https://example.com${asset}?private=value:3:19)` }
  return { env, listeners, sent, timers, state, error, capture: e => capture?.(e), subscriptions: () => subscriptions }
}
test('Sentry collects nothing before literal consent, respects signals, and revokes immediately', async () => {
  const b = browser()
  assert.equal(b.subscriptions(), 0); assert.equal(b.listeners.get('error')?.size || 0, 0)
  b.env.dispatchEvent({ type: 'error', error: b.error }); assert.equal(b.sent.length, 0)
  consent('analytics', true, b.env); assert.equal(b.subscriptions(), 1)
  b.env.dispatchEvent({type:'error',error:b.error}); await tick(); assert.equal(b.sent.length, 1)
  const sent = b.sent[0]
  assert.equal(sent.url, 'https://example.com/_brain/sentry')
  assert.deepEqual(JSON.parse(sent.options.body), value())
  assert.equal(sent.options.credentials, 'omit'); assert.equal(sent.options.referrerPolicy, 'no-referrer'); assert.equal(sent.options.redirect, 'error')
  assert.doesNotMatch(sent.options.body, /private|secret|https|message|stack|user|cookie/)
  consent('analytics', false, b.env); assert.equal(b.subscriptions(), 0); assert.equal(b.listeners.get('error').size, 0)
  b.capture(b.error); await tick(); assert.equal(b.sent.length, 1)
  b.state.stop(); assert.equal([...b.listeners.values()].reduce((n,s) => n+s.size,0), 0); assert.equal(b.timers.size, 0)
  for (const signal of ['dnt', 'gpc']) {
    const c = browser(); c.env.navigator[signal === 'dnt' ? 'doNotTrack' : 'globalPrivacyControl'] = signal === 'dnt' ? '1' : true
    consent('analytics', true, c.env); assert.equal(c.subscriptions(), 0); c.state.stop()
  }
})
test('global rejections use the same sanitizer; revoke and immediate regrant cannot send an earlier deferred report',async()=>{
  const b=browser();consent('analytics',true,b.env)
  b.capture(b.error);consent('analytics',false,b.env);consent('analytics',true,b.env)
  await tick();assert.equal(b.sent.length,0)
  b.env.dispatchEvent({type:'unhandledrejection',reason:b.error});await tick();assert.equal(b.sent.length,1)
  b.state.stop()
  const c=browser();c.env.navigator.doNotTrack='yes';consent('analytics',true,c.env);assert.equal(c.subscriptions(),0);c.state.stop()
})
test('message text cannot invent a stack frame and an accessor cannot replace the allowlisted error class',async()=>{
  const b=browser();consent('analytics',true,b.env)
  b.capture({name:'Error',stack:'Error: https://example.com'+asset+':3:19'});await tick();assert.equal(b.sent.length,0)
  let reads=0;b.capture({get name(){return ++reads===1?'Error':'private value'},stack:'synthetic@https://example.com'+asset+':3:19'})
  await tick();assert.equal(b.sent.length,1);assert.equal(JSON.parse(b.sent[0].options.body).type,'Error');assert.equal(reads,1);b.state.stop()
})
test('revoke aborts in-flight reports; regrant starts clean; hostile errors and throwing fetch cannot leak timers', async () => {
  const b = browser(); consent('analytics', true, b.env)
  b.env.fetch = (url, options) => { b.sent.push({ url, options }); return new Promise((resolve, reject) => options.signal.addEventListener('abort', () => reject(Error('aborted')))) }
  b.capture(b.error); await tick(); consent('analytics', false, b.env)
  assert.equal(b.sent[0].options.signal.aborted, true); await tick(); assert.equal(b.timers.size, 0)
  consent('analytics', true, b.env); b.env.fetch = () => { throw Error('transport failed') }
  b.capture(b.error); await tick(); assert.equal(b.timers.size, 0)
  b.capture({ get stack() { throw Error('hostile') } }); b.state.stop()
})
test('core observers preserve the error hook and boundary; unsubscribe and observer failures are isolated', () => {
  let hook = 0, boundary = 0, observed = 0
  _err(() => hook++)
  const rt = _rt(), off = rt.listenErrors(() => { observed++; throw Error('observer') })
  rt.oops(Error('synthetic'), { $g() { boundary++ } }, 'fx')
  assert.deepEqual([observed,hook,boundary], [1,1,1]); off(); rt.oops(Error('synthetic'), null, 'fx')
  assert.deepEqual([observed,hook,boundary], [1,2,1]); _err(null)
})
test('core error observation is bounded and does not recursively observe its own subscriber failures', () => {
  const rt=_rt(), stops=[];let calls=0,hook=0
  _err(()=>hook++)
  stops.push(rt.listenErrors(()=>{calls++;rt.oops(Error('nested'),null,'fx')}))
  for(let i=0;i<12;i++)stops.push(rt.listenErrors(()=>calls++))
  rt.oops(Error('synthetic'),null,'fx')
  assert.equal(calls,8);assert.equal(hook,2)
  for(const stop of stops)stop()
  _err(null)
})
test('DSN setup is private, HTTPS-only, pinned and actionable; unknown payload properties fail closed', () => {
  assert.equal(parseDSN(dsn).endpoint.href, 'https://o1.ingest.sentry.io/api/123/envelope/')
  const custom = 'https://' + 'a'.repeat(32) + '@errors.example.com/123'
  assert.throws(() => parseDSN(custom), /BRAIN_SENTRY_HOST/)
  assert.equal(parseDSN(custom,'errors.example.com').endpoint.hostname,'errors.example.com')
  for (const invalid of [undefined, 'private invalid', dsn.replace('https:', 'http:'), dsn+'?private=value', dsn+'#private', dsn.replace('/123','/../123'), dsn.replace('/123','/0123'), dsn.replace('/123','/123/'), dsn.replace('@',':private@')]) {
    assert.throws(() => parseDSN(invalid), error => !error.message.includes('private invalid') && /BRAIN_SENTRY_DSN/.test(error.message))
  }
  const clean = sanitizeEvent(value(),assets)
  assert.equal(clean.exception.values[0].value,'Application error'); assert.equal(clean.debug_meta.images[0].debug_id,id)
  for (const bad of [{ ...value(), dsn }, { ...value(), message: 'private' }, { ...value(), frames:[{ ...value().frames[0],asset:asset+'?private=value' }] }, { ...value(), frames:[{ ...value().frames[0],debugId:'00000000-0000-5000-8000-000000000000' }] }]) assert.equal(sanitizeEvent(bad,assets),null)
})
async function tunnel(t, options = {}) {
  const forwarded = []
  const fake = (url, options, callback) => {
    const outgoing = new EventEmitter(); outgoing.destroy = () => {}; outgoing.end = body => {
      forwarded.push({ url, options, body })
      if (options.stall) return
      setImmediate(() => { const incoming = new EventEmitter(); incoming.statusCode=200; incoming.headers={}; incoming.destroy=()=>{}; options.response?.(incoming); callback(incoming) })
    }
    return outgoing
  }
  const transport = createSentryTunnel({ dsn, assets, publicOrigin: 'https://example.com', request: options.request || ((url, settings, cb) => fake(url,{...settings,stall:options.stall,response:options.response},cb)), now: options.now })
  const server = createServer((req,res) => transport.handle(req,res,req.headers['x-test-client'] || req.socket.remoteAddress))
  await new Promise(ok => server.listen(0,'127.0.0.1',ok))
  t.after(async () => { transport.close(); server.closeAllConnections(); await new Promise(ok => server.close(ok)) })
  const send = (payload=value(), headers={}, method='POST') => new Promise((ok,no) => {
    const body = typeof payload === 'string' ? payload : JSON.stringify(payload)
    const req=httpRequest({host:'127.0.0.1',port:server.address().port,path:'/_brain/sentry',method,agent:false,headers:{host:'example.com',origin:'https://example.com','content-type':'application/json',...headers}},res=>{res.body='';res.on('data',chunk=>{res.body+=chunk;assert.ok(res.body.length<=1024)});res.on('end',()=>ok(res))})
    req.on('error',no);req.end(body)
  })
  return { send, forwarded, transport }
}
test('tunnel reconstructs a single private envelope and forwards no inbound headers or values', async t => {
  const s = await tunnel(t)
  assert.equal((await s.send(value(),{cookie:'private_cookie','authorization':'private_auth','x-forwarded-for':'192.0.2.1'})).statusCode,202)
  assert.equal(s.forwarded.length,1)
  const f=s.forwarded[0], lines=f.body.trim().split('\n'), event=JSON.parse(lines[2])
  assert.equal(f.url.href,'https://o1.ingest.sentry.io/api/123/envelope/'); assert.equal(f.options.rejectUnauthorized,true)
  assert.deepEqual(Object.keys(f.options.headers).sort(),['content-length','content-type','x-sentry-auth'])
  assert.equal(JSON.parse(lines[1]).type,'event'); assert.equal(JSON.parse(lines[1]).length,Buffer.byteLength(lines[2]))
  assert.equal(event.debug_meta.images[0].code_file,asset); assert.equal(event.exception.values[0].stacktrace.frames[0].abs_path,asset)
  assert.doesNotMatch(f.body,/private_cookie|private_auth|192\.0\.2|request|user|cookie|dsn/)
  assert.equal((await s.send({...value(),url:'https://other.example.com'})).statusCode,400)
  assert.equal((await s.send(value(),{origin:'https://other.example.com'})).statusCode,403)
  assert.equal((await s.send(value(),{origin:'http://example.com'})).statusCode,403)
  assert.equal((await s.send(value(),{origin:'https://other.example.com','x-forwarded-host':'other.example.com'})).statusCode,403)
  assert.equal((await s.send('x'.repeat(32769))).statusCode,413)
  assert.equal((await s.send(value(),{},'GET')).statusCode,405)
  assert.equal(s.forwarded.length,1)
})
test('upstream 200 rate limits and 429 Retry-After apply bounded cooldowns; nothing retries', async t => {
  let time=100000
  const s=await tunnel(t,{now:()=>time,response:incoming=>{incoming.headers={'x-sentry-rate-limits':'120:error:organization'}; incoming.statusCode=200}})
  assert.equal((await s.send()).statusCode,202);const limited=await s.send();assert.equal(limited.statusCode,429);assert.equal(limited.headers['retry-after'],'120')
  assert.equal(s.forwarded.length,1);time+=120001;assert.equal((await s.send()).statusCode,202)
  const r=await tunnel(t,{response:incoming=>{incoming.statusCode=429;incoming.headers={'retry-after':'999999999'}}})
  await r.send();const capped=await r.send();assert.equal(capped.statusCode,429);assert.equal(capped.headers['retry-after'],'3600');assert.equal(r.forwarded.length,1)
})
test('four in-flight requests have no queue; absolute upstream timeout releases the owned slots', {timeout:6000},async t=>{
  const s=await tunnel(t,{stall:true})
  const requests=Array.from({length:4},(_,i)=>s.send(value(),{'x-test-client':'client-'+i}))
  while(s.forwarded.length<4)await tick()
  assert.equal((await s.send(value(),{'x-test-client':'fifth'})).statusCode,429)
  assert.ok((await Promise.all(requests)).every(res=>res.statusCode===202));assert.equal(s.forwarded.length,4)
})
function errorResponse(response,status,code) {
  assert.equal(response.statusCode,status)
  assert.match(response.headers['content-type'],/^application\/json; charset=utf-8$/i)
  assert.deepEqual(JSON.parse(response.body),{error:{code,message:STATUS_CODES[status]}})
  assert.doesNotMatch(response.body,/private|synthetic|example\.com|sentry\.io|cookie|stack|dsn/i)
}
test('tunnel HTTP errors use fixed JSON envelopes, Allow and Retry-After; JSON media accepts UTF-8 charset case-insensitively',async t=>{
  const s=await tunnel(t)
  assert.equal((await s.send(value(),{'content-type':'Application/JSON; Charset="UTF-8"'})).statusCode,202)
  const method=await s.send('private method body',{},'GET');errorResponse(method,405,'SENTRY_METHOD_NOT_ALLOWED');assert.equal(method.headers.allow,'POST')
  const head=await s.send('',{},'HEAD');assert.equal(head.statusCode,405);assert.equal(head.headers.allow,'POST');assert.equal(head.body,'')
  errorResponse(await s.send('private media',{'content-type':'text/plain'}),415,'SENTRY_UNSUPPORTED_MEDIA')
  errorResponse(await s.send('private compressed',{'content-encoding':'gzip'}),413,'SENTRY_BODY_TOO_LARGE')
  errorResponse(await s.send(value(),{origin:'https://other.example.com'}),403,'SENTRY_ORIGIN_REFUSED')
  errorResponse(await s.send('{private input'),400,'SENTRY_INVALID_EVENT')
  const r=await tunnel(t,{response:incoming=>{incoming.statusCode=429;incoming.headers={'retry-after':'60'}}})
  await r.send();const limited=await r.send();errorResponse(limited,429,'SENTRY_RATE_LIMITED');assert.equal(limited.headers['retry-after'],'60')
})
test('timed-out native DNS retains four leases across successive waves; late callbacks and close cannot connect', {timeout:10000},async t=>{
  const original=dns.lookup,held=[];let connected=0
  const socket=socketServer(connection=>{connected++;connection.destroy()})
  await new Promise(ok=>socket.listen(0,'127.0.0.1',ok))
  dns.lookup=(host,options,callback)=>host==='report.example.com'?held.push({options,callback}):original(host,options,callback)
  const complete=entry=>entry.options?.all?entry.callback(null,[{address:'127.0.0.1',family:4}]):entry.callback(null,'127.0.0.1',4)
  t.after(async()=>{dns.lookup=original;for(const entry of held.splice(0))entry.callback(Object.assign(Error('Synthetic DNS cleanup'),{code:'ENOTFOUND'}));await new Promise(ok=>socket.close(ok))})
  const s=await tunnel(t,{request:(_url,settings,callback)=>httpsRequest(new URL('https://report.example.com:'+socket.address().port+'/api/123/envelope/'),settings,callback)})
  const wave=Array.from({length:4},(_,i)=>s.send(value(),{'x-test-client':'first-'+i}))
  assert.ok((await Promise.all(wave)).every(response=>response.statusCode===202))
  assert.equal(held.length,4)
  const second=await Promise.all(Array.from({length:4},(_,i)=>s.send(value(),{'x-test-client':'second-'+i})))
  assert.ok(second.every(response=>response.statusCode===429),'expired sockets must not admit a new wave while native DNS is unresolved')
  assert.equal(held.length,4,'at most four native lookups, including timed-out work')
  complete(held.shift());await tick();assert.equal(connected,0,'a late DNS success cannot open a connection')
  const next=s.send(value(),{'x-test-client':'released-slot'})
  const deadline=Date.now()+1000;while(held.length<4&&Date.now()<deadline)await tick()
  assert.equal(held.length,4,'one completed DNS callback releases exactly one admission lease')
  s.transport.close();assert.equal((await next).statusCode,503)
  for(const entry of held.splice(0))complete(entry)
  await new Promise(ok=>setTimeout(ok,20));assert.equal(connected,0,'close suppresses every late DNS connection')
  errorResponse(await s.send(value(),{'x-test-client':'closed'}),503,'SENTRY_UNAVAILABLE')
})

test('close cancels an admitted partial body and cannot start forwarding after the body completes', async () => {
  let forwarded=0,destroyed=0,status
  const transport=createSentryTunnel({dsn,assets,publicOrigin:'https://example.com',request(){forwarded++;throw Error('unexpected request')}})
  const req=new EventEmitter();req.method='POST';req.headers={host:'example.com',origin:'https://example.com','content-type':'application/json'};req.socket={};req.destroy=()=>{destroyed++};req.resume=()=>{}
  const res={destroyed:false,writableEnded:false,writeHead(code){status=code},end(){this.writableEnded=true}}
  const handling=transport.handle(req,res,'synthetic')
  req.emit('data',Buffer.from('{'))
  transport.close();transport.close()
  req.emit('data',Buffer.from(JSON.stringify(value()).slice(1)));req.emit('end')
  await handling
  assert.equal(status,503);assert.equal(forwarded,0);assert.equal(destroyed,1)
  for(const event of ['data','end','error','aborted'])assert.equal(req.listenerCount(event),0,'body listener must be removed: '+event)
})
