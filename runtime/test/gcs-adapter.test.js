import { test } from 'node:test'
import assert from 'node:assert/strict'
import { mkdtemp, mkdir, writeFile, readFile, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { createHash } from 'node:crypto'
import { Readable, Writable } from 'node:stream'
import { execute, validate } from '../server/templates/gcs-adapter.mjs'
const owner='00000000-0000-4000-8000-000000000001', id='00000000-0000-4000-8000-000000000002'
function mock(privateBucket=true) {
  const objects=new Map();let generation=0,writes=0,deletes=0
  return {objects,get writes(){return writes},get deletes(){return deletes},bucket(){return {
    getMetadata:async()=>[{iamConfiguration:{publicAccessPrevention:privateBucket?'enforced':'inherited',uniformBucketLevelAccess:{enabled:true}}}],
    file(key,options={}){return {
      getMetadata:async()=>{const entry=objects.get(key);if(!entry)throw Object.assign(Error('private backend detail'),{code:404});return [entry.metadata]},
      createWriteStream(settings){const chunks=[];return new Writable({write(chunk,encoding,callback){chunks.push(chunk);callback()},final(callback){if(objects.has(key))return callback(Object.assign(Error('private conditional detail'),{code:412}));const data=Buffer.concat(chunks);objects.set(key,{data,metadata:{size:String(data.length),generation:String(++generation),contentType:settings.metadata.contentType,metadata:settings.metadata.metadata}});writes++;callback()}})},
      createReadStream(){const entry=objects.get(key);assert.equal(options.generation,entry.metadata.generation);return Readable.from([entry.data])},
      async delete(settings){const entry=objects.get(key);if(entry){if(settings.ifGenerationMatch!==entry.metadata.generation)throw Object.assign(Error('private generation detail'),{code:412});objects.delete(key);deletes++}},
    }},
  }}}
}
async function fixture() {
  const root=await mkdtemp(join(tmpdir(),'brain-gcs-worker-')),data=Buffer.from('synthetic validated image bytes'),sha256=createHash('sha256').update(data).digest('hex')
  const key=`brain-test-bucket/site/${owner}/2026/10/${id}/v1-${sha256}/image-${sha256}.png`
  await mkdir(join(root,key,'..'),{recursive:true});await writeFile(join(root,key),data,{mode:0o600})
  const object={key,source:key,bytes:data.length,sha256,contentType:'image/png'}
  return {root,data,object,job:(op,objects=[object])=>({schemaVersion:1,bucket:'brain-test-bucket',prefix:'site',root,request:{op,objects}})}
}
test('official SDK contract uses conditional creates, private policy, checksums, generation reads/deletes and idempotent reconciliation',{timeout:5000},async()=>{
  const f=await fixture(),storage=mock()
  try {
    const result=await execute(f.job('write'),async()=>storage);assert.equal(result.objects[0].generation,'1');assert.equal(storage.writes,1)
    await execute(f.job('write'),async()=>storage);assert.equal(storage.writes,1)
    const target='gcs-read/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.bin';await mkdir(join(f.root,'gcs-read'));await writeFile(join(f.root,target),'',{mode:0o600})
    await execute(f.job('read',[{...f.object,target,generation:'1'}]),async()=>storage);assert.deepEqual(await readFile(join(f.root,target)),f.data)
    await assert.rejects(execute(f.job('delete',[{key:f.object.key,generation:'2'}]),async()=>storage),/GCS_OPERATION_FAILED/);assert.equal(storage.deletes,0)
    await execute(f.job('delete',[{key:f.object.key,generation:''}]),async()=>storage);assert.equal(storage.deletes,1)
    await execute(f.job('delete',[{key:f.object.key,generation:'1'}]),async()=>storage);assert.equal(storage.deletes,1)
  }finally{await rm(f.root,{recursive:true,force:true})}
})
test('offline validation rejects traversal, wrong provider scope and oversized plans before any SDK import',{timeout:5000},async()=>{
  const f=await fixture()
  try {
    await validate(f.job('write'))
    for(const object of [{...f.object,key:'../outside'},{...f.object,source:'private.env'},{...f.object,bytes:21*1024*1024},{...f.object,key:f.object.key.replace('site/','other/')},{...f.object,sha256:'bad'}])await assert.rejects(validate(f.job('write',[object])),/GCS_OPERATION_FAILED/)
    await assert.rejects(validate(f.job('write',Array(10).fill(f.object))),/GCS_OPERATION_FAILED/)
    await assert.rejects(validate(f.job('read',[{...f.object,target:'../private'}])),/GCS_OPERATION_FAILED/)
  }finally{await rm(f.root,{recursive:true,force:true})}
})
test('public buckets and corrupted local sources fail without publishing unverified data',{timeout:5000},async()=>{
  const f=await fixture(),storage=mock(false)
  try {
    await assert.rejects(execute(f.job('write'),async()=>storage),/GCS_OPERATION_FAILED/);assert.equal(storage.writes,0)
    const privateStorage=mock();await writeFile(join(f.root,f.object.key),Buffer.alloc(f.data.length,1))
    await assert.rejects(execute(f.job('write'),async()=>privateStorage),/GCS_OPERATION_FAILED/)
  }finally{await rm(f.root,{recursive:true,force:true})}
})
