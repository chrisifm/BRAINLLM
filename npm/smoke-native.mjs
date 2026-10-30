#!/usr/bin/env node
// Runs the actual target binary before packing. No registry credentials, publication or lasting services.
import {mkdtempSync,rmSync,readFileSync,existsSync,writeFileSync,mkdirSync} from 'node:fs'
import {tmpdir} from 'node:os'
import {join,resolve,dirname} from 'node:path'
import {run} from '../scripts/lib/jobs.mjs'
import {createHash} from 'node:crypto'
import {fileURLToPath,pathToFileURL} from 'node:url'
// Node 22 public error identifiers: https://nodejs.org/docs/latest-v22.x/api/errors.html
const NODE_CODES=new Set(['ERR_INVALID_FILE_URL_PATH','ERR_INVALID_FILE_URL_HOST','ERR_UNSUPPORTED_ESM_URL_SCHEME','ERR_MODULE_NOT_FOUND','ERR_INVALID_MODULE_SPECIFIER','ERR_INVALID_URL','ERR_INVALID_URL_SCHEME','ERR_UNKNOWN_FILE_EXTENSION','ERR_PACKAGE_PATH_NOT_EXPORTED','ERR_PACKAGE_IMPORT_NOT_DEFINED','ERR_INVALID_PACKAGE_CONFIG','ERR_INVALID_PACKAGE_TARGET','ERR_REQUIRE_ESM','ERR_DLOPEN_FAILED','ERR_IMPORT_ATTRIBUTE_MISSING','MODULE_NOT_FOUND','ENOENT','EACCES','EPERM','ENOTDIR','EISDIR'])
const ERROR_NAMES=new Set(['Error','TypeError','SyntaxError','ReferenceError','RangeError','AggregateError','URIError','EvalError'])
// Only allowlisted metadata leaves the private child log; never error messages or source excerpts.
export function failureReport(phase,bytes,error=null) {
 const buffer=Buffer.from(bytes),bounded=buffer.subarray(0,262144),codes=new Set(),nodeCodes=new Set(),errorNames=new Set()
 const raw=bounded.toString('utf8')
 for(const match of raw.matchAll(/\b(?:ERR_[A-Z0-9_]+|MODULE_NOT_FOUND|ENOENT|EACCES|EPERM|ENOTDIR|EISDIR)\b/g))if(NODE_CODES.has(match[0]))nodeCodes.add(match[0])
 for(const match of raw.matchAll(/\b(Error|TypeError|SyntaxError|ReferenceError|RangeError|AggregateError|URIError|EvalError)(?:\s*\[[A-Z0-9_]+\])?:/g))errorNames.add(match[1])
 // Only the maintenance runner's fixed suffix is parsed; command paths/messages never leave this function.
 const message=typeof error?.message==='string'?error.message:''
 const exit=/: exit ([0-9]{1,3}); inspect the private local log\.$/.exec(message)
 const exitCode=exit&&Number(exit[1])<=255?Number(exit[1]):null
 const processFailure=exitCode!==null?'exit':message.endsWith(': deadline exceeded; inspect the private local log.')?'deadline':message.endsWith(': cancelled; inspect the private local log.')?'cancelled':message.endsWith(': could not start process; inspect the private local log.')?'spawn':'unknown'
 let visited=0
 const visit=(value,depth)=>{
  if(++visited>512||depth>8||!value||typeof value!=='object')return
  if(typeof value.code==='string'&&/^B[0-9]{4}$/.test(value.code))codes.add(value.code)
  if(NODE_CODES.has(value.code))nodeCodes.add(value.code)
  if(ERROR_NAMES.has(value.name))errorNames.add(value.name)
  if(Array.isArray(value)){for(const item of value.slice(0,64))visit(item,depth+1)}
  else for(const key of ['diags','diagnostics','errors','error','results'])visit(value[key],depth+1)
 }
 for(const line of bounded.toString('utf8').split('\n').slice(0,256)){
  if(!line.trimStart().startsWith('{')&&!line.trimStart().startsWith('['))continue
  try{visit(JSON.parse(line),0)}catch{}
 }
 return {schemaVersion:1,ok:false,phase:['version','new','check','build'].includes(phase)?phase:'unknown',failure:'native-smoke-failed',diagnosticCodes:[...codes].sort().slice(0,32),observedNodeErrorCodes:[...nodeCodes].sort(),observedErrorNames:[...errorNames].sort(),errorEvidence:'allowlisted-log-tokens',exitCode,processFailure,outputSha256:createHash('sha256').update(bounded).digest('hex'),outputBytesInspected:bounded.length,outputInspectionTruncated:buffer.length>bounded.length||buffer.length===262144}
}
export async function main(args=process.argv.slice(2)) {
const expected=/\[workspace\.package\][^[]*?\nversion\s*=\s*"([^"]+)"/.exec(readFileSync(fileURLToPath(new URL('../Cargo.toml',import.meta.url)),'utf8'))?.[1]
if(!expected)throw Error('Missing source release version')
const binary=resolve(args[0]||'')
if(!args[0]||!existsSync(binary))throw Error('Pass the built native binary path')
const diagnostic=args.length===3&&args[1]==='--diagnostic'?resolve(args[2]):null
if(args.length!==1&&!diagnostic)throw Error('Expected native binary and optional --diagnostic path')
const directory=mkdtempSync(join(tmpdir(),'brain-native-smoke-'))
let phase='version'
try {
 const execute=(args,cwd=directory)=>{phase=args[0]==='--version'?'version':args[0];return run(binary,args,{cwd,timeout:120000,maxBytes:262144,log:join(directory,'command.log'),env:{...process.env,BRAIN_BACKEND_URL:'',BRAIN_NO_DELEGATE:'1'}})}
 await execute(['--version','--json'])
 const version=JSON.parse(readFileSync(join(directory,'command.log'),'utf8').trim().split('\n').findLast(line=>line.startsWith('{'))||'null')
 if(version?.schemaVersion!==1||version.version!==expected)throw Error('Native release binary does not match its source version')
 await execute(['new','native-release-app'])
 const app=join(directory,'native-release-app')
 if(JSON.parse(readFileSync(join(app,'package.json'),'utf8')).devDependencies?.['@christianirack/brainllm']!==expected)throw Error('Native scaffold must pin this exact release')
 await execute(['check','--json'],app)
 await execute(['build','--json'],app)
 const line=readFileSync(join(directory,'command.log'),'utf8').trim().split('\n').findLast(value=>value.startsWith('{'))
 if(!line)throw Error('Native build returned no structured result')
 const result=JSON.parse(line)
 if(result.schemaVersion!==1||typeof result.build!=='string'||!Array.isArray(result.ledger)||!existsSync(join(app,'dist/index.html')))throw Error('Native build did not produce a complete app')
 console.log(JSON.stringify({schemaVersion:1,ok:true,platform:process.platform,arch:process.arch,checks:['version','new','check','build']}))
} catch(error) {
 let output=Buffer.alloc(0)
 try{output=readFileSync(join(directory,'command.log')).subarray(0,262144)}catch{}
 const report=failureReport(phase,output,error)
 console.error(JSON.stringify(report))
 if(diagnostic){mkdirSync(dirname(diagnostic),{recursive:true});writeFileSync(diagnostic,JSON.stringify(report,null,2)+'\n')}
 process.exitCode=1
} finally {rmSync(directory,{recursive:true,force:true})}
}
if(process.argv[1]&&import.meta.url===pathToFileURL(resolve(process.argv[1])).href)main().catch(()=>{console.error(JSON.stringify({schemaVersion:1,ok:false,phase:'setup',failure:'invalid-smoke-input'}));process.exitCode=1})
