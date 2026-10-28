// Invariant 2 (CONTRIBUTING): the core runtime stays ≤ 4096 B gzip. Measured on the source without comments, which is
// an upper bound of what brain-min ships (the minifier only removes more), so this test is stricter than the budget.
// A page that only fetches data must not pull the form module either.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { gzipSync } from 'node:zlib'

const read = f => readFileSync(new URL('../' + f, import.meta.url), 'utf8')
const strip = s => s.split('\n').map(l => l.replace(/^\s*\/\/.*$/, '').replace(/\s+\/\/[^"'`]*$/, '')).join('\n')
const gz = s => gzipSync(strip(s), { level: 9 }).length

test('core brain.js ≤ 4096 B gzip (comments stripped)', () => {
  const n = gz(read('brain.js'))
  assert.ok(n <= 4096, `brain.js is ${n} B gz: over the 4096 B budget; move the new code to a sibling module`)
})

test('brain-data.js does not import brain-form.js (resource() pages skip the form module)', () => {
  const imports = [...read('brain-data.js').matchAll(/^import .* from '([^']+)'/gm)].map(m => m[1])
  assert.ok(!imports.includes('./brain-form.js'), imports.join(', '))
  assert.ok(imports.includes('./brain-schema.js'))
})
