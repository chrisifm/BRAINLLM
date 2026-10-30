import test from 'node:test'
import assert from 'node:assert/strict'
import { execFileSync, spawnSync } from 'node:child_process'
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { createServer } from 'node:http'
import { anonymousEnvironment, options, npmCli, childSession, httpProbe, publicFailure, logSummary, doctorSummary, browserExecutable, browserFailure } from '../smoke-registry.mjs'

test('registry smoke requires an exact version and a bounded deadline without running npm', () => {
  assert.equal(options(['--help']).help, true)
  assert.deepEqual(options(['--version', '0.4.0', '--no-dev']), { version: '0.4.0', timeout: 900000, dev: false })
  for (const args of [[], ['--version', 'latest'], ['--version', '^0.4.0'], ['--version', 'https://example.test/package'], ['--version', '0.4.0', '--timeout-ms', '0'], ['--version', '0.4.0', '--timeout-ms', '900001'], ['--version', '0.4.0', '--binary', '/local/brain']]) assert.throws(() => options(args))
  const help = execFileSync(process.execPath, ['npm/smoke-registry.mjs', '--help'], { encoding: 'utf8', timeout: 10000 })
  assert.match(help, /anonymous|anonymously/)
  assert.equal(spawnSync(process.execPath, ['npm/smoke-registry.mjs', '--version', 'latest'], { timeout: 10000, stdio: 'pipe' }).status, 2)
})
test('anonymous environment drops every credential, local binary override and inherited npm config', () => {
  const env = anonymousEnvironment('/owned/scratch', { PATH: '/usr/bin', SystemRoot: 'C:\\Windows', HOME: '/private', NPM_TOKEN: 'not-a-real-token', NODE_AUTH_TOKEN: 'not-a-real-token', NPM_CONFIG_USERCONFIG: '/private/npmrc', BRAIN_BINARY: '/local/binary', BRAIN_NO_DELEGATE: '1', OPENAI_API_KEY: 'not-a-real-key', NODE_OPTIONS: '--require /untrusted.js', GITHUB_TOKEN: 'not-a-real-token', HTTPS_PROXY: 'http://proxy.test', BRAIN_CHROME: '/inherited/browser'  })
  assert.equal(env.PATH, '/usr/bin')
  assert.equal(env.NPM_CONFIG_REGISTRY, 'https://registry.npmjs.org/')
  assert.equal(env.HOME,join('/owned/scratch','home'))
  assert.equal(env.USERPROFILE,env.HOME)
  for (const name of ['NPM_TOKEN', 'NODE_AUTH_TOKEN', 'BRAIN_BINARY', 'BRAIN_NO_DELEGATE', 'OPENAI_API_KEY', 'NODE_OPTIONS', 'GITHUB_TOKEN', 'HTTPS_PROXY', 'BRAIN_CHROME']) assert.equal(env[name], undefined, name)
  assert.ok(!Object.values(env).some(value => value.includes('not-a-real-')))
})
test('npm discovery uses its installed JavaScript CLI without invoking a platform shell', () => {
  assert.match(npmCli(), /npm-cli\.js$/)
})


test('owned command deadlines terminate the child and release lifecycle bookkeeping', { timeout: 10000 }, async () => {
  const root = mkdtempSync(join(tmpdir(), 'brain-registry-lifecycle-')), active = new Set()
  let session
  try {
    session = childSession(['-e', 'setInterval(()=>{},1000)'], { cwd: root, env: anonymousEnvironment(root), log: join(root, 'phase.log'), timeout: 100, phase: 'local-deadline', active })
    await assert.rejects(session.done, error => error.detail.phase === 'local-deadline' && error.detail.reason === 'deadline')
    assert.equal(active.size, 0)
    assert.equal(session.exited, true)
  } finally { session?.stop(); await session?.done.catch(() => {}); rmSync(root, { recursive: true, force: true }) }
})
test('temporary dev HTTP probe checks an actual loopback HTML response and rejects errors', { timeout: 10000 }, async () => {
  const server = createServer((req, res) => { res.writeHead(['/','/client'].includes(req.url) ? 200 : 503, { 'content-type': 'text/html' }); res.end(req.url==='/client'?'<html><head></head><body><script type="module" src="/client.js"></script></body></html>':'<main>Owned smoke fixture</main>') })
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve))
  try {
    const url = `http://127.0.0.1:${server.address().port}`
    assert.equal(await httpProbe(url), true)
    assert.equal(await httpProbe(url+'/client'), true)
    assert.equal(await httpProbe(url + '/unavailable'), false)
  } finally { await new Promise(resolve => server.close(resolve)) }
})

