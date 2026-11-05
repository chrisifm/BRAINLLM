import {test} from 'node:test'
import assert from 'node:assert/strict'
import {mkdtemp,copyFile,writeFile,readFile,rm} from 'node:fs/promises'
import {tmpdir} from 'node:os'
import {join} from 'node:path'
import {spawnSync} from 'node:child_process'
import {compare} from './compare.mjs'
function run(pass=20){return {summary:{provider:'codex',model:'same-model',effort:'medium',mode:'spec',briefs:'briefs.json',fix:0,n:20,stable:true,infraFail:0,binSha:'a'.repeat(12),specSha:'b'.repeat(12),specShaEnd:'b'.repeat(12),tokensSha:'c'.repeat(12),tokensShaEnd:'c'.repeat(12),firstPass:pass+'/20'},rows:Array.from({length:20},(_,i)=>({id:'view-'+i,firstPass:i<pass}))}}
test('actual before/after comparison preserves the existing tolerance and rejects incomparable or fabricated summaries',()=>{
  assert.equal(compare(run(),run(19)).ok,true)
  assert.equal(compare(run(),run(18)).ok,false)
  const incomplete=run();delete incomplete.rows;assert.equal(compare(run(),incomplete).ok,false)
  const different=run();different.rows.forEach(row=>{row.id='other-'+row.id});assert.equal(compare(run(),different).ok,false)
  for(const patch of [{model:'different'},{infraFail:1},{stable:false},{firstPass:'19/20'},{n:1},{specShaEnd:'changed'}]){
    const after=run();Object.assign(after.summary,patch);assert.equal(compare(run(),after).ok,false)
  }
})

async function isolatedGate(t) {
  const dir=await mkdtemp(join(tmpdir(),'brain-llm-gate-'))
  t.after(()=>rm(dir,{recursive:true,force:true}))
  for(const file of ['gate.mjs','compare.mjs','codex.mjs']) await copyFile(new URL(file,import.meta.url),join(dir,file))
  let next=0
  return {dir,async invoke(value,args=[]) {
    const file=join(dir,`run-${next++}.json`)
    await writeFile(file,JSON.stringify(value))
    return spawnSync(process.execPath,[join(dir,'gate.mjs'),file,...args],{encoding:'utf8',timeout:5000,maxBuffer:65536})
  }}
}

test('the release gate refuses partial, malformed and fabricated evidence without touching a baseline',async(t)=>{
  const gate=await isolatedGate(t)
  for(const alter of [
    value=>{}, // A complete cohort still needs its own reviewed reference before release acceptance.
    value=>{value.summary.n=1;value.summary.firstPass='1/1';value.rows=value.rows.slice(0,1)},
    value=>{value.summary.n=0;value.summary.firstPass='0/0';value.rows=[]},
    value=>{value.rows[0]=null},
    value=>{value.rows[0].infraFail='Synthetic infrastructure failure'},
    value=>{value.rows[1].id=value.rows[0].id},
    value=>{value.summary.firstPass='19/20'},
    value=>{value.summary.binSha='not-a-digest'},
    value=>{delete value.summary.effort},
  ]) {
    const value=run();alter(value)
    const result=await gate.invoke(value)
    assert.equal(result.status,1,result.stdout+result.stderr)
  }
  await assert.rejects(readFile(join(gate.dir,'baseline.json')),{code:'ENOENT'})
})

test('Codex accepted baselines retain provider and effort identity and reject a same-cohort regression',async(t)=>{
  const gate=await isolatedGate(t)
  const accepted=await gate.invoke(run(),['--accept'])
  assert.equal(accepted.status,0,accepted.stdout+accepted.stderr)
  const baseline=JSON.parse(await readFile(join(gate.dir,'baseline.json'),'utf8'))
  assert.deepEqual(Object.keys(baseline),['codex|same-model|medium|spec'])
  assert.equal((await gate.invoke(run(18))).status,1)
  const other=run(18);other.summary.effort='high'
  const separate=await gate.invoke(other)
  assert.equal(separate.status,1,separate.stdout+separate.stderr)
  assert.match(separate.stdout,/baseline none/)
  assert.match(separate.stdout,/no accepted baseline/)
  assert.equal((await gate.invoke(other,['--accept'])).status,0)
  assert.equal((await gate.invoke(run(18))).status,1,'accepting high effort must not replace the medium reference')
})
