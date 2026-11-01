import test from 'node:test'
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { disagreements, releaseNotes } from '../github-release.mjs'

const changelog = '# Changelog\n\nIntro.\n\n## [0.2.0] - Unreleased\n\n- two\n\n## [0.1.0] - 2026-01-01\n\n### Fixed\n\n- one\n'

test('release notes hold only the exact version section', () => {
  assert.equal(releaseNotes(changelog, '0.2.0'), '- two\n')
  assert.equal(releaseNotes(changelog, '0.1.0'), '### Fixed\n\n- one\n')
})

test('release notes refuse a missing section, an empty one and a non-version', () => {
  assert.throws(() => releaseNotes(changelog, '0.3.0'), /no section/)
  assert.throws(() => releaseNotes('## [1.0.0]\n\n## [0.9.0]\n- x\n', '1.0.0'), /empty/)
  assert.throws(() => releaseNotes(changelog, '0.1.0] - 2026'), /not an exact version/)
  assert.throws(() => releaseNotes(changelog, '0.1'), /not an exact version/)
})

test('agreement needs the tag, npm latest and the latest GitHub Release on one version', () => {
  const good = { version: '0.8.0', tag: 'v0.8.0', npmLatest: '0.8.0', releaseTag: 'v0.8.0' }
  assert.deepEqual(disagreements(good), [])
  for (const [field, value, name] of [['tag', '', 'git tag'], ['npmLatest', '0.7.0', 'npm latest'], ['releaseTag', 'v0.7.0', 'GitHub Release']]) {
    const found = disagreements({ ...good, [field]: value })
    assert.equal(found.length, 1)
    assert.match(found[0], new RegExp(name))
  }
})

test('the workflow gives contents: write only to the GitHub Release job, after a successful publish', () => {
  const release = readFileSync(new URL('../../.github/workflows/release.yml', import.meta.url), 'utf8')
  const writes = [...release.matchAll(/contents: write/g)]
  assert.equal(writes.length, 1)
  const job = release.match(/^  github-release:\n([\s\S]*?)(?=^  [a-z][a-z-]*:\n|(?![\s\S]))/m)?.[1]
  assert.ok(job, 'missing github-release job')
  assert.match(job, /contents: write/)
  assert.doesNotMatch(job, /id-token/)
  assert.match(job, /needs: publish/)
  assert.match(job, /needs\.publish\.result == 'success'/)
  assert.match(job, /persist-credentials: false/)
  // the agreement check is the job's final step
  const steps = job.split('\n      - ')
  assert.match(steps.at(-1), /github-release\.mjs agree/)
  // values reach shell through env, never by expression expansion inside a script
  for (const script of job.matchAll(/run: \|\n([\s\S]*?)(?=\n      - |\n?$(?![\s\S]))/g)) assert.doesNotMatch(script[1], /\$\{\{/)
})