test('registry failures preserve the exact phase and allowlisted npm cause without private values',()=>{
 const sentinel='PRIVATE_REGISTRY_FAILURE_SENTINEL'
 const error=Error(sentinel)
 error.detail={phase:'registry-brainllm-4',reason:'exact-version-unavailable',lastAttemptReason:'nonzero-exit',retryAttempts:5,exitCode:1,npmCodes:['E404',sentinel],diagnosticCodes:['B0720',sentinel],arbitrary:sentinel}
 const report=publicFailure(error)
 assert.equal(report.phase,'registry-brainllm-4');assert.equal(report.reason,'exact-version-unavailable');assert.equal(report.lastAttemptReason,'nonzero-exit')
 assert.deepEqual(report.npmCodes,['E404']);assert.deepEqual(report.diagnosticCodes,['B0720']);assert.equal(report.retryAttempts,5)
 assert.ok(!JSON.stringify(report).includes(sentinel))
 assert.equal(publicFailure(Error('Initializer did not install the exact version'),'validate-install-brainllm').reason,'installed-version-mismatch')
 assert.equal(logSummary(Buffer.from('Connected Viewer capability probe failed')).browserBridgeFailed,true)
 assert.equal(logSummary(Buffer.from('Connected Viewer capability probe failed')).browserProfileRequired,false)
 assert.equal(options(['--version','0.4.0','--keep-failure-logs']).keepFailureLogs,true)
})
test('an actual failing npm-like child retains its safe code and releases owned resources',async()=>{
 const root=mkdtempSync(join(tmpdir(),'brain-registry-error-test-')),active=new Set()
 try{
  const secret='PRIVATE_CHILD_FAILURE_SENTINEL'
  const job=childSession(['-e',`console.error('npm error code E404');console.error(${JSON.stringify(secret)});process.exit(1)`],{cwd:root,env:anonymousEnvironment(root),log:join(root,'phase.log'),timeout:5000,phase:'registry-brainllm-0',active})
  await assert.rejects(job.done,error=>{
   const report=publicFailure(error)
   assert.equal(report.phase,'registry-brainllm-0');assert.equal(report.reason,'nonzero-exit');assert.equal(report.exitCode,1)
   assert.deepEqual(report.npmCodes,['E404']);assert.ok(!JSON.stringify(report).includes(secret));return true
  })
  assert.equal(active.size,0);assert.equal(job.exited,true)
 }finally{rmSync(root,{recursive:true,force:true})}
})

