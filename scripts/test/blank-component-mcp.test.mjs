// The native MCP generator exposes the same minimal template and rejects ignored options.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { mkdtempSync, readFileSync, rmSync, existsSync } from 'node:fs'
import { join } from 'node:path'
import { tmpdir } from 'node:os'
const brain = process.env.BRAIN_BIN ?? new URL('../../target/debug/brain', import.meta.url).pathname
test('MCP creates minimal typed components and rejects blank outside comp', { timeout: 60000 }, () => {
  const root = mkdtempSync(join(tmpdir(), 'brain-blank-component-mcp-'))
  try {
    const created = spawnSync(brain, ['new', 'site', '--static', '--json'], { cwd: root, encoding: 'utf8', timeout: 30000 })
    assert.equal(created.status, 0, created.stdout + created.stderr)
    const app = join(root, 'site')
    const requests = [
      { jsonrpc: '2.0', id: 0, method: 'initialize', params: { protocolVersion: '2025-11-25', capabilities: {}, clientInfo: { name: 'blank-component-test', version: '1' } } },
      { jsonrpc: '2.0', method: 'notifications/initialized' },
      { jsonrpc: '2.0', id: 1, method: 'tools/call', params: { name: 'brain_add', arguments: { kind: 'comp', name: 'ContentPanel', props: 'title:str', blank: true } } },
      { jsonrpc: '2.0', id: 2, method: 'tools/call', params: { name: 'brain_add', arguments: { kind: 'view', name: 'rejected', blank: true } } },
    ]
    const run = spawnSync(brain, ['mcp'], { cwd: app, input: requests.map(JSON.stringify).join('\n') + '\n', encoding: 'utf8', timeout: 30000 })
    assert.equal(run.status, 0, run.stdout + run.stderr)
    const responses = run.stdout.trim().split('\n').map(JSON.parse)
    const success = responses.find(response => response.id === 1)
    assert.ok(success?.result && !success.result.isError && !success.error, JSON.stringify(success))
    assert.ok(responses.find(response => response.id === 2)?.result?.isError)
    const source = readFileSync(join(app, 'comp/content-panel.brain'), 'utf8')
    assert.ok(source.includes('{title}') && source.includes('<slot />') && !source.includes('<script>'))
    assert.ok(existsSync(join(app, 'comp/content-panel.test.brain')))
    assert.ok(!existsSync(join(app, 'views/rejected.brain')))
  } finally { rmSync(root, { recursive: true, force: true }) }
})
