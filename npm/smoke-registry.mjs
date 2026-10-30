#!/usr/bin/env node
// Anonymous exact-version acceptance of published packages. Never uses a checkout binary or credentials.
import { spawn, execFileSync } from 'node:child_process'
import { existsSync, mkdtempSync, mkdirSync, writeFileSync, openSync, writeSync, closeSync, readFileSync, readdirSync, lstatSync, realpathSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join, resolve, delimiter } from 'node:path'
import { pathToFileURL } from 'node:url'
import { request } from 'node:http'

const REGISTRY = 'https://registry.npmjs.org/'
const OUTPUT = 2 * 1024 * 1024
const NPM_CODES=new Set(['E404','E401','E403','E400','E429','E500','E502','E503','EUSAGE','EBADENGINE','EBADPLATFORM','ETARGET','EINTEGRITY','EAI_AGAIN','ETIMEDOUT','ECONNRESET','ECONNREFUSED','ENOTFOUND','ENOENT','EACCES','EPERM'])
const DOCTOR_REPORTS=new WeakMap()
const BROWSER_LABELS=new Set(['viewer','headless-cdp','viewer+headless-cdp','Chrome Accessibility.getFullAXTree','DOM-label-heuristic; not AX tree','DOM-label-heuristic; not Chrome AX tree','forced-GC inspector','not-requested','not-measured','MAIN','HeapProfiler.collectGarbage','HeapProfiler.enable','Accessibility.getFullAXTree','FinalizationRegistry','WeakRef'])
export function doctorSummary(report) {
 const number=value=>Number.isSafeInteger(value)&&value>=0&&value<=1000000?value:null
 const length=value=>Array.isArray(value)?value.length:null
 const label=value=>BROWSER_LABELS.has(value)?value:null
 const boolean=value=>typeof value==='boolean'?value:null
 const capability=value=>{
  if(typeof value==='boolean'||BROWSER_LABELS.has(value))return value
  if(!value||typeof value!=='object'||Array.isArray(value))return null
  const out={}
  for(const key of ['forcedGC','accessibility','heap','pageWorld','scenarioAPI','engine','available','supported'])if(typeof value[key]==='boolean'||BROWSER_LABELS.has(value[key]))out[key]=value[key]
  return out
 }
 const codes=new Set();let visited=0
 const walk=(value,depth)=>{
  if(++visited>2048||depth>12||!value||typeof value!=='object')return
  if(typeof value.code==='string'&&/^B[0-9]{4}$/.test(value.code))codes.add(value.code)
  for(const entry of (Array.isArray(value)?value.slice(0,128):Object.values(value).slice(0,64)))walk(entry,depth+1)
 }
 walk(report,0)
 const files=Array.isArray(report?.test?.files)?report.test.files:[]
 const testError=typeof report?.test?.error==='string'?report.test.error:''
 const errorKind=/Chrome not found/.test(testError)?'browser-unavailable':/Viewer/.test(testError)?'viewer-failure':/CDP|Chrome|browser/i.test(testError)?'browser-runner-failure':testError?'unclassified-test-error':null
 return {ok:boolean(report?.ok),checkErrors:number(report?.check?.errors),passedTests:number(report?.test?.passed),failedTests:number(report?.test?.failed),accessibilityFindings:number(report?.test?.a11y),runtimeErrors:length(report?.test?.errors),heapLeaks:length(report?.heap?.leaks),testErrorPresent:Boolean(testError),testErrorKind:errorKind,buildOk:boolean(report?.build?.ok),buildErrorPresent:typeof report?.build?.error==='string',seoOk:boolean(report?.seo?.ok),seoErrorPresent:typeof report?.seo?.error==='string',seoMinScore:number(report?.seo?.minScore),seoBelowThresholdPages:Array.isArray(report?.seo?.pages)&&Number.isSafeInteger(report?.seo?.minScore)?report.seo.pages.filter(page=>Number.isFinite(page?.score)&&page.score<report.seo.minScore).length:null,adviceErrors:Array.isArray(report?.advice)?report.advice.filter(item=>item?.level==='error').length:null,diagnosticCodes:[...codes].sort().slice(0,32),capability:capability(report?.test?.capability??report?.test?.capabilities),files:files.slice(0,16).map(file=>({engine:label(file?.engine),scenarios:length(file?.tests),failedScenarios:Array.isArray(file?.tests)?file.tests.filter(test=>test?.ok===false).length:null,loadErrorPresent:typeof file?.error==='string',accessibilityFindings:length(file?.a11y),runtimeErrors:length(file?.errors),leaks:length(file?.leaks),coverage:{accessibility:label(file?.coverage?.accessibility),heap:label(file?.coverage?.heap)},capability:capability(file?.capability??file?.capabilities)})),filesTruncated:files.length>16}
}