test('doctor summaries retain AX, heap and capability failures that assertion counts alone omit',()=>{
 const secret='PRIVATE_DOCTOR_VALUE_SENTINEL'
 const report={ok:false,check:{errors:0},test:{passed:2,failed:0,a11y:1,errors:[{msg:secret}],files:[{file:secret,engine:'headless-cdp',tests:[{ok:true,name:secret}],a11y:[{code:'B0406',msg:secret}],leaks:[{code:'B0601',msg:secret}],errors:[],coverage:{accessibility:'Chrome Accessibility.getFullAXTree',heap:'forced-GC inspector'},capability:{forcedGC:false,secret}}]},heap:{leaks:[{code:'B0601',msg:secret}]},build:{ok:true},seo:{ok:false,minScore:80,pages:[{score:75,url:secret}]},advice:[{code:'B0733',level:'error',msg:secret}]}
 const summary=doctorSummary(report)
 assert.equal(summary.failedTests,0);assert.equal(summary.passedTests,2);assert.equal(summary.accessibilityFindings,1);assert.equal(summary.heapLeaks,1)
 assert.equal(summary.testErrorPresent,false);assert.equal(summary.buildOk,true);assert.equal(summary.seoOk,false);assert.equal(summary.seoBelowThresholdPages,1)
 assert.deepEqual(summary.diagnosticCodes,['B0406','B0601','B0733']);assert.equal(summary.files[0].coverage.heap,'forced-GC inspector');assert.deepEqual(summary.files[0].capability,{forcedGC:false})
 assert.ok(!JSON.stringify(summary).includes(secret))
})
test('actual doctor command failure projects its private report without leaking messages or source locations',async()=>{
 const root=mkdtempSync(join(tmpdir(),'brain-registry-doctor-summary-')),active=new Set()
 try{
  const secret='PRIVATE_DOCTOR_CHILD_SENTINEL',payload={ok:false,check:{errors:0,diags:[]},test:{passed:2,failed:0,a11y:0,files:[{engine:'headless-cdp',tests:[],leaks:[{code:'B0601',msg:secret}],coverage:{heap:'forced-GC inspector'}}]},heap:{leaks:[{code:'B0601',file:secret}]},build:{ok:true},seo:{ok:true}}
  const child=childSession(['-e',`console.log(${JSON.stringify(JSON.stringify(payload))});process.exit(1)`],{cwd:root,env:anonymousEnvironment(root),log:join(root,'doctor.log'),timeout:5000,phase:'doctor-brainllm',active})
  await assert.rejects(child.done,error=>{
   const summary=publicFailure(error)
   assert.equal(summary.phase,'doctor-brainllm');assert.equal(summary.reason,'nonzero-exit');assert.equal(summary.doctor.heapLeaks,1)
   assert.deepEqual(summary.diagnosticCodes,['B0601']);assert.ok(!JSON.stringify(summary).includes(secret));return true
  })
  assert.equal(active.size,0)
 }finally{rmSync(root,{recursive:true,force:true})}
})

test('browser selection is explicit, executable and never inherited as a compiler override',()=>{
 assert.equal(browserExecutable(process.execPath),process.execPath)
 assert.equal(options(['--version','0.4.0','--browser',process.execPath]).browser,process.execPath)
 assert.throws(()=>browserExecutable(tmpdir()),/existing executable/)
 assert.throws(()=>browserExecutable('missing-owned-browser-executable'),/existing executable/)
 assert.throws(()=>options(['--version','0.4.0','--browser']),/existing executable/)
 const failure=Error('private failure');failure.detail={phase:'doctor-brainllm',reason:'nonzero-exit',browserConfigured:true}
 const report=publicFailure(failure)
 assert.equal(report.browserConfigured,true);assert.equal(report.browserSelection,'explicit')
 assert.ok(!JSON.stringify(report).includes(process.execPath))
})
test('browser failures identify known CDP causes and methods without copying raw error text',()=>{
 const secret='PRIVATE_BROWSER_ERROR_SENTINEL'
 const cases=[
  ['pipe CDP: Chrome closed the pipe','cdp-pipe-closed',null],
  ['CDP transport write failed: CDP write deadline exceeded; the owned browser session was stopped','cdp-write-timeout',null],
  [`timed out: no answer to Runtime.evaluate in 30 s (${secret})`,'cdp-response-timeout','Runtime.evaluate'],
  [`CDP Page.navigate: ${secret}`,'cdp-protocol-error','Page.navigate'],
  ['Running as root without --no-sandbox is not supported','browser-sandbox-unavailable',null],
  ['Permission denied (os error 13)','browser-permission-denied',null]
 ]
 for(const [input,reason,method] of cases){const actual=browserFailure(input);assert.deepEqual(actual,{reason,method});assert.ok(!JSON.stringify(actual).includes(secret))}
 assert.equal(browserFailure(secret).reason,'unclassified-test-error')
 assert.equal(browserFailure('').reason,null)
 const report=doctorSummary({ok:false,test:{error:`pipe CDP: Chrome closed the pipe ${secret}`},seo:{ok:true,site:[{code:'B1808',level:'info',msg:secret}]}})
 assert.equal(report.testErrorReason,'cdp-pipe-closed');assert.equal(report.testErrorPresent,true);assert.ok(!JSON.stringify(report).includes(secret))
})
