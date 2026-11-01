// Server LLM module (INTERNALS §51): `ctx.llm`, one provider-neutral call for Anthropic and OpenAI. Zero dependencies.
// Provider contracts reviewed 2026-10-31 against the official documentation:
// - Anthropic: platform.claude.com/docs/en/api/messages, /api/versioning, /api/errors, /build-with-claude/streaming.
//   API version 2026-10-21 rejects temperature, top_p and top_k with a 400, so the neutral API has no sampling options.
// - OpenAI: developers.openai.com/api/reference/resources/responses/methods/create, /api/docs/guides/streaming-responses
//   and the event schemas of github.com/openai/openai-openapi (Responses API, stored off with `store: false`).
// Keys come only from the server environment. An error never carries a key; provider text stays in `detail`, redacted.
const LLM_PROVIDERS = {
  anthropic: { key: 'ANTHROPIC_API_KEY', model: 'ANTHROPIC_MODEL', base: 'ANTHROPIC_BASE_URL', origin: 'https://api.anthropic.com' },
  openai: { key: 'OPENAI_API_KEY', model: 'OPENAI_MODEL', base: 'OPENAI_BASE_URL', origin: 'https://api.openai.com' },
}
const LLM_ANTHROPIC_VERSION = '2026-10-21'
const LLM_DEFAULT_TIMEOUT = 60000, LLM_MAX_TIMEOUT = 600000, LLM_DEFAULT_MAX_TOKENS = 1024
const LLM_MAX_BODY = 32 * 1024 * 1024, LLM_MAX_LINE = 4 * 1024 * 1024, LLM_MAX_DETAIL = 500

export class LlmError extends Error {
  constructor(code, status, message, fix, extra = {}) {
    super(message)
    this.name = 'LlmError'
    this.code = code
    this.status = status
    this.fix = fix
    Object.assign(this, extra)
  }
}
const llmConfig = (message, fix) => new LlmError('B0921', 503, message, fix)
const llmFailure = (status, message, extra) => new LlmError('B0922', status, message, 'Inspect err.provider, err.providerStatus and err.detail on the server', extra)

