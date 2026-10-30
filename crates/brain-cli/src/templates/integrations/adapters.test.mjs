import test from 'node:test'
import assert from 'node:assert/strict'
import { mkdtemp, mkdir, writeFile, readFile, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { pathToFileURL, fileURLToPath } from 'node:url'

const here = fileURLToPath(new URL('.', import.meta.url))
const definitions = {
  openai: { package: 'openai', key: 'OPENAI_API_KEY', model: 'OPENAI_MODEL', declaration: 'export default class Client', method: 'responses', call: 'create', text: 'args.input', response: '{ output_text: text }' },
  claude: { package: '@anthropic-ai/sdk', key: 'ANTHROPIC_API_KEY', model: 'ANTHROPIC_MODEL', declaration: 'export default class Client', method: 'messages', call: 'create', text: 'args.messages[0].content', response: '{ content: [{ type: "thinking", thinking: "omitted" }, { type: "text", text }] }' },
  gemini: { package: '@google/genai', key: 'GEMINI_API_KEY', model: 'GEMINI_MODEL', declaration: 'export class GoogleGenAI', method: 'models', call: 'generateContent', text: 'args.contents', response: '{ text }' },
}
async function fixture(provider) {
  const config = definitions[provider]
  const dir = await mkdtemp(join(tmpdir(), 'brain-sdk-offline-'))
  const moduleDir = join(dir, 'node_modules', ...config.package.split('/'))
  await mkdir(moduleDir, { recursive: true })
  await writeFile(join(moduleDir, 'package.json'), JSON.stringify({ type: 'module', exports: './index.js' }))
  await writeFile(join(moduleDir, 'index.js'), `import assert from 'node:assert/strict';
${config.declaration} {
  constructor(options) {
    assert.equal(options.apiKey, 'private-test-key');
    assert.equal(${provider === 'gemini' ? 'options.httpOptions.timeout' : 'options.timeout'}, 8000);
    ${provider === 'gemini' ? 'assert.equal(options.httpOptions.retryOptions.attempts, 1)' : 'assert.equal(options.maxRetries, 0)'};
    this.${config.method} = { ${config.call}: async args => {
      assert.equal(args.model, 'explicit-test-model');
      const text = ${config.text};
      if (text === 'FAIL') throw new Error('private provider response and prompt');
      if (text === 'WAIT') await new Promise(resolve => globalThis.__brainSDKWait.push(resolve));
      return ${config.response};
    }};
  }
}`)
  const compiled = process.env.BRAIN_SDK_COMPILED_DIR
  const source = await readFile(join(compiled || here, provider + (compiled ? '.mjs' : '.jsbrain')), 'utf8')
  if (!compiled) assert.ok(source.startsWith('<meta server>\n'))
  const module = join(dir, 'adapter.mjs')
  await writeFile(module, compiled ? source : source.slice('<meta server>\n'.length))
  let adapter
  try { adapter = await import(pathToFileURL(module).href) } catch (error) { await rm(dir, { recursive: true, force: true }); throw error }
  const ctx = { user: { id: 'test-user' }, env: { [config.key]: 'private-test-key', [config.model]: 'explicit-test-model' } }
  return { dir, adapter, ctx }
}
for (const provider of Object.keys(definitions)) {
  test(`${provider}: official SDK shape, server auth and safe errors`, { timeout: 5000 }, async () => {
    const { dir, adapter, ctx } = await fixture(provider)
    try {
      assert.deepEqual(await adapter.ask(ctx, 'offline input'), { text: 'offline input', provider })
      await assert.rejects(adapter.ask({ env: ctx.env }, 'input'), /authentication/)
      await assert.rejects(adapter.ask({ user: ctx.user, env: {} }, 'input'), /configure/)
      await assert.rejects(adapter.ask(ctx, 'x'.repeat(16001)), /16000/)
      await assert.rejects(adapter.ask(ctx, 'FAIL'), error => !error.message.includes('private provider') && /provider request failed/.test(error.message))
    } finally { await rm(dir, { recursive: true, force: true }) }
  })
  test(`${provider}: bounded concurrent work and release on settlement`, { timeout: 5000 }, async () => {
    const { dir, adapter, ctx } = await fixture(provider)
    globalThis.__brainSDKWait = []
    const pending = []
    try {
      for (let i = 0; i < 8; i++) pending.push(adapter.ask(ctx, 'WAIT'))
      await assert.rejects(adapter.ask(ctx, 'ninth'), /capacity/)
      for (const resolve of globalThis.__brainSDKWait) resolve()
      await Promise.all(pending)
      assert.equal((await adapter.ask(ctx, 'recovered')).text, 'recovered')
    } finally {
      for (const resolve of globalThis.__brainSDKWait) resolve()
      await Promise.allSettled(pending)
      delete globalThis.__brainSDKWait
      await rm(dir, { recursive: true, force: true })
    }
  })
}
