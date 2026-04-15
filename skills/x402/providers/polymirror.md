# Polymirror — Polymarket Whale Tracking

Synchronous service. All endpoints return JSON immediately.

## Working Example

```
# Leaderboard — top traders by profit or volume
x402_call({
  "service": "polymirror/leaderboard",
  "parameters": {
    "category": "OVERALL",
    "time_period": "DAY",
    "order_by": "PNL",
    "limit": 25
  }
})

# Search for a trader by username or wallet
x402_call({
  "service": "polymirror/search",
  "parameters": {
    "query": "GCRClassic"
  }
})

# Deep profile analytics for a specific trader
x402_call({
  "service": "polymirror/trader",
  "parameters": {
    "username": "GCRClassic"
  }
})
```

## Endpoints

| Endpoint | Cost | Description |
|----------|------|-------------|
| `/leaderboard` | ~33K sats ($0.005) | Top traders with metrics |
| `/search` | ~33K sats ($0.005) | Find traders by username/wallet/X handle |
| `/trader` | ~65K sats ($0.01) | Deep profile analytics (win rates, positions) |
| `/portfolio` | ~52K sats ($0.008) | Monitor up to 10 watched wallets |
| `/subscribe` | ~131K sats ($0.02) | Telegram alert management |
| `/history` | ~7K sats ($0.001) | Past alert records |

## Critical Details

- All enum values are UPPERCASE. Lowercase values (e.g., "all", "today") return HTTP 400.
- The `/leaderboard` endpoint defaults are: category=OVERALL, time_period=WEEK, order_by=PNL, limit=25.
- The `/search` endpoint requires a `query` parameter (username, wallet address, or X handle).
- The `/trader` endpoint requires a `username` parameter.

## Key Constraints
- category must be one of: OVERALL, POLITICS, SPORTS, CRYPTO, CULTURE, ECONOMICS, TECH, FINANCE
- time_period must be one of: DAY, WEEK, MONTH, ALL
- order_by must be one of: PNL, VOL
- limit must be between 1 and 50 (integer)
- All enum values are UPPERCASE — lowercase values return 400

## Validation Rules
- category in [OVERALL,POLITICS,SPORTS,CRYPTO,CULTURE,ECONOMICS,TECH,FINANCE] | category must be one of: OVERALL, POLITICS, SPORTS, CRYPTO, CULTURE, ECONOMICS, TECH, FINANCE (UPPERCASE)
- time_period in [DAY,WEEK,MONTH,ALL] | time_period must be one of: DAY, WEEK, MONTH, ALL (UPPERCASE)
- order_by in [PNL,VOL] | order_by must be PNL or VOL (UPPERCASE)
- limit >= 1 | limit must be at least 1
- limit <= 50 | limit must be at most 50

## Cost

- Leaderboard/Search: ~33K sats ($0.005)
- Trader profile: ~65K sats ($0.01)
- Portfolio: ~52K sats ($0.008)
- Subscribe: up to ~131K sats ($0.02)
- History: ~7K sats ($0.001)

## Common Errors

- **400 "Invalid category 'all'"**: Use UPPERCASE values: `"OVERALL"`, not `"all"` or `"All"`.
- **400 "Invalid time_period 'today'"**: Valid values are `DAY`, `WEEK`, `MONTH`, `ALL` — not `"today"` or `"daily"`.
- **400 on /trader without username**: The `username` parameter is required for trader profiles.
- **400 on /search without query**: The `query` parameter is required for search.
