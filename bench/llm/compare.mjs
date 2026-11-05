// Compare two actual frozen source-generation runs without rewriting accepted historical baselines.
import {readFileSync} from 'node:fs'
import {pathToFileURL} from 'node:url'
import {resolve} from 'node:path'
import {codexPins} from './codex.mjs'
export function cohortKey(summary) {
  if(typeof summary?.model!=='string'||!/^[A-Za-z0-9][A-Za-z0-9._:/-]{0,127}$/.test(summary.model)||!['spec','mcp'].includes(summary.mode??'spec')) throw Error('invalid cohort')
  if(summary.provider==='codex') {
    codexPins(summary.model,summary.effort)
    return `codex|${summary.model}|${summary.effort}|${summary.mode??'spec'}`
  }
  return `${summary.model}|${summary.mode??'spec'}`
}
export function validateRun(run,name='run') {
  const errors=[],s=run?.summary
  if(!s||s.stable!==true||s.infraFail!==0||!Number.isInteger(s.n)||s.n<20||!Array.isArray(run.rows)||run.rows.length!==s.n) return [name+': incomplete or unstable evidence (at least 20 complete generations required)']
  try {cohortKey(s)} catch {errors.push(name+': invalid provider/model/effort/mode cohort')}
  if(!['binSha','specSha','specShaEnd','tokensSha','tokensShaEnd'].every(key=>typeof s[key]==='string'&&/^[a-f0-9]{12,64}$/.test(s[key]))||s.specSha!==s.specShaEnd||s.tokensSha!==s.tokensShaEnd) errors.push(name+': invalid frozen provenance')
  if(run.rows.some(row=>!row||row.infraFail!==undefined||typeof row.firstPass!=='boolean'||typeof row.id!=='string'||!row.id)||new Set(run.rows.map(row=>row?.id)).size!==s.n) return [...errors,name+': incomplete or duplicate generation rows']
  const measured=run.rows.filter(row=>row.firstPass).length
  if(s.firstPass!==measured+'/'+s.n) errors.push(name+': summary disagrees with generation evidence')
  return errors
}
export function compare(before,after) {
  const errors=[]
  for(const [name,run] of [['before',before],['after',after]]) {
    errors.push(...validateRun(run,name))
  }
  const a=before?.summary,b=after?.summary
  if(a&&b) {
    for(const key of ['provider','model','effort','mode','briefs','fix','n']) if(a[key]!==b[key]) errors.push('incomparable '+key)
    if(!errors.length) {
      const ids=new Set(before.rows.map(row=>row.id))
      if(after.rows.some(row=>!ids.has(row.id))) errors.push('incomparable brief identities')
      const beforePass=before.rows.filter(row=>row.firstPass===true).length,afterPass=after.rows.filter(row=>row.firstPass===true).length
      if(afterPass<b.n*(beforePass-1)/a.n) errors.push('first-pass regression exceeds the existing one-view tolerance')
    }
  }
  return {schemaVersion:1,ok:errors.length===0,before:a??null,after:b??null,toleranceViews:1,errors,qualification:'Actual supplied-spec generations with tools disabled. Not evidence for every backend, browser or application workflow.'}
}
if(process.argv[1]&&import.meta.url===pathToFileURL(resolve(process.argv[1])).href) {
  try {
    if(process.argv.length!==4) throw Error('Usage: node bench/llm/compare.mjs before.json after.json')
    const result=compare(...process.argv.slice(2).map(path=>JSON.parse(readFileSync(path,'utf8'))))
    console.log(JSON.stringify(result,null,2));process.exitCode=result.ok?0:1
  } catch {console.error('Invalid comparison inputs; supply two complete frozen benchmark artifacts');process.exitCode=2}
}
