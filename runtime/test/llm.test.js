// ctx.llm against a local fake provider: wire contract of both providers, keys, timeouts and failures. No paid API.
import test from 'node:test'
import assert from 'node:assert/strict'
import { createServer } from 'node:http'
import { llmClient, LlmError } from '../server/llm.mjs'

const KEY = 'synthetic-provider-key-value'
function fake(handler) {
  const seen = []
  const server = createServer((req, res) => {
    let raw = ''
    req.on('data', c => (raw += c))
    req.on('end', () => { seen.push({ url: req.url, headers: req.headers, body: raw ? JSON.parse(raw) : null }); handler(req, res, seen.at(-1)) })
  })
  return new Promise(ok => server.listen(0, '127.0.0.1', () => ok({ seen, url: `http://127.0.0.1:${server.address().port}`, close: () => { server.closeAllConnections(); server.close() } })))
}
const json = (res, status, body, headers = {}) => { res.writeHead(status, { 'content-type': 'application/json', ...headers }); res.end(JSON.stringify(body)) }
const sse = (res, events) => { res.writeHead(200, { 'content-type': 'text/event-stream' }); for (const e of events) res.write(`event: ${e.type}\ndata: ${JSON.stringify(e)}\n\n`); res.end() }
const anthropicEnv = url => ({ ANTHROPIC_API_KEY: KEY, ANTHROPIC_MODEL: 'configured-model', ANTHROPIC_BASE_URL: url })
const openaiEnv = url => ({ OPENAI_API_KEY: KEY, OPENAI_MODEL: 'configured-model', OPENAI_BASE_URL: url })
const failure = async (p, check) => { const e = await p.then(() => assert.fail('expected a failure'), e => e); assert.ok(e instanceof LlmError); assert.ok(!JSON.stringify({ ...e, message: e.message }).includes(KEY), 'no error carries the key'); check(e) }

test('complete speaks the Anthropic Messages contract', async t => {
  const f = await fake((req, res) => json(res, 200, { model: 'served-model', content: [{ type: 'thinking', thinking: 'hidden' }, { type: 'text', text: 'Hello' }, { type: 'text', text: ' there' }], stop_reason: 'end_turn', usage: { input_tokens: 3, output_tokens: 2 } }, { 'request-id': 'req_1' }))
  t.after(f.close)
  const out = await llmClient(anthropicEnv(f.url)).complete({ system: 'Be brief', messages: [{ role: 'user', content: 'Hi' }], maxTokens: 50 })
  assert.deepEqual(out, { provider: 'anthropic', model: 'served-model', text: 'Hello there', stopReason: 'end', usage: { inputTokens: 3, outputTokens: 2 }, requestId: 'req_1' })
  const q = f.seen[0]
  assert.equal(q.url, '/v1/messages')
  assert.equal(q.headers['x-api-key'], KEY)
  assert.equal(q.headers['anthropic-version'], '2026-10-21')
  assert.deepEqual(q.body, { model: 'configured-model', max_tokens: 50, messages: [{ role: 'user', content: 'Hi' }], system: 'Be brief' })
})

test('complete speaks the OpenAI Responses contract', async t => {
  const f = await fake((req, res) => json(res, 200, { model: 'served-model', status: 'incomplete', incomplete_details: { reason: 'max_output_tokens' }, output: [{ type: 'reasoning' }, { type: 'message', content: [{ type: 'output_text', text: 'Partial' }] }], usage: { input_tokens: 4, output_tokens: 5 } }))
  t.after(f.close)
  const out = await llmClient(openaiEnv(f.url)).complete({ system: 'Be brief', prompt: 'Hi', maxTokens: 9 })
  assert.equal(out.text, 'Partial')
  assert.equal(out.stopReason, 'length')
  assert.deepEqual(out.usage, { inputTokens: 4, outputTokens: 5 })
  const q = f.seen[0]
  assert.equal(q.url, '/v1/responses')
  assert.equal(q.headers.authorization, 'Bearer ' + KEY)
  assert.deepEqual(q.body, { model: 'configured-model', max_output_tokens: 9, input: [{ role: 'user', content: 'Hi' }], store: false, instructions: 'Be brief' })
})

test('stream yields Anthropic text deltas and ignores thinking and pings', async t => {
  const f = await fake((req, res) => sse(res, [
    { type: 'message_start', message: { model: 'served-model', usage: { input_tokens: 7, output_tokens: 1 } } }, { type: 'ping' },
    { type: 'content_block_delta', index: 0, delta: { type: 'thinking_delta', thinking: 'hidden' } },
    { type: 'content_block_delta', index: 1, delta: { type: 'text_delta', text: 'Hel' } }, { type: 'content_block_delta', index: 1, delta: { type: 'text_delta', text: 'lo' } },
    { type: 'message_delta', delta: { stop_reason: 'max_tokens' }, usage: { output_tokens: 15 } }, { type: 'message_stop' }]))
  t.after(f.close)
  const s = llmClient(anthropicEnv(f.url)).stream({ prompt: 'Hi' })
  const parts = []
  for await (const p of s) parts.push(p)
  assert.deepEqual(parts, ['Hel', 'lo'])
  const r = await s.result
  assert.equal(r.text, 'Hello'); assert.equal(r.stopReason, 'length'); assert.equal(r.model, 'served-model')
  assert.deepEqual(r.usage, { inputTokens: 7, outputTokens: 15 })
  assert.equal(f.seen[0].body.stream, true)
})

