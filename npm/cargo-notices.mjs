#!/usr/bin/env node
// Cargo-derived public redistribution notices, tied to the exact native artifact. No compilation/publication.
import { createHash } from 'node:crypto'
import { constants, lstatSync, realpathSync, openSync, closeSync, readFileSync, readSync, fstatSync, readdirSync, mkdirSync, writeFileSync, renameSync, rmSync, mkdtempSync } from 'node:fs'
import { dirname, join, resolve, relative, isAbsolute, sep } from 'node:path'
import { tmpdir } from 'node:os'
import { pathToFileURL, fileURLToPath } from 'node:url'
import { run } from '../scripts/lib/jobs.mjs'

export const FILES = ['CARGO_NOTICES.txt', 'CARGO_DEPENDENCIES.json', 'RUST_STD_COPYRIGHT.html', 'RUST_TOOLCHAIN_COPYRIGHT.html']
export const hash = bytes => createHash('sha256').update(bytes).digest('hex')
// Git checkouts can change text line endings on Windows. Only accept a normalized
// supplemental text when its exact LF bytes still match the pinned upstream digest.
const canonicalLF = bytes => Buffer.from(new TextDecoder('utf-8',{fatal:true,ignoreBOM:true}).decode(bytes).replaceAll('\r\n','\n'))
export function pinnedText(bytes, expected) {
  if (hash(bytes) === expected) return bytes
  const canonical = canonicalLF(bytes)
  if (hash(canonical) !== expected) throw Error('Pinned public license supplement integrity mismatch')
  return canonical
}
export const lockDigest = bytes => hash(canonicalLF(bytes))
const SAFE_PHASES = new Set(['arguments','metadata','sysroot','rust-version','binary','cargo-lock','rust-std','rust-toolchain','license-inventory','write-sidecars'])
export function failureDiagnostic(phase,error) {
  const message = typeof error?.message === 'string' ? error.message : ''
  const reasons = new Map([
    ['Pinned public license supplement integrity mismatch','pinned-license-hash-mismatch'],
    ['Pinned public license supplement does not match the exact crate revision','pinned-license-revision-mismatch'],
    ['Cargo did not return its resolved package graph','resolved-graph-missing'],
    ['Cargo notice texts exceed their aggregate bound','legal-text-byte-limit'],
    ['Cargo notice source inventory exceeds its bound','source-entry-limit'],
    ['Machine path detected in redistribution notices','machine-path-in-public-notices'],
    ['Notice source changed during reading','source-changed'],
    ['Notice input must be a bounded regular file','nonregular-or-oversized-file'],
    ['Notice input exceeds its byte bound','file-byte-limit'],
    ['Missing bounded official Rust standard-library copyright document; install the matching rust-docs component','rust-std-copyright-mismatch'],
    ['Missing bounded official Rust toolchain copyright document','rust-toolchain-copyright-mismatch'],
  ])
  const missing = /Missing public LICENSE\/NOTICE files for ([A-Za-z0-9_-]+@[A-Za-z0-9.+-]+)/.exec(message)
  const allowed = new Set(['ENOENT','EACCES','EPERM','ELOOP','EINVAL','EISDIR','EBUSY','ERR_ENCODING_INVALID_ENCODED_DATA'])
  const exit = /: exit ([0-9]{1,3}); inspect the private local log\.$/.exec(message)
  return {schemaVersion:1,ok:false,phase:SAFE_PHASES.has(phase)?phase:'unknown',failure:'cargo-notices-failed',reason:missing?'public-license-missing':reasons.get(message)||'unclassified-failure',systemCode:allowed.has(error?.code)?error.code:null,exitCode:exit&&Number(exit[1])<=255?Number(exit[1]):null,...(missing?{package:missing[1]}:{}),...(typeof error?.noticePackage==='string'&&/^[A-Za-z0-9_-]+@[A-Za-z0-9.+-]+$/.test(error.noticePackage)?{package:error.noticePackage}:{})}
}

