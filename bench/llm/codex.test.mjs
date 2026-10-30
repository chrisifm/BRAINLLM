import { test } from 'node:test'
import assert from 'node:assert/strict'
import { mkdtemp, writeFile, readdir, rm, readFile } from 'node:fs/promises'
import { join, dirname } from 'node:path'
import { tmpdir } from 'node:os'
import { fileURLToPath } from 'node:url'
import { spawn } from 'node:child_process'
import { codexGenerate, codexPins } from './codex.mjs'

const here = dirname(fileURLToPath(import.meta.url))
const pins = { model: 'synthetic-model', effort: 'medium' }
async function fixture(t, body) {
  const dir = await mkdtemp(join(tmpdir(), 'brain-codex-test-'))
  t.after(() => rm(dir, { recursive: true, force: true }))
  const script = join(dir, 'fake-codex')
  // Execute the known Node binary directly: a new executable shebang file can stall in OS startup checks.
  await writeFile(script, body, { mode: 0o600 })
  return { dir, bin: process.execPath, binArgs: [script], script, scratch: dir, deadlineMs: 5000 }
}
const complete = `process.stdout.write(JSON.stringify({type:'turn.completed',usage:{input_tokens:120,cached_input_tokens:20,output_tokens:40}})+'\\n')`
const last = `process.argv[process.argv.indexOf('--output-last-message')+1]`

test('Codex requires explicit pins before executable, brain binary or filesystem access', async () => {
  assert.throws(() => codexPins(undefined, 'medium'), /--model must be explicit/)
  assert.throws(() => codexPins('synthetic-model', undefined), /--effort must be explicit/)
  assert.throws(() => codexPins('x\nsecret', 'high'), /valid model/)
  assert.throws(() => codexPins('model', 'guessed'), /--effort/)
  await assert.rejects(codexGenerate('spec', 'brief', { bin: '/missing-executable' }), /--model/)
  for (const binArgs of ['node script', ['bad\nargument'], Array(9).fill('arg')])
    await assert.rejects(codexGenerate('spec','brief',{...pins,binArgs}), /invalid executable argument prefix/)
  for (const args of [['--provider','codex'], ['--provider','codex','--model','synthetic-model']]) {
    const result = await exec(process.execPath, [join(here, 'run.mjs'), ...args, '--brain', '/missing-brain'])
    assert.notEqual(result.code, 0)
    assert.match(result.err, /--(?:model|effort) must be explicit/)
    assert.doesNotMatch(result.err, /shasum|missing-brain/)
  }
})

test('Codex reads the owned last message, supplies only explicit pins and counts completed-turn usage once', async (t) => {
  const f = await fixture(t, `
const assert=require('node:assert/strict'),fs=require('node:fs');
const a=process.argv.slice(2);
for(const value of ['exec','--ignore-user-config','--ignore-rules','--ephemeral','--json','--skip-git-repo-check','project_doc_max_bytes=0','features.shell_tool=false','web_search="disabled"','model_reasoning_effort="medium"']) assert(a.includes(value));
assert.equal(a[a.indexOf('--model')+1],'synthetic-model'); assert.equal(a[a.indexOf('--sandbox')+1],'read-only'); assert.equal(a.at(-1),'-');
let input='';process.stdin.setEncoding('utf8');process.stdin.on('data',d=>input+=d);process.stdin.on('end',()=>{
assert(input.includes('Synthetic SPEC')&&input.includes('Synthetic brief')&&input.includes('Do not call tools'));
fs.writeFileSync(${last},'<p>Generated</p>\\n');
console.log(JSON.stringify({type:'thread.started',thread_id:'private-thread-marker'}));
console.log(JSON.stringify({type:'item.completed',item:{type:'agent_message',text:'WRONG transcript text'}}));
${complete};});`)
  const result = await codexGenerate('Synthetic SPEC', 'Synthetic brief', { ...pins, ...f })
  assert.deepEqual(result, { text: '<p>Generated</p>\n', input: 120, output: 40, cachedInput: 20, cost: null })
  assert.deepEqual(await readdir(f.dir), ['fake-codex'])
})