export function logSummary(bytes) {
 const raw=Buffer.from(bytes).subarray(0,OUTPUT).toString('utf8'),codes=new Set()
 for(const match of raw.matchAll(/\b(?:E[A-Z0-9_]+)\b/g))if(NPM_CODES.has(match[0]))codes.add(match[0])
 return {npmCodes:[...codes].sort(),browserUnavailable:/Chrome not found/.test(raw),browserBridgeFailed:/Connected Viewer capability probe failed|Viewer runner failed|Viewer E2E failed/.test(raw),browserProfileRequired:/profile.{0,40}(?:required|must be selected)|(?:select|specify).{0,40}profile/i.test(raw)}
}
export function publicFailure(error, fallbackPhase='setup') {
 const d=error?.detail||{},phase=typeof d.phase==='string'&&/^[a-z0-9-]{1,80}$/.test(d.phase)?d.phase:fallbackPhase
 const reasons=new Map([
  ['Exact published version is not visible anonymously','exact-version-unavailable'],['Initializer did not install the exact version','installed-version-mismatch'],['Installed launcher escaped its own app','installed-path-escape'],['Published native compiler version mismatch','native-version-mismatch'],['Published compiler did not emit its actual build','invalid-build-output'],['Published doctor did not pass','doctor-failed'],['Invalid owned dev listener metadata','invalid-dev-metadata'],['Published dev HTTP probe did not become ready','dev-http-not-ready'],['Registry smoke deadline','deadline'],['Registry smoke cancelled or capacity exhausted','cancelled-or-capacity'],['Cannot locate the installed npm CLI JavaScript entry','npm-cli-unavailable'],['Invalid bounded smoke metadata','invalid-metadata']])
 const knownReasons=new Set(['nonzero-exit','spawn','deadline','cancelled','disk-limit','structured-output-limit','exact-version-unavailable','registry-version-response-mismatch','registry-invalid-json'])
 const report={schemaVersion:1,ok:false,code:'REGISTRY_SMOKE_FAILED',phase,reason:knownReasons.has(d.reason)?d.reason:reasons.get(error?.message)||(error instanceof SyntaxError?'invalid-json':'unclassified-failure')}
 for(const key of ['exitCode','checkErrors','failedTests','retryAttempts'])if(Number.isSafeInteger(d[key])&&d[key]>=0&&d[key]<=1000000)report[key]=d[key]
 if(['SIGTERM','SIGKILL','SIGABRT'].includes(d.signal))report.signal=d.signal
 if(Array.isArray(d.npmCodes))report.npmCodes=d.npmCodes.filter(code=>NPM_CODES.has(code)).slice(0,24)
 if(Array.isArray(d.diagnosticCodes))report.diagnosticCodes=d.diagnosticCodes.filter(code=>/^B[0-9]{4}$/.test(code)).slice(0,16)
 for(const key of ['browserUnavailable','browserBridgeFailed','browserProfileRequired','retainedFailureLogs'])if(d[key]===true)report[key]=true
 if(knownReasons.has(d.lastAttemptReason))report.lastAttemptReason=d.lastAttemptReason
 if(DOCTOR_REPORTS.has(error)){report.doctor=doctorSummary(DOCTOR_REPORTS.get(error));report.diagnosticCodes=[...new Set([...(report.diagnosticCodes||[]),...report.doctor.diagnosticCodes])].slice(0,32)}
 return report
}
const USAGE = 'usage: node npm/smoke-registry.mjs --version <exact-version> [--timeout-ms 900000] [--no-dev] [--keep-failure-logs]\nInstalls both published initializers anonymously, then runs version/check/build/doctor and a temporary dev HTTP probe. No publication or local BRAIN_BINARY override.'
export function options(args) {
  const out = { version: null, timeout: 900000, dev: true }
  if (args.length === 1 && ['--help', '-h'].includes(args[0])) return { help: true }
  for (let i = 0; i < args.length; i++) {
    if (args[i] === '--version' && out.version === null) out.version = args[++i]
    else if (args[i] === '--timeout-ms' && /^\d+$/.test(args[i + 1] || '')) out.timeout = Number(args[++i])
    else if (args[i] === '--no-dev' && out.dev) out.dev = false
    else if (args[i] === '--keep-failure-logs' && !out.keepFailureLogs) out.keepFailureLogs = true
    else throw Error('Invalid registry smoke arguments')
  }
  if (typeof out.version !== 'string' || !/^(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)(?:-[0-9A-Za-z]+(?:[.-][0-9A-Za-z]+)*)?$/.test(out.version) || out.version.length > 100) throw Error('Use one exact semantic version, without tags, ranges or URLs')
  if (!Number.isSafeInteger(out.timeout) || out.timeout < 60000 || out.timeout > 900000) throw Error('timeout-ms must be 60000..900000')
  return out
}
export function anonymousEnvironment(root, source = process.env) {
  const env = {}
  for (const name of ['PATH', 'SYSTEMROOT', 'SYSTEMDRIVE', 'COMSPEC', 'PATHEXT', 'WINDIR', 'OS', 'PROCESSOR_ARCHITECTURE', 'NUMBER_OF_PROCESSORS', 'LANG', 'LC_ALL', 'TZ']) {
    const key = Object.keys(source).find(key => key.toUpperCase() === name)
    if (key && typeof source[key] === 'string') env[name] = source[key]
  }
  return Object.assign(env, {
    HOME: join(root, 'home'), USERPROFILE: join(root, 'home'),
    APPDATA: join(root, 'home', 'AppData', 'Roaming'), LOCALAPPDATA: join(root, 'home', 'AppData', 'Local'),
    XDG_CONFIG_HOME: join(root, 'home', 'config'),
    TEMP: join(root, 'tmp'), TMP: join(root, 'tmp'), TMPDIR: join(root, 'tmp'),
    NPM_CONFIG_USERCONFIG: join(root, 'user.npmrc'), NPM_CONFIG_GLOBALCONFIG: join(root, 'global.npmrc'),
    NPM_CONFIG_CACHE: join(root, 'cache'), NPM_CONFIG_REGISTRY: REGISTRY,
    NPM_CONFIG_AUDIT: 'false', NPM_CONFIG_FUND: 'false', NPM_CONFIG_UPDATE_NOTIFIER: 'false',
    NPM_CONFIG_FETCH_RETRIES: '2', NPM_CONFIG_FETCH_RETRY_MINTIMEOUT: '1000', NPM_CONFIG_FETCH_RETRY_MAXTIMEOUT: '5000', NPM_CONFIG_FETCH_TIMEOUT: '30000',
    CI: 'true', NO_COLOR: '1', BRAIN_BACKEND_URL: '',
  })
}
export function npmCli(source = process.env) {
  const candidates = [join(dirname(process.execPath), 'node_modules', 'npm', 'bin', 'npm-cli.js'), resolve(dirname(process.execPath), '../lib/node_modules/npm/bin/npm-cli.js')]
  for (const directory of String(source.PATH || source.Path || '').split(delimiter)) {
    const npm = join(directory, process.platform === 'win32' ? 'npm.cmd' : 'npm')
    if (!existsSync(npm)) continue
    if (process.platform === 'win32') candidates.push(join(directory, 'node_modules', 'npm', 'bin', 'npm-cli.js'))
    else { try { candidates.push(realpathSync(npm)) } catch {} }
  }
  const found = candidates.find(path => path.endsWith('npm-cli.js') && existsSync(path))
  if (!found) throw Error('Cannot locate the installed npm CLI JavaScript entry')
  return realpathSync(found)
}
function descendants(pid) {
  if (process.platform === 'win32') return []
  try {
    const rows = execFileSync('/bin/ps', ['-axo', 'pid=,ppid=,pgid='], { encoding: 'utf8', timeout: 2000, maxBuffer: 1024 * 1024 }).trim().split('\n').map(line => line.trim().split(/\s+/).map(Number))
    const owned = new Set([pid]), found = []
    for (let pass = 0; pass < 32; pass++) {
      let changed = false
      for (const [child, parent, group] of rows) if (owned.has(parent) && !owned.has(child) && Number.isSafeInteger(child) && child > 1) {
        if (owned.size >= 128) return found
        owned.add(child); found.push({ pid: child, group }); changed = true
      }
      if (!changed) break
    }
    return found
  } catch { return [] }
}
function kill(child, force = false, tree = []) {
  if (!child.pid) return
  try {
    if (process.platform === 'win32') execFileSync(join(process.env.SYSTEMROOT || process.env.SystemRoot || 'C:\\Windows', 'System32', 'taskkill.exe'), ['/PID', String(child.pid), '/T', '/F'], { stdio: 'ignore', timeout: 5000, windowsHide: true })
    else {
      for (const item of [...tree].reverse()) {
        try { process.kill(item.group === item.pid ? -item.pid : item.pid, force ? 'SIGKILL' : 'SIGTERM') } catch {}
      }
      process.kill(-child.pid, force ? 'SIGKILL' : 'SIGTERM')
    }
  } catch {}
}

