# NanoStore — Content-Addressed File Storage

Two-step flow: reserve storage (paid), then upload bytes to presigned URL (free).

## Endpoints

| Endpoint | Method | Auth | Cost | Purpose |
|----------|--------|------|------|---------|
| `/quote` | POST | none | FREE | Preview the cost before committing |
| `/upload` | POST | BRC-31 | paid | Reserve storage, get presigned upload URL |
| `/list` | GET | BRC-31 | FREE | List your uploaded files |

## Recommended Flow

### Step 0: Get a quote (FREE — no auth, no payment)
```
x402_call({"service": "nanostore/quote", "method": "POST", "parameters": {"fileSize": 1024, "retentionPeriod": 525600}})
→ {"quote": 10}   // cost in satoshis
```
This is optional but recommended — it tells you the exact cost before you pay.

### Step 1: Reserve storage (PAID)
```
x402_call({"service": "nanostore/upload", "method": "POST", "parameters": {"fileSize": 1024, "retentionPeriod": 525600}})
→ {
    "status": "success",
    "uploadURL": "https://storage.googleapis.com/...",
    "requiredHeaders": {"x-goog-meta-uploaderidentitykey": "...", "x-goog-custom-time": "..."},
    "amount": 10
  }
```

### Step 2: PUT file bytes to the presigned URL (FREE — MANDATORY)
**The upload is NOT complete after Step 1!** You MUST PUT the actual file bytes to the `uploadURL` using `execute_bash`. This is a plain HTTP PUT — no x402_call, no auth, no payment.

```
execute_bash({"command": "curl -s -X PUT -H 'Content-Type: text/html' -H 'x-goog-meta-uploaderidentitykey: VALUE_FROM_STEP1' -H 'x-goog-custom-time: VALUE_FROM_STEP1' --data-binary @file.html 'UPLOAD_URL_FROM_STEP1'"})
```

- **Set the correct Content-Type for the file type** — GCS uses it to determine how the file is served. Wrong Content-Type causes browsers to download instead of display. Common types: `text/html`, `application/json`, `text/plain`, `image/png`, `application/pdf`. Use `application/octet-stream` only for raw binary data.
- Replace `VALUE_FROM_STEP1` with the actual values from `requiredHeaders` in the Step 1 response.
- Replace `UPLOAD_URL_FROM_STEP1` with the `uploadURL` from the Step 1 response.
- Include ALL headers from `requiredHeaders` — without them GCS returns 403.
- For inline content instead of a file: `--data-binary 'your content here'` or `echo "content" | curl ... --data-binary @-`
- HTTP 200 = success. The file is now publicly accessible at the GCS URL.

## IMPORTANT: Parameters are REQUIRED

Both `fileSize` and `retentionPeriod` MUST be passed inside `"parameters"` for `/quote` and `/upload`. Without them you get `ERR_NO_SIZE`. Do NOT call these endpoints without parameters.

Correct:
```
x402_call({"service": "nanostore/quote", "method": "POST", "parameters": {"fileSize": 100, "retentionPeriod": 180}})
```

Wrong (will fail with ERR_NO_SIZE):
```
x402_call({"service": "nanostore/quote", "method": "POST"})
```

## Critical Details

- `fileSize` is in BYTES (not KB). It is REQUIRED.
- `retentionPeriod` is in MINUTES: 525600 = 1 year, 1440 = 1 day, 180 = 3 hours (minimum).
- The presigned `uploadURL` expires after ~1 week. Upload promptly after receiving it.
- `requiredHeaders` MUST be included in the PUT request — without them, GCS returns 403.
- The `/list` endpoint returns `uhrpUrl` fields — these are the permanent content-addressed URLs.

## Preferred: upload_to_nanostore Recipe Tool

For uploading files or text, prefer the `upload_to_nanostore` tool over the manual 3-step flow above.
It handles reserve + PUT + public URL construction atomically. Use `search_tools` to find it.

```
upload_to_nanostore({"content": "<html>...</html>", "content_type": "text/html", "retention_minutes": 525600})
→ {"success": true, "public_url": "https://storage.googleapis.com/prod-uhrp/cdn/...", "sats_paid": 10}
```

**IMPORTANT: Set `content_type` when uploading via `content` parameter.** The tool auto-detects MIME type from file extension when using `file_path`, and sniffs HTML/JSON/XML from content prefix, but always set `content_type` explicitly for reliable browser rendering. Common values: `text/html`, `application/json`, `text/plain`, `image/png`.

Use the manual `x402_call` flow only when you need fine-grained control (custom headers, binary files from disk via curl, etc.).

## Public URL After Upload

After a successful upload, the file is publicly accessible at the **GCS CDN URL** — this is the presigned `uploadURL` with query parameters stripped:

```
Presigned URL: https://storage.googleapis.com/prod-uhrp/cdn/RnMVzGFtCMapxbV6F789Xf?X-Goog-Algorithm=...&X-Goog-Signature=...
Public URL:    https://storage.googleapis.com/prod-uhrp/cdn/RnMVzGFtCMapxbV6F789Xf
```

Strip everything after `?` from the `uploadURL` to get the permanent public URL.

To find the content-addressed **UHRP URL** (permanent identifier), call `nanostore/list`:
```
x402_call({"service": "nanostore/list", "method": "GET"})
→ {"uploads": [{"uhrpUrl": "XUUAqk6P...", "expiryTime": 1804014027}]}
```

**Do NOT guess public URL formats.** The only confirmed working format is the GCS CDN URL derived from the presigned upload URL.

## Cost

- ~730 sats per MB per year of retention (minimum 10 sats for any file).
- Use `/quote` first to check the exact price before paying.

## Key Constraints
- retentionPeriod minimum is 180 (minutes). 525600 = 1 year, 1440 = 1 day.
- fileSize is in BYTES, not KB. It is REQUIRED.
- Step 1 (/upload) returns uploadURL. You MUST complete Step 2 (PUT file bytes via execute_bash).
- Include ALL requiredHeaders in the PUT request or GCS returns 403.

## Validation Rules
- retentionPeriod >= 180 | retentionPeriod must be at least 180 minutes (3 hours minimum)
- fileSize > 0 | fileSize is required and must be a positive number (bytes)
- fileSize required | fileSize is required for /quote and /upload
- retentionPeriod required | retentionPeriod is required for /quote and /upload

## Common Errors

- **400 `ERR_NO_SIZE`**: You forgot the `fileSize` parameter. It's required. Always include `"parameters": {"fileSize": N, "retentionPeriod": N}`.
- **400 `ERR_VALIDATION`**: Check that `fileSize` is a number (not a string) and `retentionPeriod` is positive.
- **Upload fails (403 on PUT)**: The presigned URL expired, or you didn't include ALL `requiredHeaders`.
- **File downloads instead of displaying in browser**: Wrong `Content-Type` header on the PUT upload. GCS serves files with whatever Content-Type was set at upload time. Use `text/html` for HTML, `application/json` for JSON, etc. The `upload_to_nanostore` recipe tool auto-detects from file extension or content sniffing, but set `content_type` explicitly when using the `content` parameter for reliability.
