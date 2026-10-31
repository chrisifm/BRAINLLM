// Job handlers get ctx.backend (docs/backend/job-backend.md): the job runner template talks to the native backend
// with a scoped project API key, using the same list/get/create/update/patch/action/transition/remove/slugRedirect calls as load().
import {test, after} from 'node:test'
import assert from 'node:assert/strict'
import {createServer} from 'node:http'
import {spawnSync, spawn} from 'node:child_process'
import {mkdtempSync, writeFileSync, readFileSync, rmSync} from 'node:fs'
import {tmpdir} from 'node:os'
import {join} from 'node:path'
import {fileURLToPath} from 'node:url'
const runner=readFileSync(fileURLToPath(new URL('../../crates/brain-cli/src/templates/job-runner.mjs',import.meta.url)),'utf8')+'\n'+readFileSync(new URL('../../crates/brain-serve/src/key-backend.js',import.meta.url),'utf8')
const key='bk_live_0123456789ab_'+'c'.repeat(64)
const seen=[]
const fileId='00000000-0000-4000-8000-00000000000f'
const backend=createServer((req,res)=>{
  const chunks=[]
  req.on('data',chunk=>chunks.push(chunk))
  req.on('end',()=>{
    seen.push({method:req.method,path:req.url,headers:req.headers,body:Buffer.concat(chunks).toString()})
    const json=(status,value)=>{res.writeHead(status,{'content-type':'application/json'});res.end(JSON.stringify(value))}
    if(req.url==='/api/articles/redirects/old')return json(200,{slug:'new'})
    if(req.url.startsWith('/api/articles/redirects/'))return json(404,{schemaVersion:1,error:{code:'NOT_FOUND'}})
    if(req.url.startsWith('/api/orders?status=bad'))return json(422,{schemaVersion:1,error:{code:'INVALID_INPUT',fields:{status:'Expected one of: pending, paid'}}})
    if(req.url.startsWith('/api/secret'))return json(403,{schemaVersion:1,error:{code:'API_KEY_SCOPE_DENIED'}})
    if(req.url==='/_brain/files/reports/'+fileId&&req.method==='GET'){res.writeHead(200,{'content-type':'application/pdf','etag':'"sum"'});return res.end('binary')}
    if(req.method==='DELETE'){res.writeHead(204);return res.end()}
    json(200,{items:[],total:0,id:'00000000-0000-4000-8000-000000000005',updated_at:'2026-10-31T10:05:00.000Z'})
  })
})
await new Promise(resolve=>backend.listen(0,'127.0.0.1',resolve))
const url=`http://127.0.0.1:${backend.address().port}`
const dir=mkdtempSync(join(tmpdir(),'brain-job-backend-'))
after(async()=>{rmSync(dir,{recursive:true,force:true});await new Promise(resolve=>{backend.close(resolve);backend.closeAllConnections()})})
function run(body,env){
  const file=join(dir,'handler.mjs'),out=join(dir,'out.json')
  writeFileSync(file,`import {writeFileSync} from 'node:fs'\nexport default async function handle(ctx){const r=await (async()=>{${body}})();writeFileSync(${JSON.stringify(out)},JSON.stringify(r===undefined?null:r))}\n`)
  rmSync(out,{force:true})
  return new Promise(resolve=>{
    const child=spawn(process.execPath,['--input-type=module','--eval',runner,file],{env:{PATH:process.env.PATH,...env},stdio:['pipe','ignore','pipe']})
    let stderr='';child.stderr.on('data',d=>stderr+=d)
    child.stdin.end(JSON.stringify({jobId:'00000000-0000-4000-8000-000000000009',attempt:1,payload:{order:'A-17'}}))
    child.on('exit',code=>{let value;try{value=JSON.parse(readFileSync(out,'utf8'))}catch{};resolve({code,value,stderr})})
  })
}
const id='00000000-0000-4000-8000-000000000005'
test('a job reads and writes through ctx.backend with its API key and never forwards it elsewhere',async()=>{
  const r=await run(`
    const orders=ctx.backend.resource('orders')
    await orders.list({filter:{status:{in:['pending','paid']},total:{gte:'10.00'}},sort:['-created_at'],limit:5})
    await orders.get('${id}',{include:['customer']})
    await orders.create({total:'12.00'})
    await orders.update('${id}',{total:'13.00'})
    await orders.patch('${id}',{status:'paid'})
    await orders.action('${id}','reserve',{quantity:2})
    await orders.transition('${id}','ship')
    await orders.remove('${id}')
    return {slug:await ctx.backend.slugRedirect('articles','old'),missing:await ctx.backend.resource('articles').slugRedirect('nope'),keyInEnv:'BRAIN_JOBS_API_KEY' in ctx.env}
  `,{BRAIN_BACKEND_URL:url,BRAIN_JOBS_API_KEY:key})
  assert.equal(r.code,0,r.stderr)
  assert.deepEqual(r.value,{slug:'new',missing:null,keyInEnv:false})
  const calls=seen.slice(-10)
  assert.deepEqual(calls.map(c=>c.method+' '+c.path),[
    'GET /api/orders?status%5Bin%5D=pending%2Cpaid&total%5Bgte%5D=10.00&limit=5&sort=-created_at',
    `GET /api/orders/${id}?include=customer`,'POST /api/orders',`PUT /api/orders/${id}`,`PATCH /api/orders/${id}`,`POST /api/orders/${id}/actions/reserve`,`POST /api/orders/${id}/transitions/ship`,`DELETE /api/orders/${id}`,
    'GET /api/articles/redirects/old','GET /api/articles/redirects/nope'])
  assert.ok(calls.every(c=>c.headers['x-api-key']===key&&c.headers.cookie===undefined&&c.headers.origin===undefined))
  assert.equal(calls[4].body,'{"status":"paid"}')
})
test('backend failures reach the handler as status and field errors, never as response text',async()=>{
  const r=await run(`
    const out={}
    try{await ctx.backend.resource('orders').list({filter:{status:'bad'}})}catch(e){out.invalid=[e.status,e.errors]}
    try{await ctx.backend.resource('secret').list()}catch(e){out.denied=[e.status,e.code,e.message.includes('SCOPE')]}
    try{ctx.backend.resource('../auth')}catch(e){out.name=e.status}
    try{ctx.backend.resource('orders').get('nope')}catch(e){out.malformed=e.status}
    return out
  `,{BRAIN_BACKEND_URL:url,BRAIN_JOBS_API_KEY:key})
  assert.equal(r.code,0,r.stderr)
  assert.deepEqual(r.value,{invalid:[422,{status:'Expected one of: pending, paid'}],denied:[403,'API_KEY_SCOPE_DENIED',false],name:400,malformed:404})
})
test('without a backend URL and key, ctx.backend explains what to configure',async()=>{
  const before=seen.length
  const r=await run(`try{ctx.backend.resource('orders')}catch(e){return e.message}`,{BRAIN_BACKEND_URL:url})
  assert.equal(r.code,0,r.stderr)
  assert.match(r.value,/BRAIN_JOBS_API_KEY/)
  const remote=await run(`try{ctx.backend.resource('orders')}catch(e){return e.message}`,{BRAIN_BACKEND_URL:'http://192.0.2.10:5182',BRAIN_JOBS_API_KEY:key})
  assert.match(remote.value,/loopback/)
  assert.equal(seen.length,before)
  assert.equal(spawnSync(process.execPath,['--version']).status,0)
})
test('a system queue uses only the system key and never falls back to the ordinary one',async()=>{
  const system='bk_live_aaaaaaaaaaaa_'+'d'.repeat(64)
  const r=await run(`await ctx.backend.resource('orders').list();return ['BRAIN_JOBS_SYSTEM_KEY','BRAIN_JOBS_API_KEY','BRAIN_JOB_RUN_AS'].map(n=>(n in ctx.env)||(n in process.env))`,{BRAIN_BACKEND_URL:url,BRAIN_JOBS_API_KEY:key,BRAIN_JOBS_SYSTEM_KEY:system,BRAIN_JOB_RUN_AS:'system'})
  assert.equal(r.code,0,r.stderr)
  assert.deepEqual(r.value,[false,false,false])
  assert.equal(seen.at(-1).headers['x-api-key'],system)
  const missing=await run(`try{ctx.backend.resource('orders')}catch(e){return e.message}`,{BRAIN_BACKEND_URL:url,BRAIN_JOBS_API_KEY:key,BRAIN_JOB_RUN_AS:'system'})
  assert.match(missing.value,/BRAIN_JOBS_SYSTEM_KEY/)
})
test('a job reads and writes a declared bucket through ctx.backend.files',async()=>{
  const r=await run(`
    const reports=ctx.backend.files('reports')
    await reports.list({limit:5,record:'${id}'})
    await reports.meta('${fileId}')
    await reports.upload({filename:'a.pdf',contentType:'application/pdf',data:Buffer.from('%PDF-1.7'),record:'${id}'})
    const file=await reports.read('${fileId}')
    await reports.remove('${fileId}')
    const out={text:file.bytes.toString(),type:file.contentType}
    try{ctx.backend.files('../keys')}catch(e){out.name=e.status}
    try{await reports.meta('nope')}catch(e){out.id=e.status}
    return out
  `,{BRAIN_BACKEND_URL:url,BRAIN_JOBS_API_KEY:key})
  assert.equal(r.code,0,r.stderr)
  assert.deepEqual(r.value,{text:'binary',type:'application/pdf',name:400,id:404})
  const calls=seen.slice(-5)
  assert.deepEqual(calls.map(c=>c.method+' '+c.path),[`GET /_brain/files/reports?limit=5&record=${id}`,`GET /_brain/files/reports/${fileId}/meta`,`POST /_brain/files/reports?record=${id}`,`GET /_brain/files/reports/${fileId}`,`DELETE /_brain/files/reports/${fileId}`])
  assert.ok(calls.every(c=>c.headers['x-api-key']===key))
  assert.match(calls[2].headers['content-type'],/^multipart\/form-data/)
  assert.match(calls[2].body,/name="file"; filename="a\.pdf"/)
})
