---
id: mem-001
category: knowledge
tags:
  - bsv
  - fees
  - transactions
  - micropayments
created: "2026-03-01T10:00:00Z"
source: agent-session-42
---

BSV transaction fees are extremely low compared to other blockchains. A typical transaction costs less than 1 satoshi per byte, making it ideal for micropayments and high-volume applications. The fee rate is set by miners and has remained stable at approximately 0.05 sat/byte for standard transactions.

Key fee facts:
- Standard P2PKH transaction: ~226 bytes = ~12 sats
- OP_RETURN data: 0.05 sat/byte for the data portion
- No minimum relay fee in practice
- Miners compete on volume, not fee extraction
