#!/usr/bin/env node
// A real compiled job identifies invalid local list pagination without dumping arbitrary stderr values.
import assert from 'node:assert/strict'
import { createServer } from 'node:http'
import { join } from 'node:path'
import { binary, brain, createDatabase, gate, own, sleep, stop, write } from './lib.mjs'
await gate('job-diagnostics', 3 * 60 * 1000, async directory => {
  const database = createDatabase('gate_job_diagnostics')
  let backendRequests = 0
  const backend = createServer((request, response) => {
    backendRequests++
    request.resume()
    response.writeHead(422, { 'content-type': 'application/json' })
    response.end(JSON.stringify({ error: { code: 'INVALID_INPUT', message: 'synthetic-private-diagnostic-value', fields: { limit: 'limit must be 1..100' } } }))
  })
  let worker
  try {
    await new Promise((resolve, reject) => backend.once('error', reject).listen(0, '127.0.0.1', resolve))
    const env = { DATABASE_URL: database, BRAIN_BACKEND_URL: `http://127.0.0.1:${backend.address().port}`, BRAIN_JOBS_API_KEY: 'bk_live_' + 'a'.repeat(12) + '_' + 'b'.repeat(64), BRAIN_MAIL_RESEND_KEY: '', GOOGLE_CLIENT_ID: '' }
    brain(['new', 'app', '--json'], directory, env)
    const app = join(directory, 'app')
    write(app, '.brain_env', 'PUBLIC_SITE_URL=https://example.test\nBRAIN_BACKEND_URL?=\nBRAIN_JOBS_API_KEY?=\n')
    brain(['add', 'job', 'badlist', '--fields', 'value:str', '--json'], app, env)
    write(app, 'server/jobs/badlist/queue.jsbrain', "<meta server>\nexport default { attempts: 1 }\n")
    write(app, 'server/jobs/badlist/handler.jsbrain', "<meta server>\nexport default async function handle(ctx) { console.error('synthetic-private-diagnostic-value'); await ctx.backend.resource('items').list({ limit: 500 }) }\n")
    brain(['backend', 'migrate', '--json'], app, env)
    const queued = JSON.parse(brain(['jobs', 'enqueue', 'badlist', '--payload', '{}', '--json'], app, env))
    worker = own(binary, ['jobs', 'work', 'badlist', '--concurrency', '1'], app, env)
    let failure
    const deadline = Date.now() + 15000
    while (Date.now() < deadline) {
      const events = worker.output.text.split('\n').flatMap(line => { try { return [JSON.parse(line)] } catch { return [] } })
      failure = events.find(event => event.event === 'job-failed' && event.jobId === queued.id)
      if (failure) break
      assert.equal(worker.child.exitCode, null, 'the worker must remain alive until its observed job failure')
      await sleep(50)
    }
    assert.ok(failure, 'the worker must emit its job-failed event within the bound')
    assert.equal(failure.backend.status, 400, 'local client input validation preserves its existing HTTP status')
    assert.equal(failure.backend.code, 'INVALID_INPUT')
    assert.equal(typeof failure.backend.fields?.limit, 'string', 'the local failure identifies its declared input field')
    assert.match(failure.backend.fields.limit, /100/, 'the static diagnostic names the limit bound')
    assert.equal(backendRequests, 0, 'invalid local pagination must fail before any backend request')
    assert.ok(failure.trace?.location?.file?.endsWith('handler.jsbrain'), 'the source location must identify the owned handler')
    assert.ok(!JSON.stringify(failure).includes('synthetic-private-diagnostic-value'), 'arbitrary stderr values stay out of the structured event')
    assert.ok(!worker.output.text.includes('synthetic-private-diagnostic-value'), 'the worker must never echo raw handler stderr')
    let status
    const recordedBy = Date.now() + 3000
    do {
      status = JSON.parse(brain(['jobs', 'status', queued.id, '--json'], app, env))
      if (status.job.state === 'dead') break
      await sleep(20)
    } while (Date.now() < recordedBy)
    assert.equal(status.job.state, 'dead', 'the one-attempt fixture must record its failed attempt')
    console.log('Job diagnostics coverage: local invalid pagination retains status, stable code, bounded field diagnostic and handler location without backend requests or arbitrary stderr')
  } finally {
    if (worker) await stop(worker.child)
    backend.closeAllConnections()
    if (backend.listening) await new Promise(resolve => backend.close(resolve))
  }
})