const LIMIT = 32 * 1024 * 1024
const label = pkg => `${pkg.name}@${pkg.version}`
function regular(path, limit = LIMIT) {
  const before = lstatSync(path)
  if (!before.isFile() || before.isSymbolicLink() || before.size > limit) throw Error('Notice input must be a bounded regular file')
  const fd = openSync(path, constants.O_RDONLY | (constants.O_NOFOLLOW || 0) | (constants.O_NONBLOCK || 0))
  try {
    const stat = fstatSync(fd)
    if (!stat.isFile() || stat.size > limit || stat.ino !== before.ino || stat.dev !== before.dev) throw Error('Notice source changed during reading')
    const chunks = [], buffer = Buffer.allocUnsafe(65536)
    let total = 0
    for (;;) {
      const count = readSync(fd,buffer,0,Math.min(buffer.length,limit+1-total),null)
      if (!count) break
      total+=count
      if (total>limit) throw Error('Notice input exceeds its byte bound')
      chunks.push(Buffer.from(buffer.subarray(0,count)))
    }
    const after=fstatSync(fd)
    if(after.size!==before.size||after.mtimeMs!==before.mtimeMs||after.size!==total)throw Error('Notice source changed during reading')
    return Buffer.concat(chunks,total)
  } finally { closeSync(fd) }
}
function text(bytes) { return new TextDecoder('utf-8', { fatal: true }).decode(bytes) }
function within(root, path) {
  const rel = relative(root, path)
  return rel !== '..' && !rel.startsWith('..' + sep) && !isAbsolute(rel)
}
export function licenseFiles(pkg, budget) {
  const root = realpathSync(dirname(pkg.manifest_path)), pending = [root], found = new Set()
  while (pending.length) {
    const directory = pending.pop()
    for (const entry of readdirSync(directory, { withFileTypes: true }).sort((a,b) => a.name < b.name ? -1 : a.name > b.name ? 1 : 0)) {
      if (++budget.entries > 1000000) throw Error('Cargo notice source inventory exceeds its bound')
      if (entry.name.startsWith('.') || entry.isSymbolicLink()) continue
      const path = join(directory, entry.name)
      if (entry.isDirectory()) pending.push(path)
      else if (entry.isFile()) {
        const rel = relative(root, path).split(sep).join('/')
        if (/^(?:licen[cs]e|notice|copying|copyright|authors)(?:[._-].*)?$/i.test(entry.name) || rel.split('/').some(part => /^licen[cs]es$/i.test(part))) found.add(rel)
      }
    }
  }
  if (pkg.license_file) {
    const path = resolve(root, pkg.license_file)
    if (!within(root, path) || !within(root, realpathSync(path))) throw Error(`Escaping license-file refused for ${label(pkg)}`)
    found.add(relative(root, path).split(sep).join('/'))
  }
  if (!found.size) {
    const directory = fileURLToPath(new URL('licenses/', import.meta.url))
    const catalog = JSON.parse(text(regular(join(directory,'supplements.json'),256*1024)))
    const entry = catalog.packages?.[label(pkg)]
    if (!entry) throw Error(`Missing public LICENSE/NOTICE files for ${label(pkg)}; review upstream attribution before packing`)
    const vcs = JSON.parse(text(regular(join(root,'.cargo_vcs_info.json'),8192)))
    if (entry.license !== pkg.license || !/^[a-f0-9]{40}$/.test(entry.vcsCommit) || vcs.git?.sha1 !== entry.vcsCommit || !Array.isArray(entry.files) || entry.files.length > 8) throw Error('Pinned public license supplement does not match the exact crate revision')
    return entry.files.map(file => {
      if (!/^[A-Za-z0-9._-]+$/.test(file.file) || !/^https:\/\/raw\.githubusercontent\.com\/[A-Za-z0-9._/-]+$/.test(file.url)) throw Error('Invalid public license supplement provenance')
      const bytes = pinnedText(regular(join(directory,file.file),2*1024*1024),file.sha256)
      budget.bytes+=bytes.length
      if (budget.bytes>LIMIT) throw Error('Cargo notice texts exceed their aggregate bound')
      return {path:'upstream-supplement/'+file.file,sha256:hash(bytes),text:text(bytes),sourceURL:file.url,...(entry.note?{context:entry.note}:{})}
    })
  }
  return [...found].sort().map(path => {
    if (/[\x00-\x1f\x7f\\]/.test(path)) throw Error('Unsafe notice relative path')
    const absolute = join(root, path)
    if (!within(root, realpathSync(absolute))) throw Error('Notice path escaped its public package')
    const bytes = regular(absolute, 2 * 1024 * 1024)
    budget.bytes += bytes.length
    if (budget.bytes > LIMIT) throw Error('Cargo notice texts exceed their aggregate bound')
    return { path, sha256: hash(bytes), text: text(bytes) }
  })
}
export function generate({ metadata, binary, target, cargoLock, rustStdCopyright, rustToolchainCopyright, rustVersion }) {
  if (!/^[a-z0-9_-]{3,100}$/.test(target) || !/^rustc [0-9]+\.[0-9]+\.[0-9]+[^\r\n]*$/.test(rustVersion)) throw Error('Invalid native notice target/toolchain')
  if (!metadata.resolve || !Array.isArray(metadata.packages) || metadata.packages.length > 1024) throw Error('Cargo metadata must contain a bounded resolved graph')
  const packages = new Map(metadata.packages.map(pkg => [pkg.id, pkg]))
  const root = metadata.packages.find(pkg => pkg.name === 'brain-cli' && metadata.workspace_members.includes(pkg.id))
  if (!root) throw Error('Cargo metadata does not identify the framework CLI workspace member')
  const nodes = new Map(metadata.resolve.nodes.map(node => [node.id, node]))
  const visited = new Set(), pending = [root.id]
  while (pending.length) {
    const id = pending.pop(); if (visited.has(id)) continue
    visited.add(id)
    if (visited.size > 512) throw Error('Native dependency closure exceeds 512 packages')
    const node = nodes.get(id); if (!node) throw Error('Incomplete Cargo dependency graph')
    for (const dep of node.deps || []) {
      if (!Array.isArray(dep.dep_kinds)) throw Error('Cargo dependency kinds are required')
      if (dep.dep_kinds.some(kind => ![null,'normal','build','dev'].includes(kind.kind))) throw Error('Unknown Cargo dependency kind; notice closure cannot be proven')
      if (dep.dep_kinds.some(kind => kind.kind === null || kind.kind === 'normal' || kind.kind === 'build')) pending.push(dep.pkg)
    }
  }
  const selected = [...visited].map(id => packages.get(id)).filter(pkg => {
    if (!pkg) throw Error('Cargo dependency package metadata is missing')
    if (typeof pkg.name!=='string'||typeof pkg.version!=='string'||!/^[a-zA-Z0-9_-]+$/.test(pkg.name) || !/^[0-9A-Za-z.+-]+$/.test(pkg.version)) throw Error('Invalid public Cargo package identifier')
    return !(metadata.workspace_members.includes(pkg.id) && pkg.name.startsWith('brain-') && pkg.license === 'MIT')
  }).sort((a,b) => label(a) < label(b) ? -1 : label(a) > label(b) ? 1 : 0)
  const budget = { entries: 0, bytes: 0 }, manifest = [], sections = []
  for (const pkg of selected) {
    let files
    try { files = licenseFiles(pkg, budget) } catch(error) { error.noticePackage=label(pkg); throw error }
    const expression = pkg.license || 'See included license-file'
    if (typeof expression !== 'string' || expression.length > 512 || /[\r\n]/.test(expression)) throw Error('Invalid public Cargo license expression')
    manifest.push({ name: pkg.name, version: pkg.version, license: expression, sourceKind: pkg.source?.split('+')[0] || 'path', files: files.map(({path,sha256,sourceURL,context}) => ({path,sha256,...(sourceURL?{sourceURL}:{}),...(context?{context}:{})})) })
    sections.push(`\n${'='.repeat(72)}\n${label(pkg)}\nDeclared license: ${expression}\n` + files.map(file => `\n--- ${file.path} (SHA-256 ${file.sha256}) ---\n${file.sourceURL?'Public source: '+file.sourceURL+'\n':''}${file.context?'Context: '+file.context+'\n':''}${file.text}\n`).join(''))
  }
  const stdText = text(rustStdCopyright)
  if (!stdText.includes('Rust Standard Library') || rustStdCopyright.length > 8 * 1024 * 1024) throw Error('Missing bounded official Rust standard-library copyright document; install the matching rust-docs component')
  const toolchainText=text(rustToolchainCopyright)
  if(!toolchainText.includes('Rust Toolchain')||rustToolchainCopyright.length>LIMIT)throw Error('Missing bounded official Rust toolchain copyright document')
  const notice = `BRAINLLM ${root.version} — Cargo dependency redistribution notices\nTarget: ${target}\nCargo.lock canonical SHA-256 (UTF-8, CRLF to LF): ${lockDigest(cargoLock)}\nCargo.lock raw checkout SHA-256: ${hash(cargoLock)}\nScope: resolved normal/build dependency closure for brain-cli; may conservatively include build-time and vendored-source notices. Not a linker-level inventory or license compatibility decision.\nRust standard library: see RUST_STD_COPYRIGHT.html (${rustVersion}). Toolchain/vendor notices: RUST_TOOLCHAIN_COPYRIGHT.html, conservatively including tools not linked into this binary.\n` + sections.join('')
  const result = { schemaVersion: 1, target, rootPackage: {name:root.name,version:root.version}, scope:'resolved normal/build dependency closure; excludes dev-only edges; may over-include build/vendor notices', binarySha256:hash(binary), cargoLockSha256:lockDigest(cargoLock), cargoLockHashFormat:'utf8-lf', cargoLockRawSha256:hash(cargoLock), noticesSha256:hash(notice), rustStdSha256:hash(rustStdCopyright), rustToolchainSha256:hash(rustToolchainCopyright), rustVersion, packages:manifest }
  const json = JSON.stringify(result, null, 2) + '\n'
  for (const path of [metadata.workspace_root, process.env.HOME, process.env.USERPROFILE, process.env.CARGO_HOME].filter(path => typeof path === 'string' && path.length > 2)) if (notice.includes(path) || json.includes(path) || stdText.includes(path) || toolchainText.includes(path)) throw Error('Machine path detected in redistribution notices')
  return { notice, manifest:json, rustStd:rustStdCopyright, rustToolchain:rustToolchainCopyright }
}
export function writeNotices(directory, webNotices, cargoNotices = null) {
  const guide = '\nNative compiler dependency licenses accompany each platform package in THIRD_PARTY_NOTICES.txt, CARGO_NOTICES.txt, CARGO_DEPENDENCIES.json RUST_STD_COPYRIGHT.html and RUST_TOOLCHAIN_COPYRIGHT.html. Launcher/initializer/runtime-only packages do not embed a Rust compiler binary.\n'
  writeFileSync(join(directory,'THIRD_PARTY_NOTICES.txt'),webNotices+guide+(cargoNotices?'\n'+cargoNotices.notice.toString('utf8'):''))
  const files=['THIRD_PARTY_NOTICES.txt']
  if(cargoNotices) {
    writeFileSync(join(directory,FILES[0]),cargoNotices.notice)
    writeFileSync(join(directory,FILES[1]),cargoNotices.metadata)
    writeFileSync(join(directory,FILES[2]),cargoNotices.rustStd)
    writeFileSync(join(directory,FILES[3]),cargoNotices.rustToolchain)
    files.push(...FILES)
  }
  return files
}
export function readBundle(binary, target, version, cargoLock) {
  const directory = dirname(binary)
  const notice = regular(join(directory, FILES[0])), metadataBytes = regular(join(directory, FILES[1]), 4 * 1024 * 1024), rustStd = regular(join(directory, FILES[2]), 8 * 1024 * 1024), rustToolchain = regular(join(directory,FILES[3]),LIMIT)
  const metadata = JSON.parse(text(metadataBytes))
  if (metadata.schemaVersion !== 1 || metadata.target !== target || metadata.rootPackage?.name !== 'brain-cli' || metadata.rootPackage.version !== version || metadata.binarySha256 !== hash(regular(binary, 512 * 1024 * 1024)) || metadata.cargoLockSha256 !== lockDigest(cargoLock) || (metadata.cargoLockHashFormat !== undefined && metadata.cargoLockHashFormat !== 'utf8-lf') || (metadata.cargoLockRawSha256 !== undefined && !/^[a-f0-9]{64}$/.test(metadata.cargoLockRawSha256)) || metadata.noticesSha256 !== hash(notice) || metadata.rustStdSha256 !== hash(rustStd) || metadata.rustToolchainSha256 !== hash(rustToolchain) || !Array.isArray(metadata.packages) || !metadata.packages.length) throw Error('Native notices are missing, stale or do not match the binary/lock/target/version; regenerate them before packing')
  return { notice, metadata:metadataBytes, rustStd, rustToolchain }
}
let currentPhase = 'arguments'
async function main() {
  const args = process.argv.slice(2), options = {}
  if (args.length === 1 && args[0] === '--help') { console.log('usage: node npm/cargo-notices.mjs --binary <native-file> --target <rust-target>\nUses locked offline Cargo metadata and the active toolchain rust-docs copyright; writes four sidecars beside the binary. No build/publication.'); return }
  for (let i=0;i<args.length;i+=2) {
    if (!['--binary','--target'].includes(args[i]) || !args[i+1] || options[args[i]]) throw Error('Expected exact --binary and --target arguments')
    options[args[i]]=args[i+1]
  }
  if (!options['--binary'] || !/^[a-z0-9_-]{3,100}$/.test(options['--target'] || '')) throw Error('Native binary and exact Rust target are required')
  const binary = resolve(options['--binary']), root = fileURLToPath(new URL('../',import.meta.url)), temp = mkdtempSync(join(tmpdir(),'brain-cargo-notices-'))
  try {
    const command = async (cmd, argv, phase) => {
      currentPhase=phase
      const log=join(temp,phase+'.log'); await run(cmd,argv,{cwd:root,log,timeout:120000,maxBytes:16*1024*1024})
      return text(regular(log,16*1024*1024)).trim()
    }
    const raw = await command('cargo',['metadata','--locked','--offline','--format-version','1','--filter-platform',options['--target']],'metadata')
    const line = raw.split('\n').find(line => line.startsWith('{"packages":'))
    if (!line) throw Error('Cargo did not return its resolved package graph')
    const metadata=JSON.parse(line), sysroot=await command('rustc',['--print','sysroot'],'sysroot'), rustVersion=await command('rustc',['--version'],'rust-version')
    const readInput=(phase,path,limit)=>{currentPhase=phase;return regular(path,limit)}
    const native=readInput('binary',binary,512*1024*1024),cargoLock=readInput('cargo-lock',join(root,'Cargo.lock'),LIMIT)
    const rustStdCopyright=readInput('rust-std',join(sysroot,'share/doc/rust/COPYRIGHT-library.html'),8*1024*1024)
    const rustToolchainCopyright=readInput('rust-toolchain',join(sysroot,'share/doc/rust/COPYRIGHT.html'),LIMIT)
    currentPhase='license-inventory'
    const result=generate({metadata,binary:native,target:options['--target'],cargoLock,rustStdCopyright,rustToolchainCopyright,rustVersion})
    const values=[result.notice,result.manifest,result.rustStd,result.rustToolchain]
    currentPhase='write-sidecars'
    for(let i=0;i<FILES.length;i++) { const path=join(dirname(binary),FILES[i]), temporary=path+'.tmp';writeFileSync(temporary,values[i]);renameSync(temporary,path) }
    console.log(JSON.stringify({schemaVersion:1,ok:true,target:options['--target'],packages:JSON.parse(result.manifest).packages.length,sidecars:FILES}))
  } finally {rmSync(temp,{recursive:true,force:true})}
}
if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  try { await main() } catch (error) {
    console.error(JSON.stringify(failureDiagnostic(currentPhase,error)));process.exitCode=1
  }
}
