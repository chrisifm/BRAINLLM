import test from 'node:test'
import assert from 'node:assert/strict'
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, rmSync, symlinkSync, cpSync, copyFileSync } from 'node:fs'
import { pathToFileURL } from 'node:url'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { execFileSync } from 'node:child_process'
import { anonymousEnvironment, npmCli } from '../smoke-registry.mjs'
import { generate, readBundle, writeNotices, FILES, hash, pinnedText, lockDigest, failureDiagnostic } from '../cargo-notices.mjs'
const target = 'aarch64-apple-darwin'
function fixture() {
  const root = mkdtempSync(join(tmpdir(), 'brain-notices-test-'))
  const pkg = (name, version, license = 'MIT') => {
    const dir = join(root, name); mkdirSync(dir)
    writeFileSync(join(dir, 'Cargo.toml'), '# public fixture manifest\n')
    return {id:name,name,version,license,source:'registry+https://github.com/rust-lang/crates.io-index',manifest_path:join(dir,'Cargo.toml'),license_file:null}
  }
  const cli = pkg('brain-cli','0.4.0'), alpha=pkg('alpha','1.0.0'), build=pkg('build-helper','2.0.0'), dev=pkg('dev-only','3.0.0')
  for (const entry of [alpha,build]) writeFileSync(join(root,entry.name,'LICENSE-MIT'), `Public license text for ${entry.name}; copyright fixture authors.\n`)
  mkdirSync(join(root,'alpha','vendor')); writeFileSync(join(root,'alpha','vendor','NOTICE'), 'Vendored attribution must survive.\n')
  const edge = (pkg,kind=null)=>({pkg,dep_kinds:[{kind,target:null}]})
  const metadata={workspace_root:root,workspace_members:[cli.id],packages:[cli,alpha,build,dev],resolve:{nodes:[{id:cli.id,deps:[edge(alpha.id),edge(build.id,'build'),edge(dev.id,'dev')]},{id:alpha.id,deps:[]},{id:build.id,deps:[]},{id:dev.id,deps:[]}]}}
  const args={metadata,binary:Buffer.from('fixture bytes, not an executed release'),target,cargoLock:Buffer.from('locked fixture'),rustStdCopyright:Buffer.from('<html>Rust Standard Library: public fixture copyright.</html>'),rustToolchainCopyright:Buffer.from('<html>Rust Toolchain: public fixture copyright.</html>'),rustVersion:'rustc 1.96.0 (fixture)'}
  return {root,args,alpha,close:()=>rmSync(root,{recursive:true,force:true})}
}
test('Cargo notices preserve full nested attributions, exclude dev-only edges and omit machine paths deterministically',()=>{
  const f=fixture()
  try {
    const first=generate(f.args), metadata=JSON.parse(first.manifest)
    assert.deepEqual(metadata.packages.map(p=>p.name),['alpha','build-helper'])
    assert.match(first.notice,/Vendored attribution must survive/)
    assert.match(first.notice,/copyright fixture authors/)
    assert.ok(!first.notice.includes(f.root)&&!first.manifest.includes(f.root))
    f.args.metadata.packages.reverse();f.args.metadata.resolve.nodes.reverse()
    const second=generate(f.args)
    assert.equal(first.notice,second.notice);assert.equal(first.manifest,second.manifest)
  } finally {f.close()}
})
test('missing, escaping and symlinked public license inputs fail closed instead of guessing a license',()=>{
  const f=fixture()
  try {
    rmSync(join(f.root,'alpha','LICENSE-MIT'));rmSync(join(f.root,'alpha','vendor'),{recursive:true})
    assert.throws(()=>generate(f.args),/Missing public LICENSE\/NOTICE files for alpha@1.0.0/)
    writeFileSync(join(f.root,'outside-license'),'outside public fixture')
    f.alpha.license_file='../outside-license'
    assert.throws(()=>generate(f.args),/Escaping license-file/)
    f.alpha.license_file=null
    if(process.platform!=='win32') {
      symlinkSync(join(f.root,'outside-license'),join(f.root,'alpha','LICENSE'))
      assert.throws(()=>generate(f.args),/Missing public LICENSE\/NOTICE/)
    }
  } finally {f.close()}
})
test('native bundle binding rejects changed binary, lock, target and notice text before packaging',()=>{
  const f=fixture()
  try {
    const result=generate(f.args),binary=join(f.root,'brain');writeFileSync(binary,f.args.binary)
    for(const [index,value] of [result.notice,result.manifest,result.rustStd,result.rustToolchain].entries())writeFileSync(join(f.root,FILES[index]),value)
    assert.equal(readBundle(binary,target,'0.4.0',f.args.cargoLock).notice.toString(),result.notice)
    assert.throws(()=>readBundle(binary,'x86_64-unknown-linux-gnu','0.4.0',f.args.cargoLock),/do not match/)
    assert.throws(()=>readBundle(binary,target,'0.4.0',Buffer.from('changed lock')),/do not match/)
    writeFileSync(binary,'changed binary');assert.throws(()=>readBundle(binary,target,'0.4.0',f.args.cargoLock),/do not match/)
    writeFileSync(binary,f.args.binary);writeFileSync(join(f.root,FILES[0]),'changed legal text');assert.throws(()=>readBundle(binary,target,'0.4.0',f.args.cargoLock),/do not match/)
  } finally {f.close()}
})
test('pinned upstream supplemental texts are checked by recorded SHA-256 and public source URLs',()=>{
  const root=new URL('../licenses/',import.meta.url),data=JSON.parse(readFileSync(new URL('supplements.json',root),'utf8'))
  for(const entry of Object.values(data.packages)) {
    assert.match(entry.vcsCommit,/^[a-f0-9]{40}$/)
    for(const file of entry.files) {
      assert.match(file.url,/^https:\/\/raw\.githubusercontent\.com\//)
      const bytes=readFileSync(new URL(file.file,root))
      assert.ok(bytes.length>100)
      assert.equal(hash(pinnedText(bytes,file.sha256)),file.sha256)
    }
  }
})


test('native npm payload includes full Cargo/std/public notices through the actual pack writer', {timeout:30000},()=>{
  const f=fixture()
  try {
    const result=generate(f.args),directory=join(f.root,'platform');mkdirSync(directory)
    const files=writeNotices(directory,'Public web-features license fixture.\n',{notice:Buffer.from(result.notice),metadata:Buffer.from(result.manifest),rustStd:result.rustStd,rustToolchain:result.rustToolchain})
    writeFileSync(join(directory,'LICENSE'),'Framework license fixture.\n')
    writeFileSync(join(directory,'package.json'),JSON.stringify({name:'brain-license-fixture',version:'0.0.0',private:true,files}))
    const env=anonymousEnvironment(f.root);env.NPM_CONFIG_OFFLINE='true'
    for(const config of [env.NPM_CONFIG_USERCONFIG,env.NPM_CONFIG_GLOBALCONFIG])writeFileSync(config,'audit=false\nfund=false\nupdate-notifier=false\n')
    const [pack]=JSON.parse(execFileSync(process.execPath,[npmCli(),'pack','--dry-run','--json','--ignore-scripts'],{cwd:directory,env,encoding:'utf8',timeout:25000,maxBuffer:4*1024*1024}))
    const paths=pack.files.map(file=>file.path)
    for(const file of [...FILES,'THIRD_PARTY_NOTICES.txt','LICENSE'])assert.ok(paths.includes(file),file)
    assert.match(readFileSync(join(directory,'THIRD_PARTY_NOTICES.txt'),'utf8'),/Vendored attribution must survive/)
    assert.equal(readFileSync(join(directory,FILES[0]),'utf8'),result.notice)
  } finally {f.close()}
})

test('Windows CRLF checkout preserves only the exact pinned upstream license content',()=>{
 const lf=Buffer.from('Public upstream license.\nCopyright fixture authors.\n'),crlf=Buffer.from(lf.toString().replaceAll('\n','\r\n'))
 const expected=hash(lf)
 assert.deepEqual(pinnedText(lf,expected),lf)
 assert.deepEqual(pinnedText(crlf,expected),lf)
 assert.throws(()=>pinnedText(Buffer.from(crlf.toString().replace('authors','changed')),expected),/integrity mismatch/)
 assert.throws(()=>pinnedText(Buffer.from('\ufeff'+lf),expected),/integrity mismatch/)
 assert.throws(()=>pinnedText(Buffer.from(lf.toString().replaceAll('\n','\r')),expected),/integrity mismatch/)
})
test('native bundles bind canonical lock content across Windows checkouts and retain raw checkout audit hashes',()=>{
 const f=fixture()
 try{
  const lf=Buffer.from('version = 4\n# Public lock fixture\n'),crlf=Buffer.from(lf.toString().replaceAll('\n','\r\n'))
  const a=generate({...f.args,cargoLock:lf}),b=generate({...f.args,cargoLock:crlf})
  const ma=JSON.parse(a.manifest),mb=JSON.parse(b.manifest)
  assert.equal(ma.cargoLockHashFormat,'utf8-lf')
  assert.equal(ma.cargoLockSha256,mb.cargoLockSha256)
  assert.equal(ma.cargoLockRawSha256,hash(lf));assert.equal(mb.cargoLockRawSha256,hash(crlf))
  assert.notEqual(ma.cargoLockRawSha256,mb.cargoLockRawSha256)
  assert.notEqual(lockDigest(Buffer.from('changed lock')),ma.cargoLockSha256)
  const binary=join(f.root,'brain');writeFileSync(binary,f.args.binary)
  for(const [i,value] of [b.notice,b.manifest,b.rustStd,b.rustToolchain].entries())writeFileSync(join(f.root,FILES[i]),value)
  assert.ok(readBundle(binary,target,'0.4.0',lf))
  assert.throws(()=>readBundle(binary,target,'0.4.0',Buffer.from('changed lock')),/do not match/)
  // Legacy LF notices remain valid without the additive audit fields.
  delete ma.cargoLockHashFormat;delete ma.cargoLockRawSha256
  for(const [i,value] of [a.notice,JSON.stringify(ma),a.rustStd,a.rustToolchain].entries())writeFileSync(join(f.root,FILES[i]),value)
  assert.ok(readBundle(binary,target,'0.4.0',lf))
 }finally{f.close()}
})
test('notice failures retain actionable stages and allowlisted causes without arbitrary messages or paths',()=>{
 const sentinel='PRIVATE_NOTICE_SENTINEL',error=Error('Pinned public license supplement integrity mismatch')
 error.noticePackage='governor@0.10.4'
 const report=failureDiagnostic('license-inventory',error)
 assert.equal(report.phase,'license-inventory');assert.equal(report.reason,'pinned-license-hash-mismatch');assert.equal(report.package,'governor@0.10.4')
 const hidden=Error(sentinel);hidden.code='ENOENT';hidden.noticePackage=sentinel+' bad package'
 const safe=failureDiagnostic('rust-std',hidden)
 assert.equal(safe.systemCode,'ENOENT');assert.equal(safe.reason,'unclassified-failure');assert.ok(!JSON.stringify(safe).includes(sentinel))
 assert.equal(failureDiagnostic('metadata',Error('cargo: exit 101; inspect the private local log.')).exitCode,101)
})

test('actual supplemental license reader accepts a Windows checkout only after exact upstream digest verification',async()=>{
 const root=mkdtempSync(join(tmpdir(),'brain-license-crlf-reader-'))
 try{
  mkdirSync(join(root,'npm'));mkdirSync(join(root,'scripts/lib'),{recursive:true});mkdirSync(join(root,'governor'))
  copyFileSync(new URL('../cargo-notices.mjs',import.meta.url),join(root,'npm/cargo-notices.mjs'))
  copyFileSync(new URL('../../scripts/lib/jobs.mjs',import.meta.url),join(root,'scripts/lib/jobs.mjs'))
  cpSync(new URL('../licenses',import.meta.url),join(root,'npm/licenses'),{recursive:true})
  const catalog=JSON.parse(readFileSync(join(root,'npm/licenses/supplements.json'),'utf8')),entry=catalog.packages['governor@0.10.4']
  const file=join(root,'npm/licenses',entry.files[0].file),original=readFileSync(file,'utf8').replaceAll('\r\n','\n')
  writeFileSync(file,original.replaceAll('\n','\r\n'))
  const manifest=join(root,'governor/Cargo.toml');writeFileSync(manifest,'# Public fixture\n')
  writeFileSync(join(root,'governor/.cargo_vcs_info.json'),JSON.stringify({git:{sha1:entry.vcsCommit}}))
  const pkg={name:'governor',version:'0.10.4',license:entry.license,manifest_path:manifest}
  const module=await import(pathToFileURL(join(root,'npm/cargo-notices.mjs')).href)
  const [license]=module.licenseFiles(pkg,{entries:0,bytes:0})
  assert.equal(license.sha256,entry.files[0].sha256);assert.equal(license.text,original)
  writeFileSync(file,'Changed public attribution.\r\n')
  assert.throws(()=>module.licenseFiles(pkg,{entries:0,bytes:0}),/integrity mismatch/)
 }finally{rmSync(root,{recursive:true,force:true})}
})
