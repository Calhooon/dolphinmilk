---
id: mem-003
category: knowledge
tags:
  - wallet
  - brc-100
  - integration
  - api
created: "2026-03-02T09:00:00Z"
source: agent-session-44
---

The agent communicates with bsv-wallet-cli running on localhost:3322 via the BRC-100 wallet API. All 28 endpoints are available. The wallet handles UTXO management, key derivation, and transaction signing.

Critical endpoints for the agent:
- createAction: Build and sign transactions (the most complex endpoint)
- getPublicKey: Retrieve the agent's identity public key
- createSignature: Sign arbitrary data with the agent's key
- encrypt/decrypt: Wallet-native encryption using BRC-42 derived keys
- listOutputs: Query UTXOs by basket

The wallet must be funded before the agent can operate. Use `dolphin-milk fund TXID` to internalize UTXOs from an on-chain transaction.
