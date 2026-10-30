import test from 'node:test'
import assert from 'node:assert/strict'
import {spawnSync} from 'node:child_process'
import {mkdtempSync,readFileSync,rmSync} from 'node:fs'
import {join} from 'node:path'
import {tmpdir} from 'node:os'
import {failureReport} from '../smoke-native.mjs'

test('native failure diagnostics expose phases and codes without private log fields',()=>{
 const secret='PRIVATE_SMOKE_SENTINEL'
 const output=JSON.stringify({diags:[{code:'B0720',msg:secret,file:secret,source:secret},{code:'B0123'},{code:secret}],env:secret,error:{code:'B0720',message:secret}})
 const report=failureReport('check',Buffer.from(output))
 assert.equal(report.phase,'check')
 assert.deepEqual(report.diagnosticCodes,['B0123','B0720'])
 assert.ok(!JSON.stringify(report).includes(secret))
 assert.equal(failureReport(secret,Buffer.from(secret)).phase,'unknown')
 const large=failureReport('build',Buffer.alloc(262145,120))
 assert.equal(large.outputBytesInspected,262144)
 assert.equal(large.outputInspectionTruncated,true)
})
test('actual failing child keeps the smoke gate failed and writes its safe diagnostic before cleanup',()=>{
 const root=mkdtempSync(join(tmpdir(),'brain-native-diagnostic-test-'))
 try{
  const file=join(root,'diagnostic.json')
  // Node is intentionally not a Brain compiler: --json must fail at the version phase.
  const result=spawnSync(process.execPath,['npm/smoke-native.mjs',process.execPath,'--diagnostic',file],{encoding:'utf8',timeout:10000})
  assert.equal(result.status,1)
  const report=JSON.parse(readFileSync(file,'utf8'))
  assert.equal(report.phase,'version')
  assert.equal(report.failure,'native-smoke-failed')
  assert.deepEqual(report.diagnosticCodes,[])
  assert.equal(report.exitCode,9)
  assert.equal(report.processFailure,'exit')
  assert.match(result.stderr,/"phase":"version"/)
  assert.ok(!result.stderr.includes(process.execPath))
  assert.ok(!result.stderr.includes('bad option'))
 }finally{rmSync(root,{recursive:true,force:true})}
})

test('native diagnostics classify allowlisted Node failures without copying paths or messages',()=>{
 const secret='PRIVATE_NODE_FAILURE_SENTINEL'
 const raw=`TypeError [ERR_INVALID_FILE_URL_PATH]: ${secret}\ncode: 'ERR_MODULE_NOT_FOUND'\nERR_UNREVIEWED_PRIVATE_CODE_${secret}\n`+JSON.stringify({error:{name:'SyntaxError',code:'ERR_UNSUPPORTED_ESM_URL_SCHEME',message:secret},diagnostics:[{code:'B0720',msg:secret}]})
 const report=failureReport('build',Buffer.from(raw),Error('private-executable: exit 2; inspect the private local log.'))
 assert.deepEqual(report.observedNodeErrorCodes,['ERR_INVALID_FILE_URL_PATH','ERR_MODULE_NOT_FOUND','ERR_UNSUPPORTED_ESM_URL_SCHEME'])
 assert.deepEqual(report.observedErrorNames,['SyntaxError','TypeError'])
 assert.deepEqual(report.diagnosticCodes,['B0720'])
 assert.equal(report.exitCode,2)
 assert.equal(report.errorEvidence,'allowlisted-log-tokens')
 assert.ok(!JSON.stringify(report).includes(secret))
 assert.equal(failureReport('build',Buffer.from(''),Error('unstructured failure text')).exitCode,null)
 assert.equal(failureReport('build',Buffer.from(''),Error('native: deadline exceeded; inspect the private local log.')).processFailure,'deadline')
})
