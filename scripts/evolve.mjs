#!/usr/bin/env node
// Local agent contribution controller. Run this trusted copy, never a candidate's replacement controller.
import { mkdirSync, readFileSync, writeFileSync, renameSync, existsSync, readdirSync, rmSync, copyFileSync, chmodSync } from 'node:fs'
import { resolve, join, dirname } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { createHash } from 'node:crypto'
import { git, run } from './lib/jobs.mjs'
import { audit } from './privacy-audit.mjs'

const here = dirname(fileURLToPath(import.meta.url))
const sha = p => createHash('sha256').update(readFileSync(p)).digest('hex')
const read = p => { try { return JSON.parse(readFileSync(p, 'utf8')) } catch { throw new Error('Invalid or unreadable local JSON record; inspect it privately.') } }
const save = (p, v) => { const tmp = `${p}.${process.pid}.tmp`; writeFileSync(tmp, JSON.stringify(v, null, 2) + '\n', { mode: 0o600 }); renameSync(tmp, p) }
const clean = root => { if (git(root, 'status', '--porcelain', '--untracked-files=normal')) throw new Error('Working tree changed; commit or preserve your work before proceeding.') }
const validId = id => { if (!/^[a-z][a-z0-9-]{0,47}$/.test(id || '')) throw new Error('Use a proposal ID with lowercase letters, digits and hyphens (1-48 characters).'); return id }
const guarded = /^(?:scripts\/|\.github\/|AGENTS\.md$|CONTRIBUTING\.md$|\.gitmodules$|bench\/(?:llm\/|fixtures-check\.mjs$))|(?:^|\/)(?:Cargo\.(?:toml|lock)|package(?:-lock)?\.json|build\.rs)$/
const affectsAuthors = /^(?:SPEC\.md$|crates\/brain-(?:check|parse|lex)\/|crates\/brain-cli\/src\/templates\/|crates\/brain-cli\/src\/scaffold\.rs$)/

export function policy(files) {
  if (files.some(f => guarded.test(f))) throw new Error('Proposal changes its gates, dependencies or authority. Use a normal reviewed contribution for those files.')
  if (!files.some(f => /(?:\/tests?\/|\.test\.mjs$)/.test(f))) throw new Error('Include a regression test that reproduces the original failure or protects the improvement.')
  return { needsLlmGate: files.some(f => affectsAuthors.test(f)) }
}

