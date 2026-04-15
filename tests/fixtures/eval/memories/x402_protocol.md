---
id: mem-002
category: knowledge
tags:
  - x402
  - payment
  - protocol
  - http
  - authentication
created: "2026-03-01T11:00:00Z"
source: agent-session-43
---

The x402 payment protocol enables HTTP-native micropayments. When a server returns HTTP 402 Payment Required, the client constructs a BSV BEEF transaction as payment and retries the request with the payment attached.

Flow:
1. Client sends request to LLM endpoint
2. Server returns 402 with payment terms (satoshi amount, payment address)
3. Client builds BEEF transaction via wallet API (createAction)
4. Client retries request with X-BSV-Payment header containing the BEEF
5. Server validates payment, processes request, returns response

Critical: The BEEF must include full merkle proof ancestry for SPV verification.
