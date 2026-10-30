import { test } from 'node:test'
import assert from 'node:assert/strict'
import { mkdtempSync, rmSync, existsSync, readFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { spawnSync } from 'node:child_process'
const binary = process.env.BRAIN_BIN || resolve('target/debug/brain')
function call(cwd,profile,requests) {
  const init={jsonrpc:'2.0',id:0,method:'initialize',params:{protocolVersion:'2025-11-25',capabilities:{},clientInfo:{name:'brain-profile-check',version:'1'}}}
  const messages=[init,{jsonrpc:'2.0',method:'notifications/initialized'},...requests.map((r,i)=>({jsonrpc:'2.0',id:i+1,...r}))]
  const r=spawnSync(binary,['mcp','--profile',profile],{cwd,input:messages.map(JSON.stringify).join('\n')+'\n',encoding:'utf8',timeout:20000,maxBuffer:1024*1024})
  assert.equal(r.status,0,r.stderr)
  return r.stdout.trim().split('\n').filter(Boolean).map(s=>JSON.parse(s))
}
test('MCP minimal advertises only core tools; backend generates and discovers one actual contract',()=>{
  const root=mkdtempSync(join(tmpdir(),'brain-mcp-profile-'))
  try {
    const made=spawnSync(binary,['new','app','--json'],{cwd:root,encoding:'utf8',timeout:20000})
    assert.equal(made.status,0,made.stderr)
    const app=join(root,'app')
    const minimal=call(app,'minimal',[{method:'tools/list'}]).find(r=>r.id===1)
    assert.deepEqual(minimal.result.tools.map(t=>t.name).sort(),['brain_check','brain_doctor','brain_fix','brain_map','brain_spec','brain_test'])
    const backend=call(app,'backend',[
      {method:'tools/list'},
      {method:'tools/call',params:{name:'brain_resource_add',arguments:{name:'posts',fields:'title:str! body:str'}}},
      {method:'tools/call',params:{name:'brain_backend_contract',arguments:{name:'posts'}}},
    ])
    assert.equal(backend.find(r=>r.id===1).result.tools.length,10)
    const created=backend.find(r=>r.id===2).result
    assert.notEqual(created.isError,true,JSON.stringify(created))
    assert.ok(existsSync(join(app,'server/resources/posts/schema.jsbrain')))
    const card=backend.find(r=>r.id===3).result.structuredContent
    assert.equal(card.contract.fields.title.required,true)
    assert.deepEqual(card.contract.policy.create,['admin'])
    assert.equal(card.files.length,3)
    assert.match(readFileSync(join(app,'.brain/journal.jsonl'),'utf8'),/server\/resources\/posts\/schema.jsbrain/)
    const registration=JSON.parse(readFileSync(join(app,'.mcp.json'),'utf8'))
    assert.equal(registration.mcpServers.brain.command,'npx')
    assert.equal(readFileSync(join(app,'CLAUDE.md'),'utf8'),'@AGENTS.md\n')
  } finally {rmSync(root,{recursive:true,force:true})}
})