export function childSession(args, { cwd, env, log, timeout, phase, active }) {
  const fd = openSync(log, 'w', 0o600)
  const child = spawn(process.execPath, args, { cwd, env, detached: process.platform !== 'win32', windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] })
  let retained = 0, stdoutBytes = 0, chunks = [], reason = '', stopping = false, escalation, closed = false, ownedTree = []
  const stop = (failure = '') => {
    if (closed) return
    if (failure) reason ||= failure
    if (stopping) return
    stopping = true
    ownedTree = descendants(child.pid)
    kill(child, false, ownedTree)
    escalation = setTimeout(() => kill(child, true, ownedTree), 1000)
  }
  const session = { child, stop, get exited() { return closed }, done: null }
  active.add(session)
  const timer = setTimeout(() => stop('deadline'), timeout)
  const capture = (chunk, stdout) => {
    if (retained < OUTPUT) { const part = chunk.subarray(0, OUTPUT - retained); writeSync(fd, part); retained += part.length }
    if (stdout) { stdoutBytes += chunk.length; if (stdoutBytes <= OUTPUT) chunks.push(chunk) }
    if (stdoutBytes > OUTPUT) stop('structured-output-limit')
  }
  child.stdout.on('data', chunk => capture(chunk, true)); child.stderr.on('data', chunk => capture(chunk, false))
  session.done = new Promise((resolveDone, reject) => {
    child.once('error', () => { reason ||= 'spawn' })
    child.once('exit', () => { if (process.platform !== 'win32') kill(child, true, ownedTree) })
    child.once('close', (code, signal) => {
      closed = true; active.delete(session); clearTimeout(timer); clearTimeout(escalation); closeSync(fd)
      if (reason || code !== 0 && !stopping) {
        const error = Error('Published package smoke phase failed')
        error.detail = { phase, exitCode: code, signal, reason: reason || 'nonzero-exit' }
        try { Object.assign(error.detail,logSummary(readFileSync(log))) } catch {}
        try {
          const report = JSON.parse(Buffer.concat(chunks).toString('utf8'))
          if(phase.startsWith('doctor-')&&report&&typeof report==='object'&&!Array.isArray(report))DOCTOR_REPORTS.set(error,report)
          const diagnostics = Array.isArray(report) ? report : report.check?.diags || report.diagnostics || []
          error.detail.diagnosticCodes = diagnostics.map(item => item.code).filter(code => /^B[0-9]{4}$/.test(code)).slice(0, 16)
          if (Number.isInteger(report.check?.errors)) error.detail.checkErrors = report.check.errors
          if (Number.isInteger(report.test?.failed)) error.detail.failedTests = report.test.failed
          if (/Chrome not found/.test(report.test?.error || '')) error.detail.browserUnavailable = true
        } catch {}
        reject(error)
      } else resolveDone({ stdout: Buffer.concat(chunks).toString('utf8'), code, signal })
      chunks = []
    })
  })
  session.done.catch(() => {}) // Background dev failures are observed by the readiness loop/finally.
  return session
}
function diskBound(root) {
  const pending = [root]; let bytes = 0, entries = 0
  while (pending.length) {
    const directory = pending.pop()
    let entriesHere
    try { if (lstatSync(directory).isSymbolicLink()) continue; entriesHere = readdirSync(directory, { withFileTypes: true }) } catch (error) { if (error.code === 'ENOENT') continue; throw error }
    for (const entry of entriesHere) {
      if (++entries > 20000) throw Error('Owned registry smoke file-count limit')
      const path = join(directory, entry.name)
      if (entry.isSymbolicLink()) continue // npm .bin links are expected; never walk their targets.
      if (entry.isDirectory()) pending.push(path)
      else if (entry.isFile()) { try { bytes += lstatSync(path).size } catch (error) { if (error.code !== 'ENOENT') throw error } }
      if (bytes > 1024 * 1024 * 1024) throw Error('Owned registry smoke disk limit')
    }
  }
}

