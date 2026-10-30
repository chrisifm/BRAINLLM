import test from 'node:test'
import assert from 'node:assert/strict'
import { readFileSync, mkdtempSync, writeFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { pathToFileURL } from 'node:url'
import { spawn, spawnSync } from 'node:child_process'
const source=readFileSync(new URL('../../crates/brain-serve/src/load.rs',import.meta.url),'utf8')
const runner=source.split('const RUNNER: &str = r#"')[1]?.split('"#;')[0]
assert.ok(runner,'Use the actual owned sidecar runner')
const delay=ms=>new Promise(resolve=>setTimeout(resolve,ms))
const live=pid=>{try{process.kill(pid,0)}catch{return false};const result=spawnSync('/bin/ps',['-p',String(pid),'-o','stat='],{encoding:'utf8',timeout:1000,maxBuffer:1024});assert.ifError(result.error);assert.ok(result.status===0||result.status===1);return !!result.stdout.trim()&&!result.stdout.trim().startsWith('Z')}
const exited=child=>new Promise(resolve=>{if(child.exitCode!==null||child.signalCode!==null)return resolve();child.once('exit',resolve);child.once('error',resolve)})
test('owned sidecar watchdog terminates a blocking hook after its parent exits',{timeout:8000,skip:process.platform==='win32'},async()=>{
  const root=mkdtempSync(join(tmpdir(),'brain-hook-watchdog-'))
  const file=join(root,'loop.mjs');writeFileSync(file,'export default function(){while(true){}}\n')
  // This parent owns exactly one child, forwards no credentials and exits after the loop starts.
  const parentCode=`const {spawn}=require('node:child_process');const child=spawn(process.execPath,['--input-type=module','-e',${JSON.stringify(runner)}],{env:{PATH:process.env.PATH,BRAIN_SIDECAR_PARENT:String(process.pid),BRAIN_SIDECAR_GROUP:'1'},stdio:['pipe','ignore','ignore'],detached:true});child.stdin.write(JSON.stringify({id:1,file:${JSON.stringify(pathToFileURL(file).href)},fn:'default',ctx:{}})+'\\n');process.stdout.write(JSON.stringify({pid:child.pid})+'\\n');setTimeout(()=>process.exit(0),300)`
  const parent=spawn(process.execPath,['-e',parentCode],{stdio:['ignore','pipe','ignore']})
  let pid,output=''
  try {
    const startup=Date.now()+2000
    parent.stdout.on('data',chunk=>{output+=chunk;if(output.length>1024)parent.kill('SIGKILL');try{pid=JSON.parse(output.trim()).pid}catch{}})
    while(!pid&&Date.now()<startup)await delay(10)
    assert.ok(Number.isInteger(pid)&&pid>1,'The fixture must report its own child')
    await Promise.race([exited(parent),delay(2000).then(()=>{throw Error('Owned fixture parent did not exit')})])
    const deadline=Date.now()+2000
    while(live(pid)&&Date.now()<deadline)await delay(20)
    assert.equal(live(pid),false,'A separate watchdog must stop an orphan even while its main thread loops')
  } finally {
    if(pid&&live(pid))try{process.kill(pid,'SIGKILL')}catch{}
    if(parent.exitCode===null&&parent.signalCode===null)parent.kill('SIGKILL')
    await exited(parent);rmSync(root,{recursive:true,force:true})
  }
})
