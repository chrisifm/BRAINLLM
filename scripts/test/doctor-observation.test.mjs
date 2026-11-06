import { test } from 'node:test'
import assert from 'node:assert/strict'
import { hasDoctorAdvice } from '../gates/doctor-observation.mjs'

const completed = report => ({ status: 0, signal: null, stdout: JSON.stringify(report) })
test('healthy completed reports distinguish queue advice presence and absence', () => {
  assert.equal(hasDoctorAdvice(completed({ ok: true, advice: [{ code: 'B0765' }] }), 'B0765'), true)
  assert.equal(hasDoctorAdvice(completed({ ok: true, advice: [] }), 'B0765'), false)
})
test('codes outside the advice array are not queue findings', () => {
  assert.equal(hasDoctorAdvice(completed({ ok: true, advice: [], other: { code: 'B0765' } }), 'B0765'), false)
})
test('failed or incomplete doctor runs cannot prove either liveness outcome', () => {
  for (const result of [
    { status: null, signal: 'SIGTERM', error: { code: 'ETIMEDOUT' }, stdout: '' },
    { ...completed({ ok: true, advice: [{ code: 'B0765' }] }), status: null, signal: 'SIGTERM', error: { code: 'ENOBUFS' } },
    { ...completed({ ok: false, advice: [{ code: 'B0765' }] }), status: 1 },
    { status: 0, signal: null, stdout: '{truncated' },
    completed({ ok: true }), completed({ ok: false, advice: [] }), completed({ ok: true, advice: [null] }),
  ]) assert.throws(() => hasDoctorAdvice(result, 'B0765'), /doctor/i)
})
test('unhealthy report diagnostics name finite gates without quoting arbitrary report values', () => {
  const result = { ...completed({ ok: false, failed: ['tests', 'private-value-from-report'], advice: [] }), status: 1 }
  assert.throws(() => hasDoctorAdvice(result, 'B0765'), error => {
    assert.match(error.message, /failed=tests/)
    assert.ok(!error.message.includes('private-value-from-report'))
    return true
  })
})
