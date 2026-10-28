#!/usr/bin/env node
// Read-only inventory. Framework detection is metadata-based; route candidates are hints, never a transpilation.
import { readFileSync, readdirSync, lstatSync } from 'node:fs'
import { join, resolve } from 'node:path'
import { pathToFileURL } from 'node:url'

export function plan(root) {
  const pkg = JSON.parse(readFileSync(join(root, 'package.json'), 'utf8'))
  const deps = { ...pkg.dependencies, ...pkg.devDependencies }
  const frameworks = [['react', 'React'], ['@angular/core', 'Angular'], ['vue', 'Vue'], ['svelte', 'Svelte'], ['next', 'Next.js'], ['nuxt', 'Nuxt']].filter(([key]) => deps[key]).map(([, name]) => name)
  const files = [], skipped = [], queue = ['']
  let entries = 0
  while (queue.length) {
    const dir = queue.shift()
    for (const e of readdirSync(join(root, dir), { withFileTypes: true }).sort((a, b) => a.name.localeCompare(b.name))) {
      if (++entries > 20_000) throw new Error('Inventory exceeds 20000 entries. Run per app/package, not a whole monorepo.')
      const rel = dir ? `${dir}/${e.name}` : e.name
      if (e.name.startsWith('.') || /^(?:node_modules|dist|build|coverage|vendor|target)$/.test(e.name)) continue
      if (e.isSymbolicLink()) { skipped.push(rel); continue }
      if (e.isDirectory()) { if (dir.split('/').length >= 20) throw new Error('Inventory depth exceeds 20.'); queue.push(rel); continue }
      if (/\.(?:jsx?|tsx?|vue|svelte|html|css|scss)$/.test(e.name)) files.push(rel)
    }
  }
  const routes = files.filter(f => /(?:^|\/)(?:routes?|pages|views)(?:\/|\.)|(?:app|router|routing)[.-]/i.test(f))
  const tests = files.filter(f => /(?:^|\/)(?:__tests__|tests?|e2e)(?:\/)|\.(?:test|spec)\./.test(f))
  return {
    schemaVersion: 1, mode: 'incremental-migration', frameworks: frameworks.length ? frameworks : ['unknown'],
    files, routeCandidates: routes, existingTests: tests, skippedSymlinks: skipped,
    caveat: 'File-name heuristics only. Verify dynamic routing, generated code, aliases and behavior by reading the source. No source file or package script was executed.',
    steps: [
      { id: 'baseline', status: 'pending', action: 'Run existing tests in the original project; capture route, UI, API, auth and accessibility behavior with synthetic data.' },
      { id: 'target', status: 'pending', action: 'Create a separate brain new target; read brain spec and brain map --json. Keep the original project intact.' },
      { id: 'slice', status: 'pending', action: 'Choose one leaf component or route. Extract input/output, state transitions, API effects and failure cases into a parity checklist.' },
      { id: 'port', status: 'pending', action: 'Port that slice using only served SPEC primitives. Keep backend contracts and visual behavior. Record unsupported semantics instead of guessing.' },
      { id: 'verify', status: 'pending', action: 'Run original and target behavioral tests, brain check --json, brain doctor and brain build. Compare UI, network effects, loading/error/empty states and navigation.' },
      { id: 'cutover', status: 'pending', action: 'Switch only a verified route through an explicit routing boundary. Record rollback and evidence before continuing; never let two routers own the same DOM root.' },
    ],
    ledger: { slices: [], requiredFields: ['source', 'target', 'behavior', 'tests', 'evidence', 'status', 'rollback'] },
    prompt: 'Read docs/MIGRATION.md and the target brain spec first. Use this inventory as hints. Migrate one verified slice at a time, preserve frontend logic and backend contracts, and update the ledger with actual evidence. If BRAINLLM cannot express a behavior, record the gap and use docs/EVOLUTION.md to propose a tested compiler improvement; do not silently drop the behavior or weaken tests.',
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  try {
    if (!process.argv[2] || process.argv[2] === '--help') console.log('Usage: node scripts/migrate-plan.mjs /path/to/original-app > migration-plan.json')
    else {
      const root = resolve(process.argv[2])
      if (lstatSync(join(root, 'package.json')).isSymbolicLink()) throw new Error('package.json must be a regular local file.')
      console.log(JSON.stringify(plan(root), null, 2))
    }
  } catch (e) { console.log(JSON.stringify({ schemaVersion: 1, ok: false, error: e.message })); process.exitCode = 1 }
}
