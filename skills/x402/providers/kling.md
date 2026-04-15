# Kling — Video Generation (Multiple Models)

Async service with 24+ model variants. Pay on POST, poll for result (free). EXPENSIVE.

## Working Example

```
# 1. First, check available models — run discover_endpoints to see the full list.
discover_endpoints({"agent": "kling"})

# 2. Generate video (PAID)
x402_call({
  "service": "kling/generate",
  "parameters": {
    "prompt": "a worm navigating a maze",
    "model": "kling-v2-master",
    "duration": 5
  }
})

# 3. SAVE the prediction ID from the response.

# 4. Poll (FREE):
x402_call({
  "service": "https://kling.x402agency.com/status/{prediction_id}",
  "method": "GET"
})
```

## Critical Details

- Kling has 24+ model variants. Always run `discover_endpoints` first to see current models.
- Different models have different costs, quality levels, and capabilities (text-to-video, image-to-video).
- Use the full URL for polling: `https://kling.x402agency.com/status/{id}`
- Video generation can take 2-10 minutes depending on the model and duration.

## Cost

- Varies significantly by model variant. Check manifest for current pricing.
- Budget-conscious: use shorter durations and simpler models first.

## Key Constraints
- Kling has 24+ model variants. Always run discover_endpoints first to see current models.
- prompt is required.
- Poll using the FULL URL (https://kling.x402agency.com/status/{id}), not shorthand.
- Video generation can take 2-10 minutes. Budget varies significantly by model.

## Validation Rules
- prompt required | prompt is required for video generation

## Common Errors

- **400 "invalid model"**: Run `discover_endpoints` to see valid model names. They change.
- **Timeout**: Some models take much longer. Poll for up to 10 minutes.
