import { test } from 'node:test'
import assert from 'node:assert/strict'
import { readFileSync, writeFileSync, mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { spawnSync } from 'node:child_process'
const runner=readFileSync(new URL('../../crates/brain-cli/src/templates/job-runner.mjs',import.meta.url),'utf8')
const jobId='11111111-2222-4333-8444-555555555555'
function fixture(run) {
  const root=mkdtempSync(join(tmpdir(),'brain-job-runner-'))
  try {
    const module=join(root,'handler.mjs'), source='server/jobs/notify/handler.jsbrain'
    writeFileSync(join(root,'trace-index.json'),JSON.stringify({schemaVersion:1,modules:{'handler.mjs':source}}))
    const execute=(input=JSON.stringify({jobId,attempt:1,payload:{message:'Hello'}}))=>spawnSync(process.execPath,['--enable-source-maps','--input-type=module','--eval',runner,module],{input,encoding:'utf8',timeout:20000,maxBuffer:256*1024})
    run({root,module,source,execute})
  } finally {rmSync(root,{recursive:true,force:true})}
}
function trace(result) {
  assert.equal(result.status,1)
  assert.equal(result.stdout,'')
  assert.ok(result.stderr.startsWith('JOB_TRACE '))
  assert.ok(Buffer.byteLength(result.stderr)<32784)
  const record=JSON.parse(result.stderr.slice('JOB_TRACE '.length))
  assert.equal(record.schemaVersion,1)
  assert.equal(record.jobId,jobId)
  assert.equal(record.event,'job-trace')
  assert.ok(!result.stderr.includes('private-payload'))
  assert.ok(!result.stderr.includes('private-environment'))
  assert.ok(!result.stderr.includes('file://'))
  assert.ok(!result.stderr.includes('node:internal'))
  return record
}
const vlq=value=>{
  let n=value<0?(-value<<1)|1:value<<1,out=''
  do{let d=n&31;n>>>=5;if(n)d|=32;out+='ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/'[d]}while(n)
  return out
}
function mapped(source,lines,originals) {
  let previous=0
  const mappings=originals.map(line=>{const n=line-1,part='AA'+vlq(n-previous)+'A';previous=n;return part}).join(';')
  const map={version:3,sources:[source],sourcesContent:[''],names:[],mappings}
  return lines.join('\n')+'\n//# sourceMappingURL=data:application/json;base64,'+Buffer.from(JSON.stringify(map)).toString('base64')+'\n'
}

test('job runner supplies a frozen environment snapshot and reports structural failures without values',()=>fixture(({module,source,execute})=>{
  writeFileSync(module,"export default async ctx=>{if(!Object.isFrozen(ctx.env)||ctx.payload.message!=='Hello'||!ctx.jobId||ctx.attempt!==1)throw Error('Invalid context')}\n")
  const success=execute()
  assert.equal(success.status,0,success.stderr)
  assert.equal(success.stdout,'');assert.equal(success.stderr,'')
  writeFileSync(module,"export default async ()=>{throw Error('private-payload must not be logged')}\n")
  const record=trace(execute())
  assert.equal(record.location.file,source)
  assert.equal(record.location.line,null,'no source map means no invented app line')
  assert.equal(record.location.precision,'module-only')
  assert.equal(record.location.generated.file,'_server/handler.mjs')
  assert.equal(record.location.generated.line,1)
}))

test('job source maps preserve actual app lines and bounded cyclic causes without original columns',()=>fixture(({module,source,execute})=>{
  writeFileSync(module,mapped(source,[
    'export default async function handle() {',
    "  const cause = new Error('private-payload cause')",
    '  cause.cause = cause',
    "  throw new Error('private-environment outer', { cause })",
    '}',
  ],[10,17,18,37,38]))
  const record=trace(execute())
  assert.equal(record.location.file,source)
  assert.equal(record.location.line,37)
  assert.equal(record.location.column,null)
  assert.equal(record.location.precision,'source-line')
  assert.equal(record.location.generated.line,4)
  assert.ok(record.location.generated.column>0)
  assert.equal(record.causes.length,1)
  assert.equal(record.causes[0].location.line,17)
  assert.equal(record.truncated.causes,true)
  assert.ok(record.frames.length+record.causes.reduce((n,c)=>n+c.frames.length,0)<=24)
}))

test('job trace frames and causes obey total caps and ignore arbitrary formatted stacks',()=>fixture(({module,execute})=>{
  writeFileSync(module,"function fail(n){if(n)return fail(n-1);let e=Error('private-payload');for(let i=0;i<20;i++)e=Error('private-environment',{cause:e});throw e} export default async()=>fail(80)\n")
  const record=trace(execute())
  assert.equal(record.causes.length,6)
  assert.ok(record.truncated.frames)
  assert.ok(record.truncated.causes)
  assert.ok(record.frames.length+record.causes.reduce((n,c)=>n+c.frames.length,0)<=24)
  writeFileSync(module,"export default async()=>{const e=Error('private-payload');e.stack='private-environment';throw e}\n")
  const missing=trace(execute())
  assert.deepEqual(missing.frames,[])
  assert.equal(missing.location.line,null)
  assert.equal(missing.truncated.stackUnavailable,true)
}))

test('a mismatched source-map path cannot replace the private app index or leak query strings',()=>fixture(({module,source,execute})=>{
  writeFileSync(module,mapped('private-payload?token=private-environment',["export default async()=>{throw Error('private-payload')}"],[900]))
  const record=trace(execute())
  assert.equal(record.location.file,source)
  assert.equal(record.location.line,null)
  assert.equal(record.location.precision,'module-only')
  assert.ok(!JSON.stringify(record).includes('token='))
}))

test('job stdin decodes UTF-8 incrementally without retaining values in reports',()=>fixture(({module,execute})=>{
  writeFileSync(module,"export default async ctx=>{if(ctx.payload.message!=='caf\\u00e9\\u{1f600}'.repeat(6000))throw Error('private-payload')}\n")
  const result=execute(JSON.stringify({jobId,attempt:1,payload:{message:'caf\u00e9\u{1f600}'.repeat(6000)}}))
  assert.equal(result.status,0,result.stderr)
  assert.equal(result.stdout,'');assert.equal(result.stderr,'')
}))
