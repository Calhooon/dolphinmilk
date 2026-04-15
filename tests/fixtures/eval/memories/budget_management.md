---
id: mem-004
category: knowledge
tags:
  - budget
  - cost
  - spending
  - limits
created: "2026-03-02T14:00:00Z"
source: agent-session-45
---

The agent operates under strict budget controls. Every LLM inference call costs satoshis. Budget tracking happens at multiple levels:

- Per-task limit: default 20,000,000 sats (configurable via DOLPHIN_MILK_BUDGET_MAX_PER_TASK)
- Per-hour rate limit: prevents runaway spending
- Session total tracking: recorded in transcript events

When the budget is exhausted, the agent stops gracefully with a budget_check event. The on-chain proof system records spending for auditability.

Cost optimization: Use presigned responses and caching to reduce LLM calls. Memory search is essentially free (local BM25). Tool calls that require wallet operations cost ~200 sats each for on-chain proofs.