export async function main(args, root = resolve(here, '..')) {
  const [command, value] = args
  if (!command || command === '--help') return { schemaVersion: 1, commands: ['start <id>', 'finish <id> [--llm-run <json>]', 'activate <id>', 'rollback', 'push <id>', 'status', 'run -- <brain arguments>', 'discard <id>'], instructions: 'docs/EVOLUTION.md' }
  if (process.platform === 'win32') throw new Error('The lifecycle controller currently requires macOS or Linux process groups. Normal brain builds remain supported on Windows.')
  root = git(root, 'rev-parse', '--show-toplevel')
  const home = join(root, '.brain/evolve')
  mkdirSync(home, { recursive: true, mode: 0o700 })
  const configPath = join(root, '.brain/evolution.json')
  const config = existsSync(configPath) ? read(configPath) : {}
  const activePath = join(home, 'active.json')
  const active = () => existsSync(activePath) ? read(activePath) : { current: null, previous: null }
  const dir = id => join(home, validId(id))
  const record = id => read(join(dir(id), 'proposal.json'))
  if (command === 'status') return { schemaVersion: 1, active: active(), proposals: readdirSync(home).filter(n => existsSync(join(home, n, 'proposal.json'))).map(id => record(id)) }
  if (command === 'run') {
    if (args.indexOf('--') < 0 || args.indexOf('--') === args.length - 1) throw new Error('Use run -- <brain arguments>.')
    const a = active()
    if (!a.current) throw new Error('No validated compiler is active. Finish and activate a proposal first, or use your installed brain.')
    const p = record(a.current), bin = join(dir(p.id), 'brain')
    if (sha(bin) !== p.binarySha256) throw new Error('Active binary changed. Revalidate before use.')
    await run(bin, args.slice(args.indexOf('--') + 1), { cwd: process.cwd(), log: join(home, 'last-run.log'), timeout: 120_000 })
    return { schemaVersion: 1, ok: true, log: join(home, 'last-run.log') }
  }
  // One operation across worktrees, no invisible unbounded queue. A crashed lock is diagnosed, never stolen.
  const lock = resolve(root, git(root, 'rev-parse', '--git-common-dir'), 'brain-evolve.lock')
  try { mkdirSync(lock) } catch { throw new Error('Another evolution operation owns the lock. Read its owner.json; if it crashed, confirm that PID is gone before removing that lock.') }
  save(join(lock, 'owner.json'), { pid: process.pid, command, started: new Date().toISOString() })
  try {
    if (command === 'rollback') {
      const a = active()
      if (a.previous) { const p = record(a.previous); if (sha(join(dir(p.id), 'brain')) !== p.binarySha256) throw new Error('Previous binary changed.') }
      save(activePath, { current: a.previous, previous: a.current })
      return { schemaVersion: 1, ok: true, active: active() }
    }
    const id = validId(value), folder = dir(id), work = join(folder, 'worktree'), statePath = join(folder, 'proposal.json')
    if (command === 'start') {
      clean(root)
      if (existsSync(folder)) throw new Error('Proposal already exists; inspect status or choose another ID.')
      if (readdirSync(home).filter(n => existsSync(join(home, n, 'proposal.json'))).length >= 4) throw new Error('Four proposals retained; discard an inactive proposal before creating another.')
      mkdirSync(folder, { mode: 0o700 })
      const base = git(root, 'rev-parse', 'HEAD')
      git(root, 'worktree', 'add', '--detach', work, base)
      const p = { schemaVersion: 1, id, base, status: 'editing', worktree: work }
      save(statePath, p)
      return p
    }
    const p = record(id)
    if (p.id !== id || p.worktree !== work) throw new Error('Invalid proposal ownership record.')
    const eligible = () => {
      if (!['validated', 'pushed'].includes(p.status)) throw new Error('Proposal must pass finish before activation or push.')
      clean(work)
      if (git(work, 'rev-parse', 'HEAD') !== p.commit || sha(join(folder, 'brain')) !== p.binarySha256) throw new Error('Validated inputs or binary changed; run finish again.')
    }
    if (command === 'discard') {
      const a = active()
      if ([a.current, a.previous].includes(id)) throw new Error('Cannot discard an active or rollback compiler.')
      git(root, 'worktree', 'remove', work) // Refuses dirty work; the caller must preserve it first.
      rmSync(folder, { recursive: true })
      return { schemaVersion: 1, ok: true, discarded: id }
    }
    const activate = () => {
      eligible()
      const a = active()
      if (a.current !== id) save(activePath, { current: id, previous: a.current })
    }
    const push = async () => {
      eligible()
      if (!/^https:\/\/[^/@\s]+\/[^\s?#]+\.git$|^git@[^:\s]+:[^\s]+\.git$/.test(config.pushUrl || '')) throw new Error('Configure an explicit credential-free pushUrl in .brain/evolution.json.')
      const exceptions = existsSync(join(here, 'privacy-fixtures.json')) ? read(join(here, 'privacy-fixtures.json')) : []
      const privacy = audit(work, { history: true, exceptions })
      save(join(folder, 'privacy.json'), privacy)
      if (!privacy.ok) throw new Error('Publication blocked by privacy findings, including Git history. See the private privacy.json report.')
      if (!existsSync(join(work, 'LICENSE')) || /license\s*=\s*"UNLICENSED"/.test(readFileSync(join(work, 'Cargo.toml'), 'utf8'))) throw new Error('Select an open-source license before publication.')
      const branch = `refs/heads/llm/${id}-${p.commit.slice(0, 12)}`
      // Explicit URL and one refspec: no remote aliases, matching-branch push, tags, force or default-branch merge.
      const existing = git(root, 'ls-remote', config.pushUrl, branch).split(/\s/)[0]
      if (existing && existing !== p.commit) throw new Error('Destination branch already contains another commit.')
      if (!existing) await run('git', ['-c', 'push.followTags=false', 'push', '--porcelain', config.pushUrl, `${p.commit}:${branch}`], { cwd: work, log: join(folder, 'push.log'), timeout: 60_000 })
      if (git(root, 'ls-remote', config.pushUrl, branch).split(/\s/)[0] !== p.commit) throw new Error('Push result is unknown; inspect the remote branch before retrying.')
      p.status = 'pushed'; p.branch = branch; save(statePath, p)
    }
    if (command === 'activate') { activate(); return { schemaVersion: 1, ok: true, active: active() } }
    if (command === 'push') { await push(); return p }
    if (command !== 'finish') throw new Error('Unknown command; run --help.')
    const files = [...new Set([
      ...git(work, 'diff', '--name-only', p.base, '--').split('\n'),
      ...git(work, 'ls-files', '--others', '--exclude-standard').split('\n'),
    ].filter(Boolean))]
    const requirements = policy(files)
    const exceptions = existsSync(join(here, 'privacy-fixtures.json')) ? read(join(here, 'privacy-fixtures.json')) : []
    const privacy = audit(work, { exceptions })
    save(join(folder, 'privacy.json'), privacy)
    if (!privacy.ok) throw new Error('Candidate privacy audit failed. See privacy.json; values are not logged.')
    // Only this owned, isolated proposal is staged; the main working tree and its index are untouched.
    git(work, 'add', '--all')
    if (git(work, 'diff', '--cached', '--name-only')) git(work, '-c', 'user.name=BRAINLLM Agent', '-c', 'user.email=agent@users.noreply.github.com', 'commit', '-m', `fix: ${id}`)
    p.commit = git(work, 'rev-parse', 'HEAD'); p.status = 'validating'; p.files = files; save(statePath, p)
    const env = Object.fromEntries(Object.entries(process.env).filter(([k]) => /^(?:PATH|HOME|TMPDIR|TMP|TEMP|SYSTEMROOT|LANG|LC_.*|RUSTUP_HOME|CARGO_HOME|CARGO_TARGET_.*_RUNNER)$/.test(k)))
    env.CARGO_BUILD_JOBS = '2'
    // No credential-bearing environment is passed to candidate builds/tests; this is not an OS sandbox.
    const stages = [
      ['build', 'cargo', ['build', '--locked', '-p', 'brain-cli']],
      ['rust', 'cargo', ['test', '--locked', '--workspace']],
      ['runtime', 'npm', ['test', '--prefix', 'runtime']],
      ['tooling', 'node', ['--test', ...readdirSync(join(work, 'scripts/test')).filter(n => n.endsWith('.test.mjs')).map(n => `scripts/test/${n}`)]],
      ['fixtures', 'node', [join(here, '../bench/fixtures-check.mjs')]],
      ['english', 'bash', [join(here, 'check-english.sh')]],
      ['release', 'cargo', ['build', '--release', '--locked', '-p', 'brain-cli']],
    ]
    // Use the candidate's fixture cwd and English checker (both immutable under this proposal policy).
    stages[4][2] = ['bench/fixtures-check.mjs']; stages[5][2] = ['scripts/check-english.sh']
    try {
      p.stages = []
      for (const [name, cmd, argv] of stages) {
        const result = await run(cmd, argv, { cwd: work, env, timeout: name === 'release' ? 900_000 : 600_000, log: join(folder, `${name}.log`) })
        p.stages.push({ name, ...result }); save(statePath, p)
      }
      const candidate = join(work, 'target/release/brain')
      copyFileSync(candidate, join(folder, 'brain')); chmodSync(join(folder, 'brain'), 0o700)
      p.binarySha256 = sha(join(folder, 'brain'))
      await run(join(folder, 'brain'), ['--version', '--json'], { cwd: work, env, log: join(folder, 'version.log') })
      if (requirements.needsLlmGate) {
        const i = args.indexOf('--llm-run'), evidence = i >= 0 ? resolve(args[i + 1] || '') : ''
        if (!evidence || !existsSync(evidence)) throw new Error('Author-facing change requires a fresh frozen LLM run: --llm-run <json>. See docs/EVOLUTION.md.')
        const s = read(evidence).summary
        if (!s || s.binSha !== p.binarySha256.slice(0, 12) || s.stable !== true) throw new Error('LLM evidence does not match this exact release compiler.')
        await run(join(folder, 'brain'), ['spec'], { cwd: work, env, log: join(folder, 'served-spec.txt') })
        if (s.specSha !== sha(join(folder, 'served-spec.txt')).slice(0, 12)) throw new Error('LLM evidence uses a different served spec.')
        await run('node', ['bench/llm/gate.mjs', evidence], { cwd: work, env, log: join(folder, 'llm.log') })
        p.llmEvidenceSha256 = sha(evidence)
      }
      clean(work)
      if (git(work, 'rev-parse', 'HEAD') !== p.commit) throw new Error('Candidate changed during validation.')
      p.status = 'validated'; save(statePath, p)
    } catch (error) { p.status = 'failed'; save(statePath, p); throw error }
    if (config.activate === true) activate()
    if (config.autoPush === true) await push()
    return p
  } finally { rmSync(lock, { recursive: true, force: true }) }
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  try { console.log(JSON.stringify(await main(process.argv.slice(2)), null, 2)) }
  catch (e) { console.log(JSON.stringify({ schemaVersion: 1, ok: false, error: e.message })); process.exitCode = 1 }
}