function readJson(path) {
  const stat = lstatSync(path)
  if (!stat.isFile() || stat.size > OUTPUT) throw Error('Invalid bounded smoke metadata')
  return JSON.parse(readFileSync(path, 'utf8'))
}
export function httpProbe(url) {
  return new Promise(resolveProbe => {
    const req = request(url, { timeout: 3000, agent: false }, response => {
      let size = 0, html = ''
      response.on('data', chunk => { size += chunk.length; if (size > 512 * 1024) response.destroy(); else html += chunk.toString('utf8') })
      response.once('end', () => resolveProbe(response.statusCode === 200 && /^text\/html\b/.test(response.headers['content-type'] || '') && /<(?:html|main)\b/i.test(html)))
      response.once('error', () => resolveProbe(false))
    })
    req.once('timeout', () => req.destroy()); req.once('error', () => resolveProbe(false)); req.end()
  })
}
export async function smoke(config) {
  const root = mkdtempSync(join(tmpdir(), 'brain-registry-smoke-'))
  const active = new Set(), expires = Date.now() + config.timeout
  const env = anonymousEnvironment(root)
  let cancelled = false, exhausted = false, phase='setup', preserve=false
  const cancel = () => { cancelled = true; for (const job of active) job.stop('cancelled') }
  process.once('SIGINT', cancel); process.once('SIGTERM', cancel)
  let monitor
  try {
    for (const path of [env.HOME, env.APPDATA, env.LOCALAPPDATA, env.XDG_CONFIG_HOME, env.TEMP, env.NPM_CONFIG_CACHE]) mkdirSync(path, { recursive: true, mode: 0o700 })
    const npmrc = `registry=${REGISTRY}\naudit=false\nfund=false\nupdate-notifier=false\n`
    for (const name of ['user.npmrc', 'global.npmrc', '.npmrc']) writeFileSync(join(root, name), npmrc, { mode: 0o600 })
    writeFileSync(join(root, 'package.json'), '{"private":true}\n')
    const npm = npmCli()
    const remaining = () => { if (cancelled || exhausted) throw Error('Registry smoke cancelled or capacity exhausted'); const left = expires - Date.now(); if (left <= 0) throw Error('Registry smoke deadline'); return left }
    const run = async (args, cwd, nextPhase, limit = 180000) => {
      phase=nextPhase
      console.log(JSON.stringify({ phase, version: config.version }))
      return childSession(args, { cwd, env, phase, timeout: Math.min(limit, remaining()), log: join(root, phase + '.log'), active }).done
    }
    monitor = setInterval(() => { try { diskBound(root) } catch { exhausted = true; for (const job of active) job.stop('disk-limit') } }, 2000)
    for (const pkg of ['brainllm', 'create-brainllm', 'create-brainll']) {
      let available = false, lastError=null
      for (let attempt = 0; attempt < 5 && !available; attempt++) {
        try { const result = await run([npm, 'view', `@christianirack/${pkg}@${config.version}`, 'version', '--json'], root, `registry-${pkg}-${attempt}`, 30000); available = JSON.parse(result.stdout) === config.version; if(!available){lastError=Error('Registry version response mismatch');lastError.detail={phase,reason:'registry-version-response-mismatch',exitCode:result.code}} } catch(error) {lastError=error; if(error instanceof SyntaxError)lastError.detail={phase,reason:'registry-invalid-json'}}
        if (!available) { remaining(); await new Promise(resolveDelay => setTimeout(resolveDelay, Math.min(1000 * 2 ** attempt, remaining()))) }
      }
      if (!available) {const error=Error('Exact published version is not visible anonymously');error.detail={...lastError?.detail,phase,reason:'exact-version-unavailable',lastAttemptReason:lastError?.detail?.reason,retryAttempts:5};throw error}
    }
    const checks = []
    for (const [initializer, name] of [['brainllm', 'registry-long'], ['brainll', 'registry-short']]) {
      // npm create resolves the exact create-* package; its normal install path must install the pinned launcher.
      await run([npm, 'create', '--yes', `@christianirack/${initializer}@${config.version}`, name], root, `create-${initializer}`, 240000)
      phase=`validate-install-${initializer}`
      const app = join(root, name), dependency = '@christianirack/brainllm'
      if (readJson(join(app, 'package.json')).devDependencies?.[dependency] !== config.version || readJson(join(app, 'node_modules', dependency, 'package.json')).version !== config.version) throw Error('Initializer did not install the exact version')
      const launcher = join(app, 'node_modules', dependency, 'bin', 'brain.js')
      if (!realpathSync(launcher).startsWith(realpathSync(app) + (process.platform === 'win32' ? '\\' : '/'))) throw Error('Installed launcher escaped its own app')
      const version = await run([launcher, '--version'], app, `version-${initializer}`)
      phase=`validate-version-${initializer}`
      if (version.stdout.trim() !== `brain ${config.version}`) throw Error('Published native compiler version mismatch')
      await run([launcher, 'check', '--json'], app, `check-${initializer}`)
      const build = JSON.parse((await run([launcher, 'build', '--json'], app, `build-${initializer}`)).stdout)
      phase=`validate-build-${initializer}`
      if (build.schemaVersion !== 1 || !Array.isArray(build.ledger) || !existsSync(join(app, 'dist', 'index.html'))) throw Error('Published compiler did not emit its actual build')
      const doctor = JSON.parse((await run([launcher, 'doctor', '--json'], app, `doctor-${initializer}`, 240000)).stdout)
      phase=`validate-doctor-${initializer}`
      if (doctor.ok !== true) throw Error('Published doctor did not pass')
      let devHttp = 'disabled'
      if (config.dev) {
        phase = `dev-${initializer}`
        console.log(JSON.stringify({phase,version:config.version}))
        const dev = childSession([launcher, 'run', '--host', '127.0.0.1', '--port', '0', '--no-inspect', '--quiet', '--json'], { cwd: app, env, phase, active, timeout: Math.min(120000, remaining()), log: join(root, phase + '.log') })
        try {
          const until = Math.min(expires, Date.now() + 90000); let ready = false
          while (Date.now() < until && !dev.exited) {
            const file = join(app, '.brain', 'dev.json')
            if (existsSync(file)) {
              const data = readJson(file), url = new URL(data.url)
              if (url.hostname !== '127.0.0.1' || url.protocol !== 'http:' || !Number.isInteger(data.port) || data.port < 1 || data.port > 65535 || Number(url.port) !== data.port) throw Error('Invalid owned dev listener metadata')
              if (await httpProbe(url)) { ready = true; break }
            }
            await new Promise(resolveDelay => setTimeout(resolveDelay, 200))
          }
          if (!ready) throw Error('Published dev HTTP probe did not become ready')
          devHttp = 'passed'
        } finally { dev.stop(); await dev.done }
      }
      checks.push({ initializer: `@christianirack/${initializer}`, version: config.version, check: true, build: true, doctor: true, browserTests: doctor.test?.skipped ? 'unsupported-on-platform' : 'executed', devHttp })
    }
    console.log(JSON.stringify({ schemaVersion: 1, ok: true, anonymous: true, registry: REGISTRY, platform: process.platform, arch: process.arch, version: config.version, checks }))
  } catch(error) {
    error.detail={...publicFailure(error,phase),retainedFailureLogs:config.keepFailureLogs===true}
    if(config.keepFailureLogs){preserve=true;error.privateRoot=root;writeFileSync(join(root,'failure-summary.json'),JSON.stringify(publicFailure(error,phase),null,2)+'\n',{mode:0o600})}
    throw error
  } finally {
    clearInterval(monitor); process.removeListener('SIGINT', cancel); process.removeListener('SIGTERM', cancel)
    for (const job of active) job.stop()
    await Promise.allSettled([...active].map(job => job.done))
    if(!preserve)rmSync(root, { recursive: true, force: true, maxRetries: 5, retryDelay: 100 })
  }
}
async function main() {
  let config
  try { config = options(process.argv.slice(2)); if (config.help) { console.log(USAGE); return } }
  catch { console.error(USAGE); process.exitCode = 2; return }
  try { await smoke(config) }
  catch (error) { console.error(JSON.stringify({ schemaVersion: 1, ok: false, code: 'REGISTRY_SMOKE_FAILED', ...publicFailure(error) })); process.exitCode = 1 }
}
if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) await main()
