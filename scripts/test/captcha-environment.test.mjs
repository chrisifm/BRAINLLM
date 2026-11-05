// Public provider test fixtures remain local; the doctor/check warning inspects the production selection safely.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from 'node:fs'
import { join } from 'node:path'
import { tmpdir } from 'node:os'
const brain = process.env.BRAIN_BIN ?? new URL('../../target/debug/brain', import.meta.url).pathname
test('check warns on a public production captcha fixture without exposing it and respects environment overrides', { timeout: 60000 }, () => {
  const root = mkdtempSync(join(tmpdir(), 'brain-captcha-environment-'))
  try {
    const created = spawnSync(brain, ['new', 'site', '--json'], { cwd: root, encoding: 'utf8', timeout: 30000 })
    assert.equal(created.status, 0, created.stdout + created.stderr)
    const app = join(root, 'site'), dummy = '1x0000000000000000000000000000000AA'
    mkdirSync(join(app, 'server/auth'), { recursive: true })
    writeFileSync(join(app, 'server/auth/captcha.jsbrain'), "<meta server>\nexport default { verify_url: 'https://challenges.cloudflare.com/turnstile/v0/siteverify', secret_env: 'TEST_CAPTCHA_PROVIDER', protect: ['signup'] }\n")
    const check = () => {
      const result = spawnSync(brain, ['check', '--json'], { cwd: app, encoding: 'utf8', timeout: 30000 })
      assert.equal(result.status, 0, result.stdout + result.stderr)
      assert.ok(!result.stdout.includes(dummy))
      return JSON.parse(result.stdout)
    }
    writeFileSync(join(app, '.brain_env'), `TEST_CAPTCHA_PROVIDER=${dummy}\n`)
    const warning = check().find(diagnostic => diagnostic.code === 'B0790' && diagnostic.msg.includes('public provider test secret'))
    assert.equal(warning?.level, 'warning')
    assert.match(warning.msg, /\[production\]/)
    assert.match(warning.fix, /provider-issued/)
    writeFileSync(join(app, '.brain_env'), `[development]\nTEST_CAPTCHA_PROVIDER=${dummy}\n[production]\nTEST_CAPTCHA_PROVIDER=synthetic-provider-production-value\n`)
    assert.ok(!check().some(diagnostic => diagnostic.code === 'B0790' && diagnostic.msg.includes('public provider test secret')))
  } finally { rmSync(root, { recursive: true, force: true }) }
})
