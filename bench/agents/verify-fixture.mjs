// Additional fixture acceptance checks. Compilation is a separate explicit validator.
import assert from 'node:assert/strict'
import { readFile, readdir } from 'node:fs/promises'
import { join } from 'node:path'
import { execute } from './runner.mjs'
let input = ''
for await (const chunk of process.stdin) {
  input += chunk
  if (input.length > 2 * 1024 * 1024) throw Error('Validator input exceeds limit')
}
const { task } = JSON.parse(input)
if (task.files['KEEP.txt'] != null) assert.equal(await readFile('KEEP.txt', 'utf8'), task.files['KEEP.txt'])
const paths = (await readdir('views')).filter(p => p.endsWith('.brain')).sort()
const source = (await Promise.all(paths.map(p => readFile(join('views', p), 'utf8')))).join('\n')
assert.match(source, /<main(?:\s|>)/)
assert.match(source, /<h1(?:\s|>)/)
if (task.kind === 'create') {
  assert.match(source, /Task board/)
  assert.match(source, /\{#each\b[^}]*\([^)]*\)/)
  assert.match(source, /(?:label=|<label)/)
  assert.match(source, /on:(?:click|submit)/)
} else if (task.kind === 'edit') {
  assert.match(source, /\bcount\b/)
  assert.match(source, /\bstep\b/)
  assert.match(source, /(?:label=|<label)/)
} else if (task.kind === 'repair') {
  assert.doesNotMatch(source, /missingValue/)
  assert.match(source, /\bcount\b/)
  assert.match(source, /on:click/)
} else if (task.kind === 'CRUD') {
  for (const pattern of [/\.post\(/, /\.put\(/, /\.delete\(/, /offset/, /search/, /\{#await/, /:catch/, /:empty/, /role="status"/, /disabled=/]) assert.match(source, pattern)
  assert.equal(await readFile('server/resources/notes/policy.jsbrain', 'utf8'), task.files['server/resources/notes/policy.jsbrain'])
} else if (task.kind === 'atomic-conflict') {
  const proof = await readFile('conflict-check.mjs', 'utf8')
  assert.match(proof, /BRAIN_BINARY/)
  assert.match(proof, /(?:spawn|execFile)/)
  process.env.BRAIN_BINARY = process.argv[2]
  assert.ok(process.env.BRAIN_BINARY, 'Supply the explicit binary as validator argument')
  const result = await execute([process.execPath, 'conflict-check.mjs'], process.cwd(), '', { timeoutMs: 20000, maxOutputBytes: 65536 })
  assert.equal(result.code, 0, result.stderr)
  assert.equal(result.reason, null)
}
console.log(JSON.stringify({ passed: true, scope: 'source-acceptance; compilation and live behavior require separate validators', kind: task.kind }))