test('failed, malformed, missing-usage and tool-using generations fail without leaking stderr or transcripts', async (t) => {
  for (const [body, expected] of [
    [`console.error('synthetic-private-marker');console.log('{broken')`, /malformed JSON event/],
    [`console.log(JSON.stringify({type:42}))`, /invalid event/],
    [`console.log(JSON.stringify({type:'turn.failed',error:{message:'synthetic-private-marker'}}))`, /generation failed/],
    [`console.log(JSON.stringify({type:'turn.completed',usage:{input_tokens:-1,output_tokens:1}}))`, /invalid turn usage/],
    [`console.log(JSON.stringify({type:'item.started',item:{type:'command_execution',command:'synthetic-private-marker'}}))`, /tools are forbidden/],
    [`console.log(JSON.stringify({type:'thread.started',thread_id:'synthetic-private-marker'}))`, /no successful completed turn/],
  ]) {
    const f = await fixture(t, body)
    await assert.rejects(codexGenerate('spec', 'brief', { ...pins, ...f }), (e) => expected.test(e.message) && !e.message.includes('synthetic-private-marker'))
    assert.deepEqual(await readdir(f.dir), ['fake-codex'])
  }
})

test('stdout, stderr and final-message limits fail closed and clean their owned scratch', async (t) => {
  for (const [body, expected] of [
    [`process.stdout.write('x'.repeat(2*1024*1024+1))`, /stdout exceeds/],
    [`process.stderr.write('x'.repeat(64*1024+1));setInterval(()=>{},1000)`, /stderr exceeds/],
    [`require('node:fs').writeFileSync(${last},'x'.repeat(2*1024*1024+1));${complete}`, /last message/],
    [`require('node:fs').symlinkSync('/etc/hosts',${last});${complete}`, /last message/],
    [`${complete}`, /last message/],
  ]) {
    const f = await fixture(t, body)
    await assert.rejects(codexGenerate('spec', 'brief', { ...pins, ...f }), expected)
    assert.deepEqual(await readdir(f.dir), ['fake-codex'])
  }
})

test('absolute deadline kills a stubborn owned process group, reaps its parent and removes scratch', { skip: process.platform === 'win32' }, async (t) => {
  const f = await fixture(t, `
const fs=require('node:fs'),{spawn}=require('node:child_process'),path=require('node:path');
process.on('SIGTERM',()=>{});
const child=spawn(process.execPath,['-e',"process.on('SIGTERM',()=>{});setInterval(()=>{},1000)"],{stdio:'ignore'});
fs.writeFileSync(path.join(process.cwd(),'..','pids'),JSON.stringify([process.pid,child.pid]));
setInterval(()=>process.stdout.write(JSON.stringify({type:'item.updated',item:{type:'reasoning'}})+'\\n'),20);`)
  const start = Date.now()
  await assert.rejects(codexGenerate('spec', 'brief', { ...pins, ...f, deadlineMs: 350 }), /deadline exceeded/)
  assert(Date.now() - start < 3000)
  const pids = JSON.parse(await readFile(join(f.dir, 'pids'), 'utf8'))
  for (const pid of pids) {
    // A killed descendant may briefly remain a zombie until the OS adopts/reaps it.
    for (let i=0;i<30;i++) { try { process.kill(pid, 0); await new Promise(r=>setTimeout(r,20)) } catch { break } }
    assert.throws(() => process.kill(pid, 0), /ESRCH/)
  }
  assert.deepEqual((await readdir(f.dir)).sort(), ['fake-codex','pids'])
})

async function exec(bin, args, env = process.env) {
  return new Promise((resolve, reject) => {
    const p = spawn(bin, args, { env, stdio: ['ignore','pipe','pipe'] })
    let out='',err=''
    const timer=setTimeout(()=>p.kill('SIGKILL'),10000)
    p.stdout.on('data',d=>out+=d);p.stderr.on('data',d=>err+=d)
    p.on('error',reject);p.on('close',code=>{clearTimeout(timer);resolve({code,out,err})})
  })
}

