import assert from 'node:assert/strict'
import {join} from 'node:path'
import {pathToFileURL} from 'node:url'
const [app,runtime]=process.argv.slice(2),url=p=>pathToFileURL(p).href
const {install,Event}=await import(url(join(runtime,'shim/dom.js'))),doc=install()
let destination=null,requests=[],failure=false,sets=0,backendCalls=0
const user={id:'00000000-0000-4000-8000-000000000001',email:'native@example.test',role:'user',verified:true}
globalThis.location={href:'https://app.example.test/login',origin:'https://app.example.test',pathname:'/login',search:'',hash:'',assign:u=>{destination=u},replace:u=>{destination=u}}
globalThis.localStorage={getItem(){throw Error('Authentication must not read persistent storage')},setItem(){throw Error('Authentication must not write persistent storage')}}
const {mount}=await import(url(join(runtime,'brain.js')))
const Login=(await import(url(join(app,'views/login.brain.mjs')))).default
const server=await import(url(join(app,'views/login.server.mjs')))
const auth=(await import(url(join(app,'server/auth.jsbrain.mjs')))).default
const session=(await import(url(join(app,'channels/session.jsbrain.mjs')))).default
const ctx={backend:{async login(credentials){backendCalls++;assert.deepEqual(Object.keys(credentials).sort(),['email','password']);if(failure)throw {status:503};return {user}}},env:{},query:{},session:{set(){sets++}}}
globalThis.fetch=async(input,options)=>{
 const path=new URL(input,location.href).pathname;requests.push(path)
 assert(path.startsWith('/_r/')&&path.endsWith('/login'),'The client uses one small RPC transport; account tokens remain server-side')
 assert.equal(options.method,'POST');assert.equal(options.headers['x-brain-rpc'],'1')
 try {return new Response(JSON.stringify(await server.login(ctx,...JSON.parse(options.body))),{headers:{'content-type':'application/json'}})}
 catch {return new Response('{"error":"Sign-in is temporarily unavailable"}',{status:503,headers:{'content-type':'application/json'}})}
}
const all=(host,tag)=>{const found=[];const walk=n=>{for(const c of n.childNodes){if((c.tagName||'').toLowerCase()===tag)found.push(c);walk(c)}};walk(host);return found}
const flush=()=>new Promise(r=>setTimeout(r,0))
process.env.NODE_ENV='production';assert.equal(auth.configured(),false);assert.equal(auth.check('demo@example.test','synthetic-login-demo-password'),null)
process.env.NODE_ENV='development';assert.equal(auth.configured(),true)
assert.deepEqual(await server.load(ctx),{configured:true,native:true})
assert.deepEqual((await server.login(ctx,{email:user.email,password:'synthetic-native-password'})).user,user);assert.equal(sets,0)
process.env.BRAIN_BACKEND_URL='http://127.0.0.1:5182';assert.equal(auth.configured(),false)
await assert.rejects(server.login({...ctx,backend:undefined},{email:'demo@example.test',password:'synthetic-login-demo-password'}),e=>e.status===503)
process.env.BRAIN_BACKEND_URL=''
assert.equal((await server.login({...ctx,backend:undefined,query:{next:'//evil.example'}},{email:'demo@example.test',password:'synthetic-login-demo-password'})).redirect,'/account');assert.equal(sets,1)
process.env.NODE_ENV='production';await assert.rejects(server.login({...ctx,backend:undefined},{email:'demo@example.test',password:'synthetic-login-demo-password'}),e=>!!e.errors?.password)
process.env.NODE_ENV='development'
// Progressive native forms use the same server authority and work without JavaScript.
const {callForm}=await import(url(join(runtime,'server/server.mjs'))),rules=await import(url(join(runtime,'brain-form.js')))
assert(server.$forms?.login,'The compiler must preserve the native progressive form contract')
const progressive=await callForm({module:server,raw:'_brain=fixture%2Flogin&email=native%40example.test&password=synthetic-native-password',ctx:{...ctx,query:{next:'/account?tab=last%2030%20days'}},rules})
assert.equal(progressive.status,303);assert.equal(progressive.location,'/account?tab=last%2030%20days')
let host=doc.createElement('div');doc.body.appendChild(host);globalThis.__brainSSR=true
let stop=mount(Login,host,{configured:true,native:true});assert(!all(host,'button')[0].hasAttribute('disabled'));stop();host.remove();globalThis.__brainSSR=false
for(const [query,expected]of [['','/account'],['?next=%2Fposts','/posts'],['?next=%2F%2Fevil.example','/account'],['?next=%2F%0a%2Fevil.example','/account']]){
 failure=false;requests=[];destination=null;location.search=query
 host=doc.createElement('div');doc.body.appendChild(host);stop=mount(Login,host,{configured:true,native:true})
 const [email,password]=all(host,'input');email.value=user.email;email.dispatchEvent(new Event('input',{bubbles:true}));password.value='synthetic-native-password';password.dispatchEvent(new Event('input',{bubbles:true}))
 all(host,'form')[0].dispatchEvent(new Event('submit',{bubbles:true,cancelable:true}))
 for(let i=0;i<100&&destination===null;i++)await flush()
 assert.equal(destination,expected);assert.equal(requests.length,1);assert.deepEqual(session.user,user)
 stop();host.remove()
}
failure=true;requests=[];destination=null;location.search='';const before=backendCalls
host=doc.createElement('div');doc.body.appendChild(host);stop=mount(Login,host,{configured:true,native:true})
const [email,password]=all(host,'input');email.value=user.email;email.dispatchEvent(new Event('input',{bubbles:true}));password.value='synthetic-native-password';password.dispatchEvent(new Event('input',{bubbles:true}))
all(host,'form')[0].dispatchEvent(new Event('submit',{bubbles:true,cancelable:true}));for(let i=0;i<30;i++)await flush()
assert.equal(destination,null);assert.equal(requests.length,1);assert.equal(backendCalls,before+1);assert.equal(sets,1)
assert(host.textContent.includes('Something went wrong'));stop();host.remove()
