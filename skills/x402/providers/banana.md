# Banana — Image Generation

Async service. You pay on POST, then poll for the result (free).

## Working Example

```
# 1. Generate (PAID)
x402_call({
  "service": "banana/generate",
  "parameters": {
    "prompt": "a neon worm on a circuit board",
    "resolution": "1K"
  }
})
# Response includes: { "id": "abc123", "status": "starting", "poll_url": "/status/abc123" }

# 2. SAVE the prediction ID from the response "id" field immediately.

# 3. Wait 15-20 seconds, then poll (FREE — auth only, no payment):
x402_call({
  "service": "https://nano-banana-pro.x402agency.com/status/abc123",
  "method": "GET"
})

# 4. Check "status" field:
#    "succeeded" → done, image URL is in output[0] or output.url
#    "starting" / "processing" → wait 15s, poll again
#    "failed" / "canceled" → stop, report error

# 5. Max 12 polls (~3 minutes). Usually completes in ~2 minutes.
```

## Critical Details

- The response `id` field is the prediction_id. Use it in the poll URL.
- Poll URL format: `https://nano-banana-pro.x402agency.com/status/{id}` — use the FULL URL, not `banana/status/{id}`.
- Polling is identity-scoped: only the identity key that paid can poll.
- Image URL is typically in `output[0]` when status is `"succeeded"`.

## Valid Parameters

- `resolution`: `"1K"`, `"2K"`, `"4K"` — NOT pixel dimensions like "1024x1024"
- `aspect_ratio`: `"1:1"`, `"16:9"`, `"9:16"`, `"4:3"`, `"3:4"` (optional)
- `output_format`: `"jpg"` or `"png"` (optional)
- `image_input`: array of image URLs for image-to-image (optional)

## Cost

- 1K/2K: ~$0.19 (~37K sats at typical BSV price)
- 4K: ~$0.38 (~75K sats)
- Sats fluctuate with BSV/USD rate. Check the manifest for current pricing.

## Key Constraints
- resolution must be "1K", "2K", or "4K" — NOT pixel dimensions like "1024x1024".
- prompt is required. Without it, the request fails.
- Poll using the FULL URL (https://nano-banana-pro.x402agency.com/status/{id}), not shorthand.
- Image URL is in output[0] when status is "succeeded".

## Validation Rules
- prompt required | prompt is required for image generation
- resolution in [1K,2K,4K] | resolution must be "1K", "2K", or "4K"

## Common Errors

- **400 "invalid resolution"**: Use `"1K"`, `"2K"`, or `"4K"`, not pixel dimensions.
- **400 on poll**: You're using `banana/status/...` instead of the full URL. Use `https://nano-banana-pro.x402agency.com/status/{id}`.
- **Polling returns same status repeatedly**: Generation takes ~2 minutes. Don't give up after 30 seconds.
- **Empty output after "succeeded"**: Check `output[0]` not just `output`. The output field is an array.
