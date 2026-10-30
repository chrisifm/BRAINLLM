// Compare two actual frozen source-generation runs without rewriting accepted historical baselines.
import {readFileSync} from 'node:fs'
import {pathToFileURL} from 'node:url'
import {resolve} from 'node:path'
export function compare(before,after) {
  const errors=[]
  for(const [name,run] of [['before',before],['after',after]]) {
    const s=run?.summary
    if(!s||s.stable!==true||s.infraFail!==0||!Number.isInteger(s.n)||s.n<20||run.rows?.length!==s.n) {errors.push(name+': incomplete or unstable evidence');continue}
    if(!/^[a-f0-9]{12,64}$/.test(s.binSha)||!/^[a-f0-9]{12,64}$/.test(s.specSha)||s.specSha!==s.specShaEnd||s.tokensSha!==s.tokensShaEnd) errors.push(name+': invalid frozen provenance')
    const measured=run.rows.filter(row=>row.firstPass===true).length
    if(s.firstPass!==measured+'/'+s.n) errors.push(name+': summary disagrees with generation evidence')
  }
  const a=before?.summary,b=after?.summary
  if(a&&b) {
    for(const key of ['provider','model','effort','mode','briefs','fix','n']) if(a[key]!==b[key]) errors.push('incomparable '+key)
    if(a.n>=20&&b.n>=20&&Array.isArray(before.rows)&&Array.isArray(after.rows)) {
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
