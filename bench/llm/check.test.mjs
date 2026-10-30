import {test} from 'node:test'
import assert from 'node:assert/strict'
import {checkedDiagnostics} from './check.mjs'
test('compiler crashes and invalid output cannot count as successful model generations',()=>{
  for(const [raw,status] of [['',1],['',null],['[]',2],['{}',0],['broken',0],['[]',1],[JSON.stringify([{code:'B0201',msg:'Missing',file:'views/a.brain'}]),0]]) assert.throws(()=>checkedDiagnostics(raw,status,'views/a.brain'))
  assert.deepEqual(checkedDiagnostics('[]',0,'views/a.brain'),[])
  const diagnostic={code:'B0201',msg:'Missing',file:'views/a.brain',level:'error'}
  assert.deepEqual(checkedDiagnostics(JSON.stringify([diagnostic]),1,'views/a.brain'),[diagnostic])
  assert.throws(()=>checkedDiagnostics(JSON.stringify([diagnostic]),1,'views/different.brain'),/scaffold/)
  assert.deepEqual(checkedDiagnostics(JSON.stringify([{...diagnostic,level:'warning'}]),0,'views/a.brain'),[])
})
