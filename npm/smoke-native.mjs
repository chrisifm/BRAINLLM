#!/usr/bin/env node
// Runs the actual target binary before packing. No registry credentials, publication or lasting services.
import {mkdtempSync,rmSync,readFileSync,existsSync} from 'node:fs'
import {tmpdir} from 'node:os'
import {join,resolve} from 'node:path'
import {run} from '../scripts/lib/jobs.mjs'
import {fileURLToPath} from 'node:url'
const expected=/\[workspace\.package\][^[]*?\nversion\s*=\s*"([^"]+)"/.exec(readFileSync(fileURLToPath(new URL('../Cargo.toml',import.meta.url)),'utf8'))?.[1]
if(!expected)throw Error('Missing source release version')
const binary=resolve(process.argv[2]||'')
if(!process.argv[2]||!existsSync(binary))throw Error('Pass the built native binary path')
const directory=mkdtempSync(join(tmpdir(),'brain-native-smoke-'))
try {
 const execute=(args,cwd=directory)=>run(binary,args,{cwd,timeout:120000,log:join(directory,'command.log'),env:{...process.env,BRAIN_BACKEND_URL:'',BRAIN_NO_DELEGATE:'1'}})
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
} finally {rmSync(directory,{recursive:true,force:true})}