test('stream yields OpenAI text deltas', async t => {
  const f = await fake((req, res) => sse(res, [{ type: 'response.created' }, { type: 'response.output_text.delta', delta: 'A' }, { type: 'response.output_text.delta', delta: 'B' }, { type: 'response.completed', response: { model: 'served-model', status: 'completed', usage: { input_tokens: 1, output_tokens: 2 } } }]))
  t.after(f.close)
  const s = llmClient(openaiEnv(f.url)).stream({ prompt: 'Hi' })
  let text = ''
  for await (const p of s) text += p
  assert.equal(text, 'AB')
  assert.equal((await s.result).stopReason, 'end')
})

test('a failure inside the stream and an early end both reject', async t => {
  const events = [[{ type: 'message_start', message: {} }, { type: 'error', error: { type: 'overloaded_error', message: 'Overloaded ' + KEY } }], [{ type: 'content_block_delta', delta: { type: 'text_delta', text: 'x' } }]]
  const f = await fake((req, res) => sse(res, events.shift()))
  t.after(f.close)
  const read = async s => { for await (const _ of s); }
  await failure(read(llmClient(anthropicEnv(f.url)).stream({ prompt: 'Hi' })), e => { assert.equal(e.code, 'B0922'); assert.ok(e.detail.includes('[key]')) })
  const early = llmClient(anthropicEnv(f.url)).stream({ prompt: 'Hi' })
  await failure(read(early), e => assert.match(e.message, /early/))
  await failure(early.result, e => assert.equal(e.status, 502))
})

test('the timeout aborts a provider that never answers, and ctx.signal cancels', async t => {
  const f = await fake(() => {})
  t.after(f.close)
  await failure(llmClient(anthropicEnv(f.url)).complete({ prompt: 'Hi', timeoutMs: 40 }), e => { assert.equal(e.status, 504); assert.equal(e.code, 'B0922') })
  const c = new AbortController()
  setTimeout(() => c.abort(), 20)
  await failure(llmClient(anthropicEnv(f.url), c.signal).complete({ prompt: 'Hi' }), e => assert.equal(e.cancelled, true))
})

test('provider statuses map to public-safe failures without the key', async t => {
  const answers = [[429, { error: { message: 'slow down' } }], [401, { error: { message: 'bad key ' + KEY } }], [500, { error: { message: 'boom ' + KEY } }]]
  const f = await fake((req, res) => { const [s, b] = answers.shift(); json(res, s, b, { 'retry-after': '3' }) })
  t.after(f.close)
  const llm = llmClient(anthropicEnv(f.url))
  await failure(llm.complete({ prompt: 'Hi' }), e => { assert.equal(e.status, 429); assert.equal(e.retryAfter, '3') })
  await failure(llm.complete({ prompt: 'Hi' }), e => { assert.equal(e.code, 'B0921'); assert.equal(e.status, 503); assert.ok(!e.message.includes('bad key')) })
  await failure(llm.complete({ prompt: 'Hi' }), e => { assert.equal(e.status, 502); assert.ok(!e.message.includes('boom')); assert.ok(e.detail.includes('[key]')) })
})

test('configuration: provider choice, no pinned model, safe base URL, valid input', async () => {
  const none = llmClient({})
  await failure(none.complete({ prompt: 'x' }), e => { assert.equal(e.code, 'B0921'); assert.match(e.fix, /ANTHROPIC_API_KEY/) })
  await failure(llmClient({ ANTHROPIC_API_KEY: KEY }).complete({ prompt: 'x' }), e => assert.match(e.message, /No model/))
  await failure(llmClient({ ANTHROPIC_API_KEY: KEY, OPENAI_API_KEY: KEY, OPENAI_MODEL: 'm' }).complete({ prompt: 'x' }), e => assert.match(e.message, /Both/))
  await failure(llmClient({ OPENAI_API_KEY: KEY, OPENAI_MODEL: 'm', OPENAI_BASE_URL: 'http://evil.test' }).complete({ prompt: 'x' }), e => assert.match(e.message, /https/))
  await failure(llmClient({ OPENAI_API_KEY: KEY, OPENAI_MODEL: 'm', BRAIN_LLM_PROVIDER: 'constructor' }).complete({ prompt: 'x' }), e => assert.match(e.message, /Unknown/))
  const ok = llmClient({ OPENAI_API_KEY: KEY, OPENAI_MODEL: 'm' })
  await failure(ok.complete({ messages: [{ role: 'system', content: 'x' }] }), e => assert.equal(e.status, 400))
  await failure(ok.complete({ prompt: 'x', timeoutMs: 0 }), e => assert.equal(e.status, 400))
  assert.throws(() => ok.stream({}), LlmError)
})
