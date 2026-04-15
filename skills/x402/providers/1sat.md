# 1Sat — Bitcoin BSV Inscriptions

Synchronous service. Inscribe data on-chain, get a txid back immediately.

## Working Example

```
# Inscribe text data
x402_call({
  "service": "1sat/inscribe",
  "parameters": {
    "data": "Hello from Dolphin Milk!",
    "contentType": "text/plain"
  }
})
# Response: { "txid": "abc123...", "inscription_id": "...", ... }
```

## Critical Details

- `data` is the content to inscribe. For text, pass a string. For binary, base64-encode.
- `contentType` is the MIME type (e.g. `"text/plain"`, `"image/png"`, `"application/json"`).
- The inscription is permanent and on-chain. There is no undo.
- Response includes `txid` — this is the on-chain proof of the inscription.

## Cost

- Base cost: 200+ sats (varies with data size and network fees).
- Larger data = higher cost. Keep inscriptions concise.

## Key Constraints
- data and contentType are both required.
- The inscription is permanent and on-chain. There is no undo.
- For binary data, base64-encode it. For text, pass a plain string.

## Validation Rules
- data required | data is required for inscription
- contentType required | contentType (MIME type) is required for inscription

## Common Errors

- **Missing contentType**: Always include `contentType` with the correct MIME type.
- **Data too large**: Keep inscriptions reasonable in size. Very large data is expensive.
