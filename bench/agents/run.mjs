import { readFile } from 'node:fs/promises'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { run } from './runner.mjs'

if (process.argv.length !== 4 || process.argv.includes('--help')) {
  console.log('Usage: node bench/agents/run.mjs config.json fixtures.json')
  process.exit(process.argv.includes('--help') ? 0 : 2)
}
try {
  const [config, fixtures] = await Promise.all(process.argv.slice(2).map(async path => JSON.parse(await readFile(path, 'utf8'))))
  if (!Array.isArray(fixtures) || !fixtures.length || fixtures.length > 20) throw Error('fixtures must contain 1..20 tasks')
  const result = await run(config, fixtures, resolve(dirname(fileURLToPath(import.meta.url)), '../..'))
  console.log(JSON.stringify({ runId: result.runId, stable: result.stable, passed: result.results.filter(r => r.passed).length, total: result.results.length, out: config.out }))
  if (!result.stable || result.results.some(r => !r.passed)) process.exitCode = 1
} catch (error) { console.error(error.message); process.exitCode = 2 }
