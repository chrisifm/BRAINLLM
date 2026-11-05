// A component must not destroy the harness; a failed test must not erase unrelated assertions.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from 'node:fs'
import { join } from 'node:path'
import { tmpdir } from 'node:os'
const brain = process.env.BRAIN_BIN ?? new URL('../../target/debug/brain', import.meta.url).pathname
test('test-document navigation is attributed to its file and test while later tests and files still run', { timeout: 120000 }, () => {
  const root=mkdtempSync(join(tmpdir(),'brain-test-navigation-'))
  const cli=(args,cwd)=>spawnSync(brain,args,{cwd,encoding:'utf8',timeout:90000,maxBuffer:4*1024*1024,env:{...process.env,BRAIN_VIEWER:'0'}})
  try {
    const created=cli(['new','site','--static','--json'],root)
    assert.equal(created.status,0,created.stdout+created.stderr)
    const app=join(root,'site'),folder=join(app,'comp/reload-proof')
    mkdirSync(folder,{recursive:true})
    writeFileSync(join(folder,'reloader.brain'),'<script>mounted(() => location.reload())</script><p>Reload proof</p>')
    writeFileSync(join(folder,'ready.brain'),'<p>Ready proof</p>')
    writeFileSync(join(folder,'probe.test.brain'),`<script>
import Reloader from './reloader.brain'
import Ready from './ready.brain'
test('navigation failure belongs here',async()=>{await mount(Reloader,{});expect(text('p')).toBe('Reload proof')})
test('next assertion still runs',async()=>{await mount(Ready,{});expect(text('p')).toBe('Ready proof')})
</script>`)
    writeFileSync(join(folder,'z-next.test.brain'),`<script>
import Ready from './ready.brain'
test('another file still runs',async()=>{await mount(Ready,{});expect(text('p')).toBe('Ready proof')})
</script>`)
    for (const heap of [false,true]) {
      const result=cli(['test','comp/reload-proof',...(heap?['--heap']:[]),'--json'],app)
      assert.notEqual(result.status,0,'navigation must never silently pass')
      const report=JSON.parse(result.stdout)
      const files=Array.isArray(report)?report:report.files
      assert.ok(Array.isArray(files),result.stdout+result.stderr)
      const file=files.find(file=>file.file.endsWith('/probe.test.brain'))
      const failed=file.tests.find(row=>row.name==='navigation failure belongs here')
      assert.equal(failed.ok,false)
      assert.match(failed.error,/navigation/i)
      assert.match(failed.error,/probe\.test\.brain/)
      assert.match(failed.error,/navigation failure belongs here/)
      assert.ok(!failed.error.includes('brain_click')&&!failed.error.includes('brain_eval'))
      assert.equal(file.tests.find(row=>row.name==='next assertion still runs').ok,true)
      assert.equal(files.find(file=>file.file.endsWith('/z-next.test.brain')).tests[0].ok,true)
    }
  } finally {rmSync(root,{recursive:true,force:true})}
})
