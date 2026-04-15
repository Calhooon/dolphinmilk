# Veo — Video Generation

Async service. Pay on POST, poll for result (free). EXPENSIVE — check budget first.

## Working Example

```
# 1. Generate video (PAID — expensive!)
x402_call({
  "service": "veo/generate",
  "parameters": {
    "prompt": "a worm crawling through digital circuits, cinematic lighting",
    "duration": 5
  }
})
# Response includes prediction ID and status

# 2. SAVE the prediction ID immediately.

# 3. Wait 30 seconds (video takes longer than images), then poll (FREE):
x402_call({
  "service": "https://veo-3-1-fast.x402agency.com/status/{prediction_id}",
  "method": "GET"
})

# 4. Check status field. Video generation can take 2-5 minutes.
# Poll every 15-20 seconds, max 20 polls.
```

## Critical Details

- Video generation is EXPENSIVE: ~200K+ sats per ~5 seconds of video.
- Always check your budget before generating video.
- Generation takes longer than images — expect 2-5 minutes.
- Use the full URL for polling: `https://veo-3-1-fast.x402agency.com/status/{id}`
- Polling is identity-scoped (only the paying key can poll).

## Cost

- ~$0.19+ per second of video (~200K+ sats for a 5-second clip)
- This is one of the most expensive x402 services. Don't call without good reason.

## Key Constraints
- Video generation is EXPENSIVE: ~200K+ sats per ~5 seconds. Check budget first.
- prompt is required.
- Poll using the FULL URL (https://veo-3-1-fast.x402agency.com/status/{id}), not shorthand.
- Generation takes 2-5 minutes. Poll every 15-20 seconds, max 20 polls.

## Validation Rules
- prompt required | prompt is required for video generation

## Common Errors

- **Budget exceeded**: Check balance before generating. A single video can cost more than several image generations.
- **Timeout on polling**: Video takes longer than images. Increase your polling patience (up to 5 minutes).
