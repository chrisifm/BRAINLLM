import test from 'node:test'
import assert from 'node:assert/strict'
import {spawn} from 'node:child_process'
import {chmodSync,existsSync,mkdtempSync,rmSync,writeFileSync} from 'node:fs'
import {join} from 'node:path'
import {tmpdir} from 'node:os'
import {fileURLToPath} from 'node:url'

const launcher=fileURLToPath(new URL('../brainllm/bin/brain.js',import.meta.url))

test('a signal to the launcher reaches the binary and its exit code passes through',{skip:process.platform==='win32'},async()=>{
 const dir=mkdtempSync(join(tmpdir(),'brain-launcher-'))
 try{
  const marker=join(dir,'stopped'),fake=join(dir,'brain')
  writeFileSync(fake,`#!/bin/sh\ntrap 'touch "${marker}"; exit 7' TERM\necho ready\nwhile :; do sleep 0.05; done\n`)
  chmodSync(fake,0o755)
  const run=spawn(process.execPath,[launcher],{cwd:dir,env:{...process.env,BRAIN_BINARY:fake,BRAIN_NO_DELEGATE:'1'},stdio:['ignore','pipe','inherit']})
  await new Promise((resolve,reject)=>{run.stdout.once('data',resolve);run.once('error',reject)})
  run.kill('SIGTERM')
  const [code]=await new Promise(resolve=>run.once('exit',(...result)=>resolve(result)))
  assert.equal(code,7)
  assert.ok(existsSync(marker),'the binary received SIGTERM')
 }finally{rmSync(dir,{recursive:true,force:true})}
})
