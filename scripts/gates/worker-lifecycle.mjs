// Runs inside backend-check so its signal cleanup owns every suspended database session and child.
import assert from 'node:assert/strict'
import { spawn, spawnSync } from 'node:child_process'
import { join } from 'node:path'
import { writeFileSync } from 'node:fs'

export async function workerLifecycle({ binary, directory, postgresPort, postmasterPID, exe,
  auxiliary, auxiliaryCleanup, suspendedOwnedDatabase, terminateOwned, waitExit }) {
  const application = 'brain_gate_worker_lifecycle'
  const database = 'gate_worker_lifecycle'
  const app = join(directory, 'worker-lifecycle-app')
  const until = Date.now() + 105000
  const env = { PATH: process.env.PATH, HOME: process.env.HOME, TMPDIR: directory, LANG: 'C', LC_ALL: 'C',
    BRAIN_NO_DELEGATE: '1', PUBLIC_SITE_URL: 'https://example.test',
    DATABASE_URL: `postgres://brain_test@127.0.0.1:${postgresPort}/${database}?application_name=${application}`,
    BRAIN_JOBS_API_KEY: 'bk_live_' + 'a'.repeat(12) + '_' + 'b'.repeat(64), BRAIN_BACKEND_URL: 'http://127.0.0.1:1' }
  const sleep = ms => new Promise(resolve => setTimeout(resolve, ms))
  const alive = child => child?.pid && child.exitCode === null && child.signalCode === null
  function run(command, args, cwd = directory, timeout = 20000, maxBuffer = 65536) {
    assert.ok(Date.now() < until, 'worker lifecycle gate exceeded its operation deadline')
    const result = spawnSync(command, args, { cwd, env, encoding: 'utf8',
      timeout: Math.min(timeout, Math.max(1, until - Date.now())), maxBuffer })
    assert.ok(!result.error && result.status === 0, `worker lifecycle command failed: ${args[0]} (error ${result.error?.code || 'none'}, exit ${result.status}, signal ${result.signal || 'none'}) ${(result.stderr || '').slice(-1000)}`)
    return result.stdout.trim()
  }
  const cli = (args, cwd = app) => run(binary, args, cwd)
  const pgArgs = ['-X', '-h', '127.0.0.1', '-p', String(postgresPort), '-U', 'brain_test', '-d', database, '-At', '-v', 'ON_ERROR_STOP=1']
  const sql = statement => run(exe('psql'), [...pgArgs, '-c', statement], directory, 2000)
  function launch(command, args, cwd = directory, input = 'ignore') {
    const child = spawn(command, args, { cwd, env, detached: true, stdio: [input, 'pipe', 'pipe'] })
    auxiliary.add(child)
    const state = { child, output: '', bytes: 0 }
    child.once('error', () => {})
    child.stdin?.on('error', () => {})
    const capture = chunk => { state.bytes += chunk.length; if (state.bytes <= 65536) state.output += chunk.toString() }
    child.stdout.on('data', capture); child.stderr.on('data', capture)
    return state
  }
  async function observe(probe, milliseconds, message) {
    const end = Math.min(until, Date.now() + milliseconds)
    do { const result = probe(); if (result) return result; await sleep(20) } while (Date.now() < end)
    throw Error(message)
  }
  function identity(pid) {
    const output = spawnSync('ps', ['-p', String(pid), '-o', 'pid=,ppid=,pgid=,lstart=,comm='], { encoding: 'utf8', timeout: 1000, maxBuffer: 4096 }).stdout || ''
    const match = output.trim().match(/^(\d+)\s+(\d+)\s+(\d+)\s+(.{24})\s+(.+)$/)
    return match ? { pid: Number(match[1]), parent: Number(match[2]), group: Number(match[3]), started: match[4], command: match[5] } : null
  }
  const same = (first, second) => first && second && first.pid === second.pid && first.group === second.group && first.started === second.started
  let worker, locker, databasePID, handler
  const cleanupHandler = async () => {
    if (!handler || !same(handler, identity(handler.pid))) return
    // Only the child observed under our worker, with its original start time and process group, is owned.
    try { process.kill(handler.group === handler.pid ? -handler.pid : handler.pid, 'SIGKILL') } catch {}
    const deadline = Date.now() + 1000
    while (same(handler, identity(handler.pid)) && Date.now() < deadline) await sleep(20)
    assert.ok(!same(handler, identity(handler.pid)), 'owned handler survived failure cleanup')
  }
  auxiliaryCleanup.add(cleanupHandler)
  try {
    run(exe('createdb'), ['-h', '127.0.0.1', '-p', String(postgresPort), '-U', 'brain_test', database])
    cli(['new', 'worker-lifecycle-app', '--json'], directory)
    writeFileSync(join(app, '.brain_env'), 'PUBLIC_SITE_URL=https://example.test\nBRAIN_JOBS_API_KEY?=\nBRAIN_BACKEND_URL?=\n')
    cli(['add', 'job', 'slowwork', '--fields', 'value:str', '--json'])
    writeFileSync(join(app, 'server/jobs/slowwork/queue.jsbrain'), '<meta server>\nexport default { attempts: 3, lease: 10, timeout: 30, drain: 0, concurrency: 1 }\n')
    writeFileSync(join(app, 'server/jobs/slowwork/handler.jsbrain'), '<meta server>\nexport default async function handle(ctx) { await new Promise(resolve => setTimeout(resolve, 60000)) }\n')
    cli(['backend', 'migrate', '--json'])
    const job = JSON.parse(cli(['jobs', 'enqueue', 'slowwork', '--payload', '{}', '--json']))
    assert.match(job.id, /^[a-f0-9-]{36}$/)
    worker = launch(binary, ['jobs', 'work', 'slowwork', '--concurrency', '1'], app)
    await observe(() => {
      assert.ok(alive(worker.child) && worker.bytes <= 65536, 'worker exited before its job started')
      return sql(`SELECT state FROM public._brain_jobs WHERE id='${job.id}'`) === 'running'
    }, 15000, 'synthetic job did not start')
    handler = await observe(() => {
      const rows = run('ps', ['-axo', 'pid=,ppid=,comm='], directory, 1000, 512 * 1024).split('\n')
      const children = rows.flatMap(line => {
        const match = line.trim().match(/^(\d+)\s+(\d+)\s+(.+)$/)
        return match && Number(match[2]) === worker.child.pid && /(^|\/)node(?:\s|$)/.test(match[3]) ? [Number(match[1])] : []
      })
      assert.ok(children.length <= 1, 'fixture must own exactly one handler')
      return children.length ? identity(children[0]) : null
    }, 2000, 'owned compiled handler was not observed')
    assert.equal(handler.parent, worker.child.pid)
    locker = launch(exe('psql'), pgArgs, directory, 'pipe')
    locker.child.stdin.write(`BEGIN; SELECT id FROM public._brain_jobs WHERE id='${job.id}' FOR UPDATE; SELECT 'LOCK_READY:' || pg_backend_pid();\n`)
    const lockerPID = await observe(() => {
      assert.ok(alive(locker.child) && locker.bytes <= 65536, 'owned lock client failed')
      return Number(locker.output.match(/LOCK_READY:(\d+)/)?.[1]) || null
    }, 3000, 'owned row lock was not acquired')
    databasePID = await observe(() => {
      assert.ok(alive(worker.child), 'worker exited before the renewal was blocked')
      return Number(sql(`SELECT pid FROM pg_stat_activity WHERE datname='${database}' AND application_name='${application}' AND wait_event_type='Lock' AND query LIKE 'UPDATE _brain_jobs SET lease_until=%' AND ${lockerPID}=ANY(pg_blocking_pids(pid))`)) || null
    }, 6000, 'no live renewal waited on the owned row lock')
    assert.equal(identity(databasePID)?.parent, postmasterPID, 'database waiter must belong to our disposable postmaster')
    assert.notEqual(databasePID, worker.child.pid)
    // Otherwise the normal 2 s database lock timeout could hide an unbounded pool close.
    process.kill(databasePID, 'SIGSTOP')
    suspendedOwnedDatabase.add(databasePID)
    assert.ok(alive(worker.child) && same(handler, identity(handler.pid)), 'worker and handler must remain alive after suspending the renewal')
    assert.equal(sql(`SELECT count(*) FROM pg_stat_activity WHERE pid=${databasePID} AND datname='${database}' AND application_name='${application}' AND state='active' AND wait_event_type='Lock' AND query LIKE 'UPDATE _brain_jobs SET lease_until=%' AND ${lockerPID}=ANY(pg_blocking_pids(pid))`), '1',
      'the same suspended renewal must still be active and waiting on the owned locker immediately before shutdown')
    assert.ok(alive(worker.child) && same(handler, identity(handler.pid)), 'the renewal deadline must not expire before the shutdown signal')
    const signaledAt = Date.now()
    process.kill(worker.child.pid, 'SIGTERM')
    await waitExit(worker.child, 3000)
    const elapsed = Date.now() - signaledAt
    assert.ok(!alive(worker.child), 'SQL-blocked worker did not exit autonomously within 3000 ms')
    assert.equal(worker.child.signalCode, null, 'forced termination never counts as passing')
    const events = worker.output.split('\n').flatMap(line => { try { return [JSON.parse(line)] } catch { return [] } })
    assert.ok(events.some(event => event.event === 'worker-lease-renewal-deadline'), 'renewal cancellation needs a diagnosed deadline')
    const closeDeadline = events.some(event => event.event === 'worker-pool-close-deadline' && event.phase === 'pool-close' && event.private === true)
    assert.ok(worker.child.exitCode === 0 || (worker.child.exitCode === 2 && closeDeadline), 'exit 2 requires the specific private pool-close deadline event')
    await observe(() => !same(handler, identity(handler.pid)), 1000, 'handler survived worker shutdown')
    process.kill(databasePID, 'SIGCONT')
    suspendedOwnedDatabase.delete(databasePID)
    databasePID = undefined
    await observe(() => sql(`SELECT count(*) FROM pg_stat_activity WHERE datname='${database}' AND application_name='${application}'`) === '0',
      2000, 'worker or listener database connections survived shutdown')
    locker.child.stdin.end('ROLLBACK;\n')
    await waitExit(locker.child, 1000)
    assert.equal(locker.child.exitCode, 0, 'owned lock transaction did not roll back cleanly')
    const state = JSON.parse(sql(`SELECT json_build_object('state',state,'attempts',attempts,'lease',lease_token IS NOT NULL) FROM public._brain_jobs WHERE id='${job.id}'`))
    assert.deepEqual(state, { state: 'running', attempts: 1, lease: true }, 'an uncertain renewal must not ack, release or replay')
    console.log(`Worker lifecycle gate: PASS (observed renewal waiter, unresponsive owned database connection, autonomous exit ${elapsed} ms, closed handler/listener, uncertain lease retained; no forced KILL)`)
  } finally {
    if (databasePID && suspendedOwnedDatabase.has(databasePID)) {
      try { process.kill(databasePID, 'SIGCONT') } catch {}
      suspendedOwnedDatabase.delete(databasePID)
    }
    if (alive(locker?.child)) { locker.child.stdin.end('ROLLBACK;\n'); await waitExit(locker.child, 500) }
    if (worker) { await terminateOwned(worker.child); auxiliary.delete(worker.child) }
    if (locker) { await terminateOwned(locker.child); auxiliary.delete(locker.child) }
    await cleanupHandler()
    auxiliaryCleanup.delete(cleanupHandler)
  }
}
