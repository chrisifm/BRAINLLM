import { test } from 'node:test'
import assert from 'node:assert/strict'
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, rmSync } from 'node:fs'
import { join, resolve, dirname } from 'node:path'
import { tmpdir } from 'node:os'
import { fileURLToPath } from 'node:url'
import { buildIndex, summary, unlinkedBackendPages } from '../gen-docs-index.mjs'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '../..')

test('docs/INDEX.md is up to date (fix: node scripts/gen-docs-index.mjs)', () => {
  assert.equal(readFileSync(join(root, 'docs/INDEX.md'), 'utf8'), buildIndex(root))
})

test('every docs/backend page is linked from docs/BACKEND.md', () => {
  assert.deepEqual(unlinkedBackendPages(root), [])
})

test('summary takes one sentence of the first prose paragraph', () => {
  assert.equal(summary(['', '```sh', 'brain run', '```', 'Shop docs. More text.']), 'Shop docs.')
  assert.equal(summary(['- `a` item.', '  continued', '## Next', 'Not this.']), '`a` item.')
  assert.equal(summary(['<!--planned:#1-->', 'Hidden.', '<!--/planned-->', 'Shown [link](x.md) | pipe.']), 'Shown link \\| pipe.')
})

test('new docs/backend pages appear with no generator edits and must be linked', () => {
  const dir = mkdtempSync(join(tmpdir(), 'brain-docs-index-'))
  try {
    mkdirSync(join(dir, 'docs/backend'), { recursive: true })
    mkdirSync(join(dir, 'crates/brain-cli/guides'), { recursive: true })
    writeFileSync(join(dir, 'SPEC.md'), '# Spec\n\n## Files\nViews live in views/.\n')
    writeFileSync(join(dir, 'crates/brain-cli/guides/start.md'), '# guide: start — the working loop\n')
    writeFileSync(join(dir, 'docs/BACKEND.md'), '# Backend\n\nSee [jobs](backend/jobs.md).\n')
    writeFileSync(join(dir, 'docs/backend/jobs.md'), '# Background jobs\n\nDurable queues for a shop.\n')
    writeFileSync(join(dir, 'docs/backend/tasks.md'), '# Task API\n\nCRUD for tasks.\n')
    const index = buildIndex(dir)
    assert.match(index, /\| \[Files\]\(\.\.\/SPEC\.md#files\) \| Views live in views\/\. \|/)
    assert.match(index, /\| \[start\]\(\.\.\/crates\/brain-cli\/guides\/start\.md\) \| the working loop \|/)
    assert.match(index, /\| \[Task API\]\(backend\/tasks\.md\) \| CRUD for tasks\. \|/)
    assert.deepEqual(unlinkedBackendPages(dir), ['docs/backend/tasks.md'])
  } finally {
    rmSync(dir, { recursive: true, force: true })
  }
})
