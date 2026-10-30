import { test } from 'node:test'
import assert from 'node:assert/strict'
import { mkdtempSync, mkdirSync, writeFileSync, rmSync, chmodSync } from 'node:fs'
import { join, resolve } from 'node:path'
import { tmpdir } from 'node:os'
import { spawnSync } from 'node:child_process'
const hook = resolve('crates/brain-cli/hooks/check.mjs')
const root = mkdtempSync(join(tmpdir(),'brain-hook-'))
const run = value => spawnSync(process.execPath,[hook],{cwd:root,input:JSON.stringify(value),encoding:'utf8',timeout:25000,env:{...process.env,CLAUDE_PROJECT_DIR:root}})
test('hook ignores unrelated edits, checks local source with argv, and reports missing pinned binary', t => {
  t.after(()=>rmSync(root,{recursive:true,force:true}))
  writeFileSync(join(root,'view.brain'),'<main>Page</main>')
  assert.equal(run({tool_name:'Bash',tool_input:{file_path:'view.brain'}}).stdout,'')
  assert.equal(run({tool_name:'Write',tool_input:{file_path:'outside.txt'}}).stdout,'')
  const missing=JSON.parse(run({tool_name:'Write',tool_input:{file_path:'view.brain'}}).stdout)
  assert.match(missing.hookSpecificOutput.additionalContext,/install the pinned/)
  mkdirSync(join(root,'node_modules','.bin'),{recursive:true})
  const bin=join(root,'node_modules','.bin','brain')
  writeFileSync(bin,'#!/usr/bin/env node\nif(process.argv.slice(2).join(" ")!=="check --json")process.exit(5);console.log(JSON.stringify([{code:"B0202",fix:"Add a key"}]));process.exitCode=1\n')
  chmodSync(bin,0o755)
  const result=run({tool_name:'Edit',tool_input:{file_path:join(root,'view.brain')}})
  assert.equal(result.status,0)
  const output=JSON.parse(result.stdout)
  assert.equal(output.hookSpecificOutput.hookEventName,'PostToolUse')
  assert.match(output.hookSpecificOutput.additionalContext,/B0202/)
  assert.match(output.hookSpecificOutput.additionalContext,/Exit 1/)
  assert.equal(run({tool_name:'Edit',tool_input:{file_path:'../escaped.brain'}}).stdout,'')
})
