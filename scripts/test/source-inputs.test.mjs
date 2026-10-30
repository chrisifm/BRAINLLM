import {test} from 'node:test'
import assert from 'node:assert/strict'
import {execFileSync} from 'node:child_process'
import {mkdtempSync,writeFileSync,rmSync} from 'node:fs'
import {tmpdir} from 'node:os'
import {join} from 'node:path'
import {checkSourceInputs} from '../source-inputs-check.mjs'
test('clean-source gate detects a machine-only embedded template and accepts its explicit inclusion',()=>{
 const root=mkdtempSync(join(tmpdir(),'brain-source-inputs-'))
 try{
  execFileSync('git',['init','-q'],{cwd:root,stdio:'ignore',timeout:5000})
  writeFileSync(join(root,'main.rs'),'const TEMPLATE: &str = include_str!("needed.native");')
  writeFileSync(join(root,'needed.native'),'public source')
  writeFileSync(join(root,'.gitignore'),'needed.native\n')
  let report=checkSourceInputs(root);assert.equal(report.ok,false);assert.equal(report.findings[0].rule,'embedded-input-excluded-from-source')
  writeFileSync(join(root,'.gitignore'),'needed.native\n!needed.native\n')
  assert.equal(checkSourceInputs(root).ok,true)
 }finally{rmSync(root,{recursive:true,force:true})}
})
