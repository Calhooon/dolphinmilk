# x402 Service Example

A minimal axum server demonstrating how to protect API endpoints with x402 BSV micropayments using the `bsv-x402-server` crate.

## Endpoints

| Method | Path | Cost | Description |
|--------|------|------|-------------|
| GET | `/api/joke` | 100 sats | Returns a random programming joke |
| POST | `/api/echo` | 50 sats | Echoes back a message with metadata |
| GET | `/api/time` | 25 sats | Returns the current server time |
| GET | `/health` | free | Health check |
| GET | `/.well-known/x402-info` | free | Service manifest (endpoint discovery) |

## Running

```bash
# From the workspace root
cargo run -p x402-service-example

# With custom port
X402_PORT=8080 cargo run -p x402-service-example
```

## Testing

```bash
# Health check (free)
curl http://localhost:3402/health

# Service manifest (free)
curl http://localhost:3402/.well-known/x402-info | jq

# Without payment — returns 402 with payment terms
curl -v http://localhost:3402/api/joke
# Response headers include:
#   x-bsv-payment-version: 1.0
#   x-bsv-payment-satoshis-required: 100
#   x-bsv-payment-derivation-prefix: <hex>
#   x-bsv-auth-identity-key: <server pubkey>

# With a payment header (normally constructed by a BRC-100 wallet)
curl -H 'x-bsv-payment: {"derivationPrefix":"abc123","derivationSuffix":"def456","transaction":"AQEBAQ=="}' \
     http://localhost:3402/api/joke

# Echo endpoint
curl -X POST \
     -H 'Content-Type: application/json' \
     -H 'x-bsv-payment: {"derivationPrefix":"abc123","derivationSuffix":"def456","transaction":"AQEBAQ=="}' \
     -d '{"message":"Hello from x402!"}' \
     http://localhost:3402/api/echo
```

## How It Works

1. Client sends a request to a paid endpoint
2. Server checks for the `x-bsv-payment` header
3. If missing: returns HTTP 402 with payment terms in response headers
4. If present: validates the BRC-29 payment JSON structure and serves the response

In production, step 4 would also:
- Decode the base64 AtomicBEEF transaction
- Verify the payment output pays the correct amount to the correct derivation path
- Broadcast the payment transaction via the wallet

## Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `X402_PORT` | `3402` | Port to listen on |
| `X402_IDENTITY_KEY` | secp256k1 generator | Server identity public key (hex) |

## Integration with bsv-worm

A bsv-worm agent can call this service using the `x402_call` tool. The agent's wallet handles the BRC-29 payment construction automatically when it receives a 402 response.
