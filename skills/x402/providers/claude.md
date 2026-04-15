# Claude — LLM Chat Completions

Synchronous service. Used internally by think() — you typically don't call this directly.

## When You Would Call This Directly

Normally the agent loop calls `think()` which handles this service automatically. You'd only use `x402_call` with this service if you need:
- A different model than the one configured for the agent loop
- A one-off query with custom parameters
- To call the Claude API without going through the agent loop

## Working Example

```
x402_call({
  "service": "claude/chat",
  "parameters": {
    "model": "claude-sonnet-4-6",
    "messages": [
      {"role": "user", "content": "Summarize this in one sentence: ..."}
    ],
    "max_tokens": 500
  }
})
```

## Valid Models

Check the manifest for the current list. As of last check:
- `claude-haiku-4-5` (cheapest, 64K max output, 200K context)
- `claude-sonnet-4-6` (balanced, 64K max output, 200K context)
- `claude-opus-4-6` (most capable, 128K max output, 200K context)

## API Format

Claude uses the native Anthropic Messages API format (NOT OpenAI-compatible):
- `messages` array with `role`/`content` (required)
- `model` (required)
- `max_tokens` (required, always — no `max_completion_tokens` distinction)
- `system` as a separate parameter (NOT a system message in the messages array)
- `temperature` is always supported (0.0–1.0, optional)
- `tools` use `{ "name", "description", "input_schema" }` format (NOT OpenAI function format)
- `tool_choice` is an object `{ "type": "auto" }` (NOT the string `"auto"`)

## Response Format

- `content`: array of blocks (`text`, `tool_use`)
- `stop_reason`: `end_turn`, `tool_use`, `max_tokens`, `stop_sequence`
- `usage`: `input_tokens`, `output_tokens` (NOT `prompt_tokens`/`completion_tokens`)

## Cost

Dynamic per-token pricing with 25% margin:
- `claude-haiku-4-5`: $1.0/M input, $5.0/M output
- `claude-sonnet-4-6`: $3.0/M input, $15.0/M output
- `claude-opus-4-6`: $5.0/M input, $25.0/M output

Pre-payment covers the ceiling (max_tokens of output). Unused tokens auto-refund (threshold >= 100 sats).

## Key Constraints
- model is required. Use exact names: `claude-haiku-4-5`, `claude-sonnet-4-6`, `claude-opus-4-6`.
- messages array is required with at least one message.
- max_tokens is required (1–128,000).
- Do NOT put system messages in the messages array — use the `system` parameter.

## Common Errors

- **400 `invalid_request_error`**: Check message format, parameter names. Claude API differs from OpenAI.
- **400 model not found**: Use exact model names listed above.
