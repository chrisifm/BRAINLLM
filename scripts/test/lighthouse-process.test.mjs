// Verify the actual CLI-to-Lighthouse process boundary without downloading a browser audit dependency.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { mkdtempSync,writeFileSync,readFileSync,chmodSync,rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join,delimiter } from 'node:path'
const brain=process.env.BRAIN_BIN??new URL('../../target/debug/brain',import.meta.url).pathname
const categories=Object.fromEntries(['performance','accessibility','best-practices','seo'].map(name=>[name,{score:1}]))
test('Lighthouse failures cannot become an empty successful audit; explicit browser options reach the process', {skip:process.platform==='win32'},()=>{
  const root=mkdtempSync(join(tmpdir(),'brain-lighthouse-process-'))
  try {
    const npx=join(root,'npx'),seen=join(root,'seen.json')
    writeFileSync(npx,'#!/usr/bin/env node\nconst fs=require("node:fs");fs.writeFileSync(process.env.BRAIN_LH_SEEN,JSON.stringify({args:process.argv.slice(2),chrome:process.env.CHROME_PATH}));process.stdout.write(process.env.BRAIN_LH_REPORT);process.exit(Number(process.env.BRAIN_LH_EXIT));\n')
    chmodSync(npx,0o700)
    const run=(report,exit=0,options=[])=>spawnSync(brain,['lighthouse','https://example.test','--json',...options],{cwd:root,encoding:'utf8',timeout:10000,env:{...process.env,PATH:root+delimiter+process.env.PATH,CHROME_PATH:'/browser/fixture',BRAIN_LH_SEEN:seen,BRAIN_LH_REPORT:JSON.stringify(report),BRAIN_LH_EXIT:String(exit)}})
    for(const [report,exit] of [[{runtimeError:{code:'FAILED_DOCUMENT_REQUEST'}},1],[{runtimeError:{code:'PAGE_HUNG'}},0],[{},0],[{categories:{performance:{score:1}}},0],[{categories:{...categories,seo:{score:null}}},0]]){
      const result=run(report,exit)
      assert.equal(result.status,2,result.stdout+result.stderr)
      assert.equal(JSON.parse(result.stdout).ok,false)
    }
    const unmeasurable=run({categories,audits:{'is-crawlable':{score:0}}},0,['--expect-noindex'])
    assert.equal(unmeasurable.status,2,unmeasurable.stdout+unmeasurable.stderr)
    assert.equal(JSON.parse(unmeasurable.stdout).ok,false)
    const stagedCategories={...categories,seo:{score:0.5,auditRefs:[{id:'is-crawlable',weight:1},{id:'meta-description',weight:1}]}}
    const staged=run({categories:stagedCategories,audits:{'is-crawlable':{score:0},'meta-description':{score:1}}},0,['--expect-noindex'])
    assert.equal(staged.status,0,staged.stdout+staged.stderr)
    assert.equal(JSON.parse(staged.stdout).scores.seo,100)
    for(const audit of [undefined,{}, {score:'invalid'},{score:-1},{score:2}]){
      const partial={...categories,seo:{score:0.5,auditRefs:[{id:'is-crawlable',weight:1},{id:'missing',weight:1},{id:'meta-description',weight:1}]}}
      const measured=run({categories:partial,audits:{'is-crawlable':{score:0},'meta-description':{score:1},...(audit===undefined?{}:{missing:audit})}},0,['--expect-noindex'])
      assert.equal(measured.status,2,measured.stdout+measured.stderr)
    }
    const nonApplicable={...categories,seo:{score:0.5,auditRefs:[{id:'is-crawlable',weight:1},{id:'optional',weight:1},{id:'meta-description',weight:1}]}}
    const nullable=run({categories:nonApplicable,audits:{'is-crawlable':{score:0},optional:{score:null},'meta-description':{score:1}}},0,['--expect-noindex'])
    assert.equal(nullable.status,0,nullable.stdout+nullable.stderr)
    assert.equal(JSON.parse(nullable.stdout).scores.seo,100)
    const complete=run({categories,finalDisplayedUrl:'https://example.test'},0,['--preset=desktop','--chrome-flags=--no-sandbox --disable-dev-shm-usage'])
    assert.equal(complete.status,0,complete.stdout+complete.stderr)
    assert.equal(JSON.parse(complete.stdout).ok,true)
    const command=JSON.parse(readFileSync(seen,'utf8'))
    assert.equal(command.chrome,'/browser/fixture')
    assert.ok(command.args.includes('--preset=desktop'))
    assert.ok(command.args.includes('--chrome-flags=--headless=new --no-sandbox --disable-dev-shm-usage'))
    const spaced=run({categories},0,['--preset','desktop','--chrome-flags','--disable-dev-shm-usage'])
    assert.equal(spaced.status,0,spaced.stdout+spaced.stderr)
    writeFileSync(join(root,'tokens.cssbrain'),':root{}')
    writeFileSync(join(root,'brain.json'),JSON.stringify({lighthouse:{minScores:{performance:99.5}}}))
    const threshold=run({categories:{...categories,performance:{score:0.99}}})
    assert.equal(threshold.status,1,threshold.stdout+threshold.stderr)
    writeFileSync(join(root,'brain.json'),JSON.stringify({lighthouse:{minScores:{performance:99}}}))
    assert.equal(run({categories:{...categories,performance:{score:0.99}}}).status,0)
    for(const minimum of [-1,101,'99',null]){
      writeFileSync(join(root,'brain.json'),JSON.stringify({lighthouse:{minScores:{performance:minimum}}}))
      writeFileSync(seen,'not invoked')
      const invalidMinimum=run({categories})
      assert.equal(invalidMinimum.status,2,invalidMinimum.stdout+invalidMinimum.stderr)
      assert.equal(readFileSync(seen,'utf8'),'not invoked','invalid minimums must fail before invoking Lighthouse')
    }
    writeFileSync(join(root,'brain.json'),'{}')
    const invalid=run({categories},0,['--chrome-flags','--bad\n--flag'])
    assert.equal(invalid.status,2)
  } finally {rmSync(root,{recursive:true,force:true})}
})
