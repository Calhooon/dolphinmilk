---
name: image-generation
description: AI image generation via x402 micropayment
auto_activate: false
tools: [discover_endpoints, x402_call]
---

# Image Generation Skill

Generate images using the banana agent via x402.

## Steps

1. Run `discover_endpoints({"agent": "banana"})` to see available options
2. Call `x402_call({"service": "banana/generate", "parameters": {"prompt": "...", "resolution": "1K"}})`
3. Extract the `prediction_id` from the response
4. Poll: `x402_call({"service": "banana/status/PREDICTION_ID", "method": "GET"})` every 15s
5. When status is "succeeded", the `output` field contains image URLs

Cost: ~1.2M sats (~$0.19) per image. Generation takes ~1-2 minutes.

Parameters:
- `prompt`: Detailed text description. Be specific about style, composition, colors.
- `resolution`: "1K" (default), "2K", or "4K" (~$0.38)

## Gotchas

- **Poll prediction status — don't assume instant completion.** Banana image generation takes 10-60s. The initial POST returns a `prediction_id`, not the image. You must poll with GET until `status` is `"succeeded"`.
- **Prefer the `generate_image` recipe tool over raw `x402_call`.** The recipe tool handles polling internally and returns the final image URL — fewer tool calls, fewer iterations, fewer sats.
- **Check wallet balance before generating.** Each image costs ~37K-75K sats (~$0.19). If funds are low, warn the user before spending.
