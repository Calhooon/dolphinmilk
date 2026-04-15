# OpenAI — LLM Chat Completions

Synchronous service. Used internally by think() — you typically don't call this directly.

## When You Would Call This Directly

Normally the agent loop calls `think()` which handles this service automatically. You'd only use `x402_call` with this service if you need:
- A different model than the one configured for the agent loop
- A one-off query with custom parameters
- To call the API without going through the agent loop

## Working Example

```
x402_call({
  "service": "openai/chat",
  "parameters": {
    "model": "gpt-5-mini",
    "messages": [
      {"role": "user", "content": "Summarize this in one sentence: ..."}
    ],
    "max_tokens": 500
  }
})
```

## Valid Models

Check the manifest for the current list. As of last check:
- `gpt-5-nano` (cheapest)
- `gpt-5-mini`
- `gpt-5.2`
- `o4-mini`

Do NOT use bare `gpt-5` — it's not a valid model name. Always use the specific variant.

## Reasoning Models

`gpt-5.2` and `o4-mini` are reasoning models:
- Use `max_completion_tokens` instead of `max_tokens`
- Do NOT set `temperature` (it's not supported for reasoning models)

## Cost

- Varies by model and token count. `gpt-5-nano` is cheapest, `gpt-5.2` is most expensive.
- Cost is dynamic based on input + output tokens.

## Key Constraints
- Do NOT use bare "gpt-5" — use specific variant like "gpt-5-mini" or "gpt-5-nano".
- Reasoning models (gpt-5.2, o4-mini): use max_completion_tokens, NOT max_tokens. Do NOT set temperature.
- messages array is required with at least one message object.
- model is required. Check manifest for current valid model names.

## Validation Rules
- model required | model is required (e.g. "gpt-5-mini", "gpt-5-nano")
- messages required | messages array is required with at least one message

## Common Errors

- **400 `ERR_INVALID_MODEL`**: Model name wrong. Use exact names like `gpt-5-mini`, not `gpt-5`.
- **400 `ERR_OPENAI_REJECTED`**: The request body has an issue — check message format, parameter names.