function llmResolve(env, o) {
  if (!o || typeof o !== 'object') throw llmConfig('ctx.llm needs an options object', 'Pass { messages: [{ role: "user", content: "…" }] } or { prompt: "…" }')
  const requested = o.provider ?? env.BRAIN_LLM_PROVIDER
  if (requested != null && requested !== '' && !Object.hasOwn(LLM_PROVIDERS, requested)) throw llmConfig(`Unknown LLM provider "${String(requested).slice(0, 40)}"`, 'Use "anthropic" or "openai"')
  const present = Object.keys(LLM_PROVIDERS).filter(name => env[LLM_PROVIDERS[name].key])
  const provider = requested || (present.length === 1 ? present[0] : null)
  if (!provider) throw llmConfig(present.length ? 'Both LLM provider keys are set and none is selected' : 'No LLM provider key is set', present.length ? 'Pass { provider } or set BRAIN_LLM_PROVIDER in .brain_env' : 'Set ANTHROPIC_API_KEY or OPENAI_API_KEY in .brain_env (never with PUBLIC_)')
  const p = LLM_PROVIDERS[provider], key = env[p.key]
  if (!key) throw llmConfig(`${p.key} is not set`, `Set ${p.key} in .brain_env (never with PUBLIC_)`)
  const model = o.model ?? env[p.model]
  if (typeof model !== 'string' || !model) throw llmConfig(`No model selected for ${provider}`, `Pass { model } or set ${p.model} in .brain_env; BRAINLLM pins no model`)
  let base
  try { base = new URL(env[p.base] || p.origin) } catch { throw llmConfig(`${p.base} is not a URL`, `Set ${p.base} to an https address`) }
  const loopback = base.hostname === '127.0.0.1' || base.hostname === 'localhost' || base.hostname === '[::1]'
  if (base.protocol !== 'https:' && !(base.protocol === 'http:' && loopback)) throw llmConfig(`${p.base} must use https`, 'Only a loopback address may use plain http')
  if (base.username || base.password) throw llmConfig(`${p.base} must not carry credentials`, `Keep the key in ${p.key}`)
  const messages = o.messages ?? (typeof o.prompt === 'string' ? [{ role: 'user', content: o.prompt }] : null)
  if (!Array.isArray(messages) || !messages.length) throw new LlmError('B0921', 400, 'ctx.llm needs messages or prompt', 'Pass { messages: [{ role: "user", content: "…" }] }')
  for (const m of messages) if (!m || (m.role !== 'user' && m.role !== 'assistant') || typeof m.content !== 'string') throw new LlmError('B0921', 400, 'Each message needs role "user" or "assistant" and text content', 'The system prompt goes in { system }')
  if (o.system != null && typeof o.system !== 'string') throw new LlmError('B0921', 400, 'system must be text', 'Pass { system: "…" }')
  const maxTokens = o.maxTokens ?? LLM_DEFAULT_MAX_TOKENS
  if (!Number.isSafeInteger(maxTokens) || maxTokens < 1) throw new LlmError('B0921', 400, 'maxTokens must be a positive integer', `Omit it for ${LLM_DEFAULT_MAX_TOKENS}`)
  const timeoutMs = o.timeoutMs ?? LLM_DEFAULT_TIMEOUT
  if (!Number.isSafeInteger(timeoutMs) || timeoutMs < 1 || timeoutMs > LLM_MAX_TIMEOUT) throw new LlmError('B0921', 400, `timeoutMs must be between 1 and ${LLM_MAX_TIMEOUT}`, `Omit it for ${LLM_DEFAULT_TIMEOUT}`)
  return { provider, key, model, root: base.href.replace(/\/+$/, ''), messages, system: o.system ?? null, maxTokens, timeoutMs, signal: o.signal }
}

function llmRequest(r, stream) {
  if (r.provider === 'anthropic') return {
    url: r.root + '/v1/messages',
    headers: { 'x-api-key': r.key, 'anthropic-version': LLM_ANTHROPIC_VERSION, 'content-type': 'application/json' },
    body: { model: r.model, max_tokens: r.maxTokens, messages: r.messages, ...(r.system ? { system: r.system } : {}), ...(stream ? { stream: true } : {}) },
  }
  return {
    url: r.root + '/v1/responses',
    headers: { authorization: 'Bearer ' + r.key, 'content-type': 'application/json' },
    body: { model: r.model, max_output_tokens: r.maxTokens, input: r.messages, store: false, ...(r.system ? { instructions: r.system } : {}), ...(stream ? { stream: true } : {}) },
  }
}

const llmStop = reason => ({ end_turn: 'end', stop_sequence: 'end', completed: 'end', max_tokens: 'length', max_output_tokens: 'length', content_filter: 'filter', refusal: 'filter' })[reason] || 'other'
const llmRedact = (text, key) => String(text ?? '').split(key).join('[key]').slice(0, LLM_MAX_DETAIL)

function llmParsed(r, json, requestId) {
  if (r.provider === 'anthropic') {
    if (!Array.isArray(json?.content)) throw llmFailure(502, 'The model provider sent an unexpected answer', { provider: r.provider })
    return { provider: r.provider, model: json.model ?? r.model, text: json.content.filter(b => b?.type === 'text').map(b => b.text).join(''), stopReason: llmStop(json.stop_reason), usage: { inputTokens: json.usage?.input_tokens ?? null, outputTokens: json.usage?.output_tokens ?? null }, requestId }
  }
  if (json?.status === 'failed') throw llmFailure(502, 'The model provider could not complete the request', { provider: r.provider, detail: llmRedact(json.error?.message, r.key) })
  if (!Array.isArray(json?.output)) throw llmFailure(502, 'The model provider sent an unexpected answer', { provider: r.provider })
  const text = json.output.filter(i => i?.type === 'message').flatMap(i => i.content || []).filter(c => c?.type === 'output_text').map(c => c.text).join('')
  return { provider: r.provider, model: json.model ?? r.model, text, stopReason: llmStop(json.incomplete_details?.reason ?? json.status), usage: { inputTokens: json.usage?.input_tokens ?? null, outputTokens: json.usage?.output_tokens ?? null }, requestId }
}

