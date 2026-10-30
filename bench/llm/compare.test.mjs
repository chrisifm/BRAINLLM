import {test} from 'node:test'
import assert from 'node:assert/strict'
import {compare} from './compare.mjs'
function run(pass=20){return {summary:{provider:'synthetic',model:'same-model',effort:'medium',mode:'spec',briefs:'briefs.json',fix:0,n:20,stable:true,infraFail:0,binSha:'a'.repeat(12),specSha:'b'.repeat(12),specShaEnd:'b'.repeat(12),tokensSha:'same',tokensShaEnd:'same',firstPass:pass+'/20'},rows:Array.from({length:20},(_,i)=>({firstPass:i<pass}))}}
test('actual before/after comparison preserves the existing tolerance and rejects incomparable or fabricated summaries',()=>{
  assert.equal(compare(run(),run(19)).ok,true)
  assert.equal(compare(run(),run(18)).ok,false)
  const incomplete=run();delete incomplete.rows;assert.equal(compare(run(),incomplete).ok,false)
  for(const patch of [{model:'different'},{infraFail:1},{stable:false},{firstPass:'19/20'},{n:1},{specShaEnd:'changed'}]){
    const after=run();Object.assign(after.summary,patch);assert.equal(compare(run(),after).ok,false)
  }
})
