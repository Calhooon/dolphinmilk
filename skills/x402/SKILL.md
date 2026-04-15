---
name: x402
description: "Discover and call paid x402 services — images, video, search, storage, transcription, inscriptions. Handles BRC-31 auth, 402 payment, async polling, multi-step flows, and refunds automatically. USE THIS when the task involves generating images, generating video, searching X/Twitter, uploading files, transcribing audio, inscribing data, or calling any paid HTTP API."
auto_activate: true
tools: [discover_services, discover_endpoints, x402_call]
---

# x402 Services

**COST WARNING: Every failed x402_call POST wastes real satoshis. There are no free retries. Read this entire skill before calling any paid service.**

Three tools. Two are free. One costs money.

| Tool | Cost | Purpose |
|------|------|---------|
| `discover_services` | FREE | Browse the registry for available services |
| `discover_endpoints` | FREE | Get a service's manifest: endpoints, pricing, schemas, provider tips |
| `x402_call` | PAID | Call any endpoint with auto auth + payment |

## Step 1: List Available Services

```json
discover_services({"category": "image"})
```

Use the optional `category` filter: "image", "video", "search", "storage", "audio", "inscription".

## Step 2: Discover the Service (MANDATORY — DO NOT SKIP)

```json
discover_endpoints({"agent": "banana"})
```

This is FREE. It returns the full manifest AND provider tips. Read the entire response. Note:
- **Required fields** and their types
- **Delivery mode** — "synchronous", "async-poll", or "two-step"
- **Key Constraints** — hard limits that cause failures if violated
- **Validation Rules** — pre-checked before payment

**DO NOT call x402_call without first calling discover_endpoints.** Guessing parameters costs money and fails.

## Step 3: Call the Service

```json
x402_call({
  "service": "agent_name/endpoint_path",
  "method": "POST",
  "parameters": { "field1": "value1", "field2": "value2" }
})
```

- `service`: agent shorthand (e.g. `"banana/generate"`) or full URL
- `method`: defaults to POST. Use GET only when the manifest says to.
- `parameters`: the request body. Construct from the manifest schema you read in Step 2.

**POST with no parameters is rejected before payment.** Pre-flight validation catches bad parameters (e.g. `retentionPeriod: 60` when minimum is 180) before any money is spent.

### Async Services (delivery: "async-poll")

Some services return a job ID immediately. You must poll for the result.

1. POST via x402_call — response has `id` or `prediction_id` (PAID)
2. Wait 15-20 seconds
3. GET the status via x402_call using the **full URL**: `{"service": "https://full-url.com/status/{id}", "method": "GET"}` (FREE)
4. Check `status`: "succeeded"/"completed" = done. "failed"/"canceled" = stop. Anything else = wait and poll again.
5. Max 12 polls (~3 minutes) for images, 20 polls (~5 minutes) for video.

**Always use the full URL for polling**, not shorthand like `banana/status/{id}`.

### Two-Step Services (e.g. NanoStore)

1. **Reserve** via x402_call (PAID) — returns `uploadURL` + `requiredHeaders`
2. **Upload** via execute_bash with curl (FREE) — PUT file bytes to the presigned URL

```
# Step 1: Reserve (PAID)
x402_call({"service": "nanostore/upload", "method": "POST", "parameters": {"fileSize": 1024, "retentionPeriod": 525600}})

# Step 2: Upload bytes (FREE — MANDATORY, Step 1 alone does nothing)
execute_bash({"command": "curl -s -X PUT -H 'Content-Type: application/octet-stream' -H 'x-goog-meta-uploaderidentitykey: VALUE' -H 'x-goog-custom-time: VALUE' --data-binary @file.txt 'UPLOAD_URL'"})
```

Include ALL `requiredHeaders` or GCS returns 403.

## Step 4: Handle Errors

When x402_call fails:

1. **Read the error completely.** It includes provider tips and key constraints.
2. **DO NOT retry the same call.** Each failed POST wastes sats.
3. **Call discover_endpoints again.** Re-read the manifest. Fix your parameters.
4. **Retry once with corrected parameters.** If it fails again with a different error, stop and report.
5. **After 2 consecutive failures, the circuit breaker activates.** You must call discover_endpoints and change your approach.

## Step 5: Track Budget

Every paid call reports `sats_paid` in the response. Track spending.

| Service | Typical cost |
|---------|-------------|
| Image (banana) | ~37K-75K sats |
| Video (veo/kling) | ~200K+ sats |
| X/Twitter search | ~3K-6K sats/page |
| Transcription (whisper) | ~1.3K sats/min |
| NanoStore upload | ~730 sats/MB/year |
| Inscription (1sat) | 200+ sats |

Do not call expensive services repeatedly. One image costs ~$0.25. Do not generate variations unless asked.

## Notes

- `discover_services` and `discover_endpoints` are FREE. Always call them.
- Agent names are case-insensitive.
- Full URLs bypass the registry and work directly in x402_call.
- Refunds are automatic when servers return excess payment.
- For async polling, always use the **full URL**, not shorthand.

## Gotchas

- **Never POST with an empty body.** x402_call rejects empty POST parameters before payment — but if you bypass this with raw calls, you waste sats on a guaranteed failure.
- **Async services require polling.** Image generation (banana) takes 10-60s, video (veo/kling) takes minutes. Don't assume the first response contains your result — check the `status` field and poll with GET.
- **BRC-31 sessions go stale.** On 401 errors, the system auto-retries with a fresh Authrite handshake. If you see persistent 401s, the base URL may be wrong (e.g. using `/upload` instead of `scheme://host`).
- **Read the manifest before calling.** Required vs optional fields, value constraints (e.g. `retentionPeriod` minimum 180), and delivery mode vary per service. Guessing costs money.
