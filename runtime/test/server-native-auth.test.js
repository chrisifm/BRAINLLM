import { test, after } from 'node:test'
import assert from 'node:assert/strict'
import { createServer as http } from 'node:http'
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { createServer, sessionFrom } from '../server/server.mjs'
const user = { id: '00000000-0000-4000-8000-000000000001', email: 'candidate@example.test', role: 'candidate', verified: true }
const dist = mkdtempSync(join(tmpdir(), 'brain-native-ssr-')); mkdirSync(join(dist, '_server'))
writeFileSync(join(dist, '_server/native.mjs'), `export function who(ctx) { return { user:ctx.user, sessionUser:ctx.session.user } } export function set(ctx) { ctx.session.set({role:'admin'});return 'changed' } export function logout(ctx) { ctx.session.clear();return 'signed out' }`)
writeFileSync(join(dist, '_server/guard.mjs'), `export default ctx => ctx.user?.role==='admin' ? {redirect:'/permitted'} : false`)
writeFileSync(join(dist, '_server/read.mjs'), `export function load(ctx) { return {redirect:ctx.user ? '/signed-in' : '/anonymous'} }`)
writeFileSync(join(dist, '_server/manifest.json'), JSON.stringify({ routes: [{path:'/guarded',guards:['_server/guard.mjs']},{path:'/read',server:'_server/read.mjs'}], rpc:{native:'_server/native.mjs'} }))
let mode = 'normal', logoutAllowed = true
const seen = [], pending = new Set()
const backend = http((req, res) => {
  seen.push({ path:req.url, method:req.method, headers:req.headers })
  if (mode === 'drip') { res.writeHead(200, {'content-type':'application/json'});res.write('{');const timer=setInterval(()=>res.write(' '),100);timer.unref();res.once('close',()=>clearInterval(timer));return }
  if (mode === 'hold') { pending.add(res); res.once('close', () => pending.delete(res)); return }
  if (mode === 'redirect') { res.writeHead(302,{location:'https://must-not-follow.example'}); return res.end() }
  if (mode === 'oversize') { res.writeHead(200,{'content-type':'application/json'});return res.end('x'.repeat(17000)) }
  if (mode === 'malformed') { res.writeHead(200,{'content-type':'application/json'});return res.end('{"privateStack":"do-not-show-this"}') }
  if (req.url === '/_brain/auth/logout') {
    res.writeHead(logoutAllowed ? 200 : 503, {'content-type':'application/json','set-cookie':['__Host-brain-access=; Path=/; Secure; HttpOnly; SameSite=Lax; Max-Age=0','__Host-brain-refresh=; Path=/; Secure; HttpOnly; SameSite=Lax; Max-Age=0','unrelated=do-not-relay; Path=/']})
    return res.end(JSON.stringify({ok:logoutAllowed}))
  }
  const authorized = /__Host-brain(?:-access)?=native/.test(req.headers.cookie || '')
  res.writeHead(authorized ? 200 : 401, {'content-type':'application/json','cache-control':'no-store'})
  res.end(JSON.stringify(authorized ? {user:{...user,password_hash:'private-server-value',stack:'private-server-stack'}} : {error:'private denial detail'}))
})
await new Promise(resolve => backend.listen(0,'127.0.0.1',resolve))
const backendURL = `http://127.0.0.1:${backend.address().port}`
const frontend = createServer({dist,backendURL,secret:'synthetic-legacy-cookie-secret',log(){}})
await new Promise(resolve => frontend.listen(0,'127.0.0.1',resolve))
const base = `http://127.0.0.1:${frontend.address().port}`
after(async () => {
  for(const res of pending) res.destroy()
  await new Promise(resolve => {frontend.close(resolve);frontend.closeAllConnections()})
  await new Promise(resolve => {backend.close(resolve);backend.closeAllConnections()})
  rmSync(dist,{recursive:true,force:true})
})
const rpc = (fn, cookie = '__Host-brain-access=native', extra = {}) => fetch(base+'/_r/native/'+fn,{method:'POST',headers:{origin:base,'x-brain-rpc':'1','content-type':'application/json',cookie,...extra},body:'[]',redirect:'manual'})
test('native SSR authority sanitizes user once/request and forwards only required native cookies and request reference', async () => {
  seen.length=0
  const rid='00000000-0000-4000-8000-000000000002'
  const response=await rpc('who','private_cookie=never-forward; __Host-brain-access=native; __Host-brain-refresh=secret-refresh; __Host-session=never-forward',{authorization:'Bearer never-forward','x-request-id':rid})
  assert.equal(response.status,200);assert.deepEqual(await response.json(),{user,sessionUser:user})
  assert.equal(seen.length,1);assert.equal(seen[0].path,'/_brain/auth/me');assert.equal(seen[0].headers.cookie,'__Host-brain-access=native');assert.equal(seen[0].headers.authorization,undefined);assert.equal(seen[0].headers['x-request-id'],rid)
  const read=await fetch(base+'/read',{headers:{cookie:'__Host-brain-access=native'},redirect:'manual'})
  assert.equal(read.headers.get('location'),'/signed-in');assert.equal(seen.length,2)
})
test('native mode never accepts a valid legacy signed session and cannot establish a legacy backdoor', async () => {
  const old=sessionFrom({},'synthetic-legacy-cookie-secret');old.session.set({role:'admin'})
  const legacy=old.session.headers()['set-cookie'][0].split(';')[0]
  const who=await rpc('who',legacy);assert.deepEqual(await who.json(),{user:null,sessionUser:null})
  const guarded=await fetch(base+'/guarded',{headers:{cookie:legacy},redirect:'manual'});assert.equal(guarded.status,403)
  const set=await rpc('set');assert.equal(set.status,409);assert.equal(set.headers.get('set-cookie'),null)
})
test('unawaited native clear waits for revocation and relays only native clear cookies', async () => {
  seen.length=0
  const response=await rpc('logout','__Host-brain-access=native; __Host-brain-refresh=refresh-token; other=private')
  assert.equal(response.status,200);assert.equal(await response.json(),'signed out')
  const clear=seen.find(request=>request.path==='/_brain/auth/logout');assert.equal(clear.method,'POST');assert.equal(clear.headers.origin,base);assert.equal(clear.headers.cookie,'__Host-brain-access=native; __Host-brain-refresh=refresh-token')
  assert.equal(response.headers.get('clear-site-data'),'"storage"');assert.ok(response.headers.getSetCookie().every(value=>value.startsWith('__Host-brain-')))
  logoutAllowed=false
  try {const failed=await rpc('logout');assert.equal(failed.status,503);assert.equal(failed.headers.get('set-cookie'),null)} finally {logoutAllowed=true}
})
test('native unavailable, redirected or malformed responses fail closed without public diagnostics', async () => {
  for(const failure of ['redirect','oversize','malformed']) {
    mode=failure
    const response=await rpc('who');assert.equal(response.status,503)
    const body=await response.text();assert.ok(!body.includes('private') && !body.includes('stack') && !body.includes(backendURL))
  }
  mode='normal'
})
test('loopback target is explicit and bounded capacity has no pending work queue', async () => {
  for(const url of ['https://public.example','http://127.0.0.1/private','http://user:password@127.0.0.1','http://127.0.0.1/?target=other','http://127.0.0.1/#other']) assert.throws(()=>createServer({dist,backendURL:url}),/loopback/)
  mode='hold';seen.length=0
  const requests=Array.from({length:9},()=>rpc('who'))
  try {
    const rejected=await Promise.race(requests);assert.equal(rejected.status,503);
    const deadline=Date.now()+250;while(pending.size<8 && Date.now()<deadline)await new Promise(resolve=>setTimeout(resolve,1));assert.equal(pending.size,8)
    mode='normal';for(const res of pending) {res.writeHead(401,{'content-type':'application/json'});res.end('{}')}
    const responses=await Promise.all(requests);assert.equal(responses.filter(response=>response.status===503).length,1)
  } finally {mode='normal';for(const res of pending)res.destroy()}
})

test('native request has an absolute deadline even while a backend drips response bytes', async () => {
  mode='drip';const started=performance.now()
  try {const response=await rpc('who');assert.equal(response.status,503);assert.ok(performance.now()-started<3000)}finally{mode='normal'}
})

test('native SSR preserves server-verified custom roles without treating them as an administrator',async()=>{
  const previous=user.role
  try {
    user.role='editor'
    const response=await rpc('who')
    assert.equal(response.status,200);assert.deepEqual(await response.json(),{user,sessionUser:user})
    const guarded=await fetch(base+'/guarded',{headers:{cookie:'__Host-brain-access=native'},redirect:'manual'})
    assert.equal(guarded.status,403)
  } finally {user.role=previous}
})
