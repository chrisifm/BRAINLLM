# guide: llm — call Anthropic or OpenAI from server code with `ctx.llm`

## When
A server function, `load`, route or job needs text from a hosted model and the key must stay on the server. `ctx.llm` needs no package. For tools, images, structured output or another provider, use an official SDK from the integration catalog. For a model on the user's device, see `brain guide ai`.

## What exists
- Configuration: `ANTHROPIC_API_KEY` and `ANTHROPIC_MODEL`, or `OPENAI_API_KEY` and `OPENAI_MODEL`. No model is built in: pass `{ model }` or set the variable, or the call fails B0921. With both keys, pass `{ provider }` or set `BRAIN_LLM_PROVIDER`. `ANTHROPIC_BASE_URL` / `OPENAI_BASE_URL` point at a gateway or a local test provider (https, or http on loopback only).
- Input: `prompt` or `messages: [{ role: 'user' | 'assistant', content: 'text' }]`; `system` is separate. `maxTokens` defaults to 1024. There are no sampling options: the current Anthropic API version rejects them.
- Time: `timeoutMs` defaults to 60000 (at most 600000) and `ctx.signal` also cancels the call. A `server:` function or `load` still ends at brain.json `server.timeoutMs` (default 10000): raise it, or the deadline stops the call first.
- Streaming: `const s = ctx.llm.stream(o); for await (const text of s) { … }; const r = await s.result`. It streams to server code only. An RPC answers one JSON value and BRAINLLM has no server route that emits events, so **streaming to the browser is not available**: return the final text, or publish parts through a native socket you already run.
- Failures: `err.code` B0921 (configuration or bad options, also a key the provider refuses) or B0922 (provider failure); `err.status` is 400, 429, 499, 502, 503 or 504. A 429 reaches the client with a safe message; 5xx stay generic. `err.detail` holds the provider's text with the key removed; keep it on the server. No automatic retries: a repeated call can cost money.

## Steps
1. Add the key and the model to `.brain_env`, not `PUBLIC_` variables.
2. Call `ctx.llm.complete` inside `server:` or `load`, never in browser code.
3. Raise brain.json `server.timeoutMs` above the slowest answer you accept.
4. Show the waiting and failure states in the view: a model call takes seconds and can fail.
5. Test against a local fake provider with `ANTHROPIC_BASE_URL` or `OPENAI_BASE_URL`; a test must not spend money.

## Verify
`brain check` reports B0920 and B0923. After `brain build`, no client file holds a key: search `dist/_b` for your key name.

## Common mistakes
- `ctx.llm` or a `PUBLIC_` provider key in browser code: B0920, and `process.env.ANTHROPIC_API_KEY` there is B0702.
- `ctx.llm` with no provider key declared: warning B0923.
- No model configured: B0921 when the call runs. No model is built in.
- A call that always ends at 10 seconds: `server.timeoutMs` stopped it.
- Reading `err.detail` into the page: it holds provider text, so keep it on the server.

## Minimal correct example
```js
server: async function ask(ctx, q) {
  const r = await ctx.llm.complete({ system: 'Answer briefly.', prompt: q, maxTokens: 300 })
  return r.text // also: stopReason ('end' | 'length' | 'filter' | 'other'), usage { inputTokens, outputTokens }, model, provider, requestId
}
```