async function llmStatusError(r, res) {
  let detail = ''
  try { const raw = await res.text(); try { const j = JSON.parse(raw); detail = j?.error?.message ?? raw } catch { detail = raw } } catch {}
  const providerStatus = res.status, extra = { provider: r.provider, providerStatus, detail: llmRedact(detail, r.key) }
  if (providerStatus === 429) return llmFailure(429, 'The model provider is rate limiting requests; try again shortly', { ...extra, retryAfter: res.headers.get('retry-after') })
  if (providerStatus === 401 || providerStatus === 403) return new LlmError('B0921', 503, `The model provider refused the key in ${LLM_PROVIDERS[r.provider].key}`, 'Check the key and its permissions', extra)
  return llmFailure(providerStatus === 504 || providerStatus === 408 ? 504 : 502, 'The model provider could not complete the request', extra)
}

async function llmFetch(r, stream, fallback) {
  const deadline = AbortSignal.timeout(r.timeoutMs)
  const signal = AbortSignal.any([deadline, ...(fallback ? [fallback] : []), ...(r.signal ? [r.signal] : [])])
  const q = llmRequest(r, stream)
  const failed = e => deadline.aborted ? llmFailure(504, 'The model provider did not answer in time', { provider: r.provider }) : signal.aborted ? llmFailure(499, 'The LLM call was cancelled', { provider: r.provider, cancelled: true }) : e instanceof LlmError ? e : llmFailure(502, 'The model provider could not be reached', { provider: r.provider })
  let res
  try { res = await fetch(q.url, { method: 'POST', headers: q.headers, body: JSON.stringify(q.body), signal, redirect: 'error' }) } catch (e) { throw failed(e) }
  return { res, failed, requestId: res.headers.get('request-id') ?? res.headers.get('x-request-id') }
}

async function llmReadAll(res) {
  const reader = res.body.getReader(), parts = []
  let total = 0
  for (;;) {
    const { done, value } = await reader.read()
    if (done) break
    total += value.length
    if (total > LLM_MAX_BODY) { await reader.cancel(); throw llmFailure(502, 'The model provider answer is too large') }
    parts.push(value)
  }
  return Buffer.concat(parts).toString('utf8')
}

// One `data:` payload per server-sent event; the event name is repeated inside each payload by both providers.
async function* llmEvents(res) {
  const decoder = new TextDecoder(), reader = res.body.getReader()
  let buffer = '', data = []
  try {
    for (;;) {
      const { done, value } = await reader.read()
      if (done) return
      buffer += decoder.decode(value, { stream: true })
      if (buffer.length > LLM_MAX_LINE) throw llmFailure(502, 'The model provider sent an oversized event')
      for (let at; (at = buffer.indexOf('\n')) >= 0;) {
        const line = buffer.slice(0, at).replace(/\r$/, '')
        buffer = buffer.slice(at + 1)
        if (line === '') { if (data.length) { const raw = data.join('\n'); data = []; let e; try { e = JSON.parse(raw) } catch { throw llmFailure(502, 'The model provider sent an unreadable event') } yield e } }
        else if (line.startsWith('data:')) data.push(line.slice(5).trimStart())
      }
    }
  } finally { await reader.cancel().catch(() => {}) }
}

