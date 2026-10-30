// Publication backstop: public literal Rust embeddings must exist in a clean source checkout.
import {execFileSync} from 'node:child_process'
import {readFileSync,lstatSync,existsSync} from 'node:fs'
import {resolve,relative,dirname,sep} from 'node:path'
import {pathToFileURL} from 'node:url'
export function checkSourceInputs(root){
 root=resolve(root)
 const listed=execFileSync('git',['ls-files','--cached','--others','--exclude-standard','-z'],{cwd:root,encoding:'utf8',timeout:30000,maxBuffer:4*1024*1024}).split('\0').filter(Boolean)
 const files=new Set(listed),findings=[];let checked=0
 for(const source of files){
  if(!source.endsWith('.rs')||!existsSync(resolve(root,source)))continue
  const text=readFileSync(resolve(root,source),'utf8')
  if(Buffer.byteLength(text)>8*1024*1024)throw Error('Oversized source input inventory')
  for(const match of text.matchAll(/\binclude_(?:str|bytes)!\(\s*"([^"\r\n]+)"\s*\)/g)){
   const target=resolve(root,dirname(source),match[1]);checked++
   const location={source,line:text.slice(0,match.index).split('\n').length,column:match.index-text.lastIndexOf('\n',match.index),columnUnit:'utf16'}
   if(target!==root&&!target.startsWith(root+sep)){findings.push({...location,rule:'embedded-input-outside-project',fix:'Move the required public source into the repository and use an app-relative embedding path'});continue}
   const input=relative(root,target).split(sep).join('/')
   if(!existsSync(target)){findings.push({...location,input,rule:'embedded-input-missing',fix:'Restore the public template or correct the literal relative path before compilation'});continue}
   if(!lstatSync(target).isFile()){findings.push({...location,input,rule:'embedded-input-not-regular',fix:'Use a regular repository file instead of a symlink or special file'});continue}
   if(!files.has(input))findings.push({...location,input,rule:'embedded-input-excluded-from-source',fix:'Review this public input and include it in source control with a scoped ignore exception; private configuration must stay excluded'})
  }
 }
 return {schemaVersion:1,ok:findings.length===0,literalEmbeddings:checked,findings,coverage:'Literal include_str/include_bytes inputs only; generated or computed macro paths still require a clean compilation.'}
}
if(process.argv[1]&&import.meta.url===pathToFileURL(resolve(process.argv[1])).href){
 try{const report=checkSourceInputs(process.cwd());console.log(JSON.stringify(report,null,2));process.exitCode=report.ok?0:1}catch{console.error('Cannot verify public source inputs');process.exitCode=2}
}