test('SIGTERM cancels the active generation and prevents a subsequent paid retry', { skip: process.platform === 'win32' }, async (t) => {
  const f = await fixture(t, `
require('node:fs').writeFileSync(require('node:path').join(process.cwd(),'..','ready'),String(process.pid));
setInterval(()=>{},1000);`)
  const code = `import {codexGenerate} from ${JSON.stringify(new URL('./codex.mjs',import.meta.url).href)};
const options=${JSON.stringify({...pins,...f})};
const messages=[];for(let i=0;i<2;i++){try{await codexGenerate('spec','brief',options)}catch(e){messages.push(e.message)}}
console.log(JSON.stringify(messages));`
  const child = spawn(process.execPath, ['--input-type=module','-e',code], { stdio:['ignore','pipe','pipe'] })
  t.after(()=>{try{child.kill('SIGKILL')}catch{}})
  const done = new Promise((resolve,reject)=>{
    let out='',err=''
    child.stdout.on('data',d=>out+=d);child.stderr.on('data',d=>err+=d)
    child.on('error',reject);child.on('close',status=>resolve({status,out,err}))
  })
  let ready=false
  for(let i=0;i<100;i++){try{await readFile(join(f.dir,'ready'),'utf8');ready=true;break}catch{await new Promise(r=>setTimeout(r,20))}}
  assert(ready,'fake CLI must start before cancellation')
  child.kill('SIGTERM')
  let timer
  const result=await Promise.race([done,new Promise((_,reject)=>{timer=setTimeout(()=>reject(new Error('cancel test deadline')),3000)})]).finally(()=>clearTimeout(timer))
  assert.equal(result.status,0,result.err)
  assert.deepEqual(JSON.parse(result.out),['codex: benchmark cancelled','codex: benchmark cancelled'])
  const pid=Number(await readFile(join(f.dir,'ready'),'utf8'))
  assert.throws(()=>process.kill(pid,0),/ESRCH/)
  assert.deepEqual((await readdir(f.dir)).sort(),['fake-codex','ready'])
})

test('frozen run artifacts compose with the existing gate and preserve unknown cost', async (t) => {
  const stage=`adapter-test-${process.pid}-${Date.now()}`
  const dottedModel='synthetic-model.v1'
  const tag=`codex-${dottedModel}-medium-${stage}`
  t.after(async()=>{await rm(join(here,'results',tag),{recursive:true,force:true});await rm(join(here,'results',`${tag}-03-login.json`),{force:true})})
  const f = await fixture(t, `require('node:fs').writeFileSync(${last},'<p>Generated</p>\\n');${complete}`)
  const brain=join(f.dir,'brain')
  await writeFile(brain, `
const fs=require('node:fs'),path=require('node:path');const a=process.argv.slice(2);
if(a[0]==='spec') console.log('## Reference component\\nSynthetic reference\\n## SEO\\nSynthetic SEO');
else if(a[0]==='new'){if(!/^[a-z][a-z0-9-]*$/.test(path.basename(a[1])))process.exit(2);fs.mkdirSync(a[1],{recursive:true});fs.writeFileSync(path.join(a[1],'tokens.cssbrain'),'--space-1: 4px;');}
else if(a[0]==='check') console.log('[]');else process.exit(2);
`,{mode:0o600})
  // Keep run.mjs and gate.mjs real. Only the two synthetic executables are transported through Node,
  // including the exact brain source bytes copied/hash-frozen by run.mjs, without executing a temp shebang.
  const preload=join(f.dir,'synthetic-launcher.mjs')
  const work=join(f.dir,'work')
  await writeFile(preload,`import cp from 'node:child_process';import {syncBuiltinESMExports} from 'node:module';import {dirname,basename} from 'node:path';
const originalSpawn=cp.spawn,originalExec=cp.execFileSync;
cp.spawn=function(bin,args,options){return bin===${JSON.stringify(f.script)}?originalSpawn(process.execPath,[bin,...args],options):originalSpawn(bin,args,options)};
cp.execFileSync=function(bin,args,options){const fake=bin===${JSON.stringify(brain)}||(dirname(bin)===${JSON.stringify(join(work,'bin'))}&&basename(bin).startsWith('brain-'));return fake?originalExec(process.execPath,[bin,...args],options):originalExec(bin,args,options)};
syncBuiltinESMExports();`)
  const r=await exec(process.execPath,['--import',preload,join(here,'run.mjs'),'--provider','codex','--model',dottedModel,'--effort',pins.effort,'--brain',brain,'--fix','0','--only','03-login','--stage',stage],{...process.env,BRAIN_CODEX_BIN:f.script,QA_LLM_WORK:work})
  assert.equal(r.code,0,r.err)
  const artifact=join(here,'results',`${tag}-03-login.json`)
  const {summary,rows}=JSON.parse(await readFile(artifact,'utf8'))
  assert.equal(summary.provider,'codex');assert.equal(summary.effort,'medium');assert.equal(summary.costUSD,null)
  assert.equal(summary.stable,true);assert.equal(summary.n,1);assert.equal(summary.firstPass,'1/1');assert.equal(summary.infraFail,0)
  assert.equal(rows[0].tries[0].cost,null);assert.equal(rows[0].tries[0].cachedInput,20)
  const gate=await exec(process.execPath,[join(here,'gate.mjs'),artifact])
  assert.equal(gate.code,0,gate.out+gate.err)
  assert.match(gate.out,/baseline none → PASS/)
})
