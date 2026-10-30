import test from 'node:test'
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'

const release = readFileSync(new URL('../../.github/workflows/release.yml', import.meta.url), 'utf8')
const verify = readFileSync(new URL('../../.github/workflows/verify.yml', import.meta.url), 'utf8')
function condition(job) {
  const block = release.match(new RegExp(`^  ${job}:\\n([\\s\\S]*?)(?=^  [a-z][a-z-]*:|$(?![\\s\\S]))`, 'm'))?.[1]
  assert.ok(block, `missing ${job} job`)
  const expression = block.match(/^    if: (.+)$/m)?.[1]
  assert.ok(expression, `missing ${job} condition`)
  // Evaluate the checked-in boolean expression, with only documented GitHub context/functions.
  const evaluate = new Function('github', 'inputs', 'needs', 'always', 'cancelled', 'startsWith', `return (${expression})`)
  return { block, evaluate: (context) => evaluate(context.github, context.inputs, context.needs, () => true, () => context.canceled, (value, prefix) => value.startsWith(prefix)) }
}
function context(overrides = {}) {
  return { github: { event_name: 'push', ref: 'refs/tags/v0.4.1' }, inputs: { publish: false, artifact_run: '' }, needs: { pack: { result: 'success' }, 'verify-source': { result: 'success' } }, canceled: false, ...overrides }
}

test('publisher requires successful source verification even for reviewed package reuse', () => {
  const { evaluate, block } = condition('publish')
  assert.match(block, /^    needs: \[pack, verify-source\]$/m)
  for (const source of ['failure', 'cancelled', 'skipped', '']) {
    for (const artifact_run of ['', '41113503223', 'native:123:456']) {
      const candidate = context({ inputs: { publish: true, artifact_run }, needs: { pack: { result: 'success' }, 'verify-source': { result: source } } })
      assert.equal(evaluate(candidate), false, `source=${source}, artifact=${artifact_run}`)
    }
  }
  assert.equal(evaluate(context()), true)
  assert.equal(evaluate(context({ github: { event_name: 'workflow_dispatch', ref: 'refs/tags/v0.4.1' }, inputs: { publish: true, artifact_run: '41113503223' }, needs: { pack: { result: 'skipped' }, 'verify-source': { result: 'success' } } })), true)
})

test('cancellation and manual pack-only dispatch cannot initiate verification or publication', () => {
  for (const job of ['verify-source', 'publish']) {
    const { evaluate } = condition(job)
    assert.equal(evaluate(context({ canceled: true })), false)
    for (const ref of ['refs/tags/v0.4.1', 'refs/heads/main']) {
      assert.equal(evaluate(context({ github: { event_name: 'workflow_dispatch', ref } })), false)
    }
    assert.equal(evaluate(context({ github: { event_name: 'push', ref: 'refs/heads/main' } })), false)
  }
})

test('release reuses the same-commit source gates without inheriting publication credentials', () => {
  const { block } = condition('verify-source')
  assert.match(block, /^    uses: \.\/\.github\/workflows\/verify\.yml$/m)
  assert.match(verify, /^  workflow_call:/m)
  assert.match(block, /^      contents: read$/m)
  assert.doesNotMatch(block, /secrets:|id-token:|environment:/)
})