function llmStream(r, parent) {
  let settle, reject
  const result = new Promise((a, b) => { settle = a; reject = b })
  result.catch(() => {}) // reading `.result` is optional; the iterator throws the same failure
  const run = async function* () {
    let opened
    try { opened = await llmFetch(r, true, parent) } catch (e) { reject(e); throw e }
    const { res, failed, requestId } = opened
    const out = { provider: r.provider, model: r.model, text: '', stopReason: 'other', usage: { inputTokens: null, outputTokens: null }, requestId }
    let finished = false
    try {
      if (!res.ok) throw await llmStatusError(r, res)
      for await (const e of llmEvents(res)) {
        let piece = ''
        if (r.provider === 'anthropic') {
          if (e.type === 'error') throw e.error?.type === 'rate_limit_error' ? llmFailure(429, 'The model provider is rate limiting requests; try again shortly', { provider: r.provider, detail: llmRedact(e.error?.message, r.key) }) : llmFailure(502, 'The model provider failed during the answer', { provider: r.provider, detail: llmRedact(e.error?.message, r.key) })
          if (e.type === 'message_start') { out.model = e.message?.model ?? out.model; out.usage.inputTokens = e.message?.usage?.input_tokens ?? null; out.usage.outputTokens = e.message?.usage?.output_tokens ?? null }
          else if (e.type === 'content_block_delta' && e.delta?.type === 'text_delta') piece = e.delta.text
          else if (e.type === 'message_delta') { out.stopReason = llmStop(e.delta?.stop_reason); if (e.usage?.output_tokens != null) out.usage.outputTokens = e.usage.output_tokens }
          else if (e.type === 'message_stop') finished = true
        } else {
          if (e.type === 'error') throw llmFailure(502, 'The model provider failed during the answer', { provider: r.provider, detail: llmRedact(e.message, r.key) })
          if (e.type === 'response.failed') throw llmFailure(502, 'The model provider could not complete the request', { provider: r.provider, detail: llmRedact(e.response?.error?.message, r.key) })
          if (e.type === 'response.output_text.delta') piece = e.delta
          else if (e.type === 'response.completed' || e.type === 'response.incomplete') {
            finished = true
            out.model = e.response?.model ?? out.model
            out.stopReason = llmStop(e.response?.incomplete_details?.reason ?? e.response?.status)
            out.usage = { inputTokens: e.response?.usage?.input_tokens ?? null, outputTokens: e.response?.usage?.output_tokens ?? null }
          }
        }
        if (piece) { out.text += piece; yield piece }
      }
      if (!finished) throw llmFailure(502, 'The model provider ended the answer early', { provider: r.provider })
      settle(out)
    } catch (e) { const f = failed(e); reject(f); throw f }
  }
  const iterator = run()
  // Leaving the loop early closes the provider connection; `.result` then rejects as cancelled.
  const stop = iterator.return.bind(iterator)
  return {
    result,
    [Symbol.asyncIterator]() { return this },
    next: () => iterator.next(),
    return: value => { reject(llmFailure(499, 'The LLM stream was closed before its end', { provider: r.provider, cancelled: true })); return stop(value) },
  }
}

// ctx.llm for one request: `signal` is ctx.signal, so a closed client or a passed deadline stops the provider call.
export function llmClient(env, signal) {
  return {
    async complete(o) {
      const r = llmResolve(env, o)
      const { res, failed, requestId } = await llmFetch(r, false, signal)
      try {
        if (!res.ok) throw await llmStatusError(r, res)
        let json
        try { json = JSON.parse(await llmReadAll(res)) } catch (e) { if (e instanceof LlmError) throw e; if (e instanceof SyntaxError) throw llmFailure(502, 'The model provider sent an unreadable answer', { provider: r.provider }); throw e }
        return llmParsed(r, json, requestId)
      } catch (e) { throw failed(e) }
    },
    // Validates now, connects on the first read. `for await (const text of s)`, then `await s.result`.
    stream(o) { return llmStream(llmResolve(env, o), signal) },
  }
}
