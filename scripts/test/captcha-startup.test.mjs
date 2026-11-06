import test from 'node:test'
import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join, dirname } from 'node:path'

const binary = process.env.BRAIN_BIN ?? new URL('../../target/debug/brain', import.meta.url).pathname
test('production rejects a proven CAPTCHA test site key before database configuration', { timeout: 60000 }, () => {
  const root = mkdtempSync(join(tmpdir(), 'brain-captcha-preflight-'))
  const write = (name, value) => { const path = join(root, name); mkdirSync(dirname(path), { recursive: true }); writeFileSync(path, value) }
  const environment = { ...process.env }
  delete environment.DATABASE_URL
  delete environment.BRAIN_TEST_CAPTCHA_SITE_SECRET
  try {
    write('brain.json', '{}')
    write('app.brain', '<html><head></head><body><slot /></body></html>')
    write('server/auth/captcha.jsbrain', "<meta server>\nexport default { verify_url: 'https://challenges.example.com/verify', secret_env: 'BRAIN_TEST_CAPTCHA_SITE_SECRET', protect: ['signup'] }")
    write('.brain_env', 'BRAIN_TEST_CAPTCHA_SITE_SECRET=synthetic-provider-issued-secret\n')
    for (const [source, refuses] of [
      ['<div data-sitekey="1x00000000000000000000AA"></div>', true],
      ['<div data-sitekey="synthetic-provider-issued-site-key"></div>', false],
      ["<script>mounted(() => turnstile.render('#widget', { sitekey: '1x00000000000000000000AA' }))</script><div id=\"widget\"></div>", true],
      ["<script>mounted(() => turnstile.render('#widget', { sitekey: '1x00000000000000000000AA', sitekey: 'synthetic-provider-issued-site-key' }))</script><div id=\"widget\"></div>", false],
      ["<script>mounted(() => turnstile.render('#widget', { sitekey: '1x00000000000000000000AA', ...unknownOptions }))</script><div id=\"widget\"></div>", false]
    ]) {
      write('views/index.brain', '<meta view>\n' + source)
      const result = spawnSync(binary, ['backend', 'serve', '--env', 'production'], { cwd: root, env: environment, encoding: 'utf8', timeout: 10000, maxBuffer: 65536 })
      assert.ifError(result.error)
      assert.equal(result.signal, null)
      assert.notEqual(result.status, 0)
      const output = result.stdout + result.stderr
      if (refuses) {
        assert.match(output, /B0790:.*test site key/)
        assert.doesNotMatch(output, /DATABASE_URL is not set/)
        assert.doesNotMatch(output, /1x00000000000000000000AA|synthetic-provider-issued-secret/)
      } else {
        assert.match(output, /B0762: DATABASE_URL is not set/, 'unknown or overwritten bindings must reach the independent missing database diagnostic')
        assert.doesNotMatch(output, /B0790/)
      }
    }
  } finally { rmSync(root, { recursive: true, force: true }) }
})
test('check warns about a declared missing CAPTCHA secret without printing any environment value', {timeout:30000},()=>{
 const root=mkdtempSync(join(tmpdir(),'brain-captcha-missing-')),environment={...process.env,BRAIN_TEST_CAPTCHA_SITE_SECRET:''}
 try{
  mkdirSync(join(root,'server/auth'),{recursive:true});mkdirSync(join(root,'views'))
  writeFileSync(join(root,'brain.json'),'{}');writeFileSync(join(root,'tokens.cssbrain'),'')
  writeFileSync(join(root,'app.brain'),'<html><head></head><body><slot /></body></html>')
  writeFileSync(join(root,'views/home.brain'),'<h1>Public example</h1>')
  writeFileSync(join(root,'server/auth/captcha.jsbrain'),"<meta server>\nexport default { verify_url: 'https://challenges.example.com/verify', secret_env: 'BRAIN_TEST_CAPTCHA_SITE_SECRET', protect: ['signup','login'] }")
  writeFileSync(join(root,'.brain_env'),'UNRELATED_PRIVATE_VALUE=synthetic-private-value\n')
  const result=spawnSync(binary,['check','--json'],{cwd:root,env:environment,encoding:'utf8',timeout:10000,maxBuffer:65536});assert.ifError(result.error)
  const warnings=JSON.parse(result.stdout).filter(row=>row.code==='B0790');assert.ok(warnings.length>0,result.stdout+result.stderr)
  assert.ok(warnings.some(row=>row.file==='server/auth/captcha.jsbrain'&&row.level==='warning'&&/BRAIN_TEST_CAPTCHA_SITE_SECRET/.test(row.msg)&&/secret/.test(row.fix)))
  assert.doesNotMatch(result.stdout+result.stderr,/synthetic-private-value/)
 }finally{rmSync(root,{recursive:true,force:true})}
})
