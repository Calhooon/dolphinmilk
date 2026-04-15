# x402 Manifest Input Schema Specification

> How to write `/.well-known/x402-info` input schemas that AI agents can consume without guessing.

## Why This Matters

Every ambiguity in a manifest costs real money:
- The agent guesses wrong parameters → HTTP 400 → wasted LLM tokens (~95K sats per retry)
- After 2 failures, the circuit breaker blocks further attempts entirely
- A single missing `enum` field can turn a $0.005 API call into a $0.36 debugging session

The manifest is consumed by AI agents, not human developers. Structured schemas give the LLM exactly the information it needs to construct valid requests on the first attempt.

## Recommended Format

Use structured JSON Schema objects for every input field:

```json
{
  "endpoints": [
    {
      "path": "/leaderboard",
      "method": "POST",
      "description": "Top traders with profit and volume metrics",
      "auth": true,
      "delivery": "synchronous",
      "payment": { "satoshis": 32691 },
      "input": {
        "contentType": "application/json",
        "schema": {
          "category": {
            "type": "string",
            "enum": ["OVERALL", "POLITICS", "SPORTS", "CRYPTO", "CULTURE", "ECONOMICS", "TECH", "FINANCE"],
            "default": "OVERALL",
            "description": "Market category to filter by"
          },
          "limit": {
            "type": "integer",
            "minimum": 1,
            "maximum": 50,
            "default": 25,
            "description": "Number of results to return"
          },
          "order_by": {
            "type": "string",
            "enum": ["PNL", "VOL"],
            "default": "PNL",
            "description": "Sort order: profit-and-loss or volume"
          },
          "time_period": {
            "type": "string",
            "enum": ["DAY", "WEEK", "MONTH", "ALL"],
            "default": "WEEK",
            "description": "Time window for metrics"
          }
        }
      }
    }
  ]
}
```

## Field Reference

Each property in the schema should include:

| Field | Required | Purpose |
|-------|----------|---------|
| `type` | **Yes** | JSON Schema type: `string`, `integer`, `number`, `boolean`, `array`, `object` |
| `description` | Strongly recommended | What this field does — helps the LLM choose correct values |
| `enum` | When constrained | Exhaustive list of valid values — prevents guessing |
| `default` | When has default | Default value if omitted — agent can skip optional fields |
| `required` | When mandatory | `true` if field must be present (per-field boolean, not top-level array) |
| `minimum`/`maximum` | For numeric ranges | Hard bounds on numeric values |
| `items` | For arrays | Schema of array elements |
| `format` | Optional | Hint: `"date-time"`, `"uri"`, `"email"`, etc. |

## Acceptable Variants

The parser handles three structured formats. All are acceptable, but Variant A is preferred:

### Variant A — Wrapped (recommended)
```json
"input": {
  "contentType": "application/json",
  "schema": {
    "prompt": { "type": "string", "required": true, "description": "Image prompt" },
    "resolution": { "type": "string", "enum": ["1K", "2K", "4K"], "default": "2K" }
  }
}
```

### Variant B — Flat
```json
"input": {
  "prompt": { "type": "string", "required": true },
  "resolution": { "type": "string", "enum": ["1K", "2K", "4K"], "default": "2K" }
}
```

### Variant C — Standard JSON Schema
```json
"input": {
  "properties": {
    "prompt": { "type": "string" },
    "resolution": { "type": "string", "enum": ["1K", "2K", "4K"] }
  },
  "required": ["prompt"]
}
```

## What NOT to Do

### Freetext string descriptions

```json
"input": {
  "category": "OVERALL|POLITICS|SPORTS|CRYPTO (default: OVERALL)",
  "limit": "1-50 (default: 25)"
}
```

**Problems:**
- The parser must guess that `|` means enum values and `(default: X)` means a default
- Edge cases are ambiguous: is `"true|false"` a boolean or a string enum?
- No type information — the agent doesn't know if `limit` is a string or integer
- The parser now handles this format (since 2026-03-02), but it's lossy and heuristic-based

**Fix:** Convert each string to a structured object:
```json
"input": {
  "category": {
    "type": "string",
    "enum": ["OVERALL", "POLITICS", "SPORTS", "CRYPTO"],
    "default": "OVERALL"
  },
  "limit": {
    "type": "integer",
    "minimum": 1,
    "maximum": 50,
    "default": 25
  }
}
```

### Missing enum values

```json
"input": {
  "category": { "type": "string", "description": "Category filter" }
}
```

If `category` only accepts specific values, **list them in `enum`**. Without an enum, the agent will try `"all"`, `"general"`, `"default"`, or whatever seems natural — and fail.

### Missing type fields

```json
"input": {
  "limit": { "default": 25 }
}
```

Always include `type`. Without it, the agent doesn't know whether to send `25` (integer) or `"25"` (string).

## Checklist for New Services

Before deploying a new `/.well-known/x402-info` manifest:

- [ ] Every input field has a `type`
- [ ] Constrained fields have `enum` arrays with ALL valid values
- [ ] Optional fields have `default` values
- [ ] Required fields have `"required": true`
- [ ] Numeric fields have `minimum`/`maximum` where applicable
- [ ] Descriptions explain what the field does (not just the field name)
- [ ] Test by fetching the manifest and checking `format_manifest_summary()` output
- [ ] Create a provider tips file at `skills/x402/providers/{name}.md` with a working example

## Validation Test

You can verify your manifest is agent-friendly by running:

```bash
# Fetch and check raw format
curl -s https://your-service/.well-known/x402-info | python3 -c "
import json, sys
data = json.load(sys.stdin)
issues = []
for ep in data.get('endpoints', []):
    path = ep.get('path', '?')
    inp = ep.get('input', {})
    schema = inp.get('schema', inp)
    if isinstance(schema, dict) and 'properties' in schema:
        schema = schema['properties']
    for field, spec in schema.items():
        if field in ('contentType', 'schema', 'type', 'required', 'properties'):
            continue
        if isinstance(spec, str):
            issues.append(f'{path}.{field}: freetext string (should be object)')
        elif isinstance(spec, dict):
            if 'type' not in spec:
                issues.append(f'{path}.{field}: missing type')
if issues:
    print('ISSUES FOUND:')
    for i in issues:
        print(f'  - {i}')
else:
    print('OK: all fields are structured')
"
```

## Cost of Getting It Wrong

Real example (polymirror, 2026-03-02):
- Agent tried to get leaderboard data
- Manifest used freetext strings → agent saw bare field names → guessed `"all"` and `"today"`
- 4 failed attempts across 6 LLM iterations
- Total cost: ~789K sats (~$0.12) for zero useful output
- Fix: 5 minutes to restructure the manifest input schema
