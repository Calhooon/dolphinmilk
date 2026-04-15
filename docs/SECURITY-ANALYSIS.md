# Security Analysis: Hosted Platform Trust Model

**Date:** 2026-03-23
**Status:** Living document
**Scope:** What the platform can and cannot do at each phase

---

## Summary

The hosted Dolphin Milk platform has three distinct security properties, each at a different maturity level:

| Property | Alpha (now) | Beta (M1) | GA (M7) |
|----------|-------------|-----------|---------|
| **Fund security** | Platform trust | Independent key custody | Mathematical non-custody |
| **Data privacy at rest** | Partially encrypted | Fully encrypted (with work) | Fully encrypted |
| **Data privacy during execution** | Platform can see | Platform can see | Requires additional solution |

**The gap:** Fund security is solved progressively via MPC. Data-at-rest is solvable now with existing encryption infrastructure. Data privacy *during execution* is the unsolved problem — and it's the same unsolved problem for every SaaS AI platform (ChatGPT, Claude, Copilot, etc.).

---

## 1. Fund Security

### How MPC Protects Funds

The agent's BSV private key is split into threshold shares via CGGMP'24 (state-of-the-art MPC protocol). The full key **never exists in memory or on disk** — it is computed implicitly during threshold signing.

| Threshold | Shares | Who Signs |
|-----------|--------|-----------|
| 2-of-2 (Alpha) | Platform KSS + Platform proxy | Platform alone (both shares) |
| 2-of-3 (Beta) | External KSS + Platform proxy + Recovery service | Any 2 of 3 independent parties |
| 3-of-5 (GA) | Multiple independent operators via overlay | Any 3 of 5, attacker must breach 3 systems |

### What the Platform CAN Do with Funds

| Action | Alpha | Beta | GA |
|--------|-------|------|-----|
| Sign agent transactions unilaterally | **Yes** | **No** | **No** |
| Block agent transactions | **Yes** | **Yes** (can refuse relay) | **No** (multiple KSS via overlay) |
| Sweep agent funds | **Yes** | Only with KSS cooperation | Only with KSS cooperation |
| Read balance/address | **Yes** | **Yes** | **Yes** |
| Modify fee amounts | **Yes** | Detectable (open source) | Detectable (binary hash on-chain) |

### Containment: Even at Alpha

Even when the platform has signing capability, exposure is limited:

- **BRC-52 budget delegation:** The agent only has access to sats explicitly delegated to it, not the user's full wallet.
- **BRC-48 budget allocation tokens:** On-chain tokens cap total spend per task.
- **Budget caps:** Per-task, per-hour, per-day, per-week, per-month hard limits (configurable).
- **On-chain audit trail:** Every payment is a BRC-18 proof on the BSV blockchain. Theft is publicly detectable and provable.
- **Rate limiting:** Cert-driven rate throttling limits transactions per time window.

### Signing Performance

MPC signing adds minimal overhead to the payment flow:

| Operation | Latency | When |
|-----------|---------|------|
| Presigned transaction (production path) | **16ms** over HTTPS | Every payment |
| Presignature generation (background) | ~1.2s | During 5-30s LLM dead time |
| Full 4-round signing (fallback) | ~33ms over HTTPS | Only if presig pool empty |
| DKG (one-time per wallet) | ~52ms over HTTPS | Wallet creation only |

**Total overhead: ~2% of LLM cost, ~16ms on a 5-30 second operation.**

### Comparison: MPC vs Custodial Alternatives

| | Dolphin Milk MPC | Coinbase (custodial) | Fireblocks (MPC) | Self-custody |
|---|---|---|---|---|
| Who holds keys | Split across parties | Exchange holds all | MPC across parties | User holds all |
| Cost | $5-19/mo (CF Workers) | Exchange fees | $500+/mo | $0 |
| Sign without user | Alpha: yes. Beta: no | Yes (they hold keys) | Configurable | No |
| On-chain audit | BRC-18 every action | Statement only | API logs | User verifies |
| Hardware lock-in | None | N/A | None | N/A |

---

## 2. Data Privacy at Rest

### Current State: What's Encrypted vs Plaintext

**Encrypted at rest (wallet-native AES-256-GCM via BRC-42):**

| Data | Protocol ID | Key ID | Notes |
|------|-------------|--------|-------|
| Memory `.enc` mirrors | `[2, "worm memory"]` | category name | Secondary copy, not source of truth |
| Memory sync index | `[2, "worm memory"]` | `"index"` | Encrypted manifest |
| Wallet-synced conversations | `[2, "worm conversation"]` | `"conv-{id}"` | Full bundle encrypted |
| Cross-agent messages (BRC-78) | `[2, "worm message encryption"]` | `"message"` | End-to-end encrypted |
| MPC key shares | AES-256-GCM | per-session | HMAC-SHA256(root_key) derivation |
| UHRP cloud backups | wallet-native | varies | CDN serves ciphertext only |

**Plaintext on disk (in the agent container):**

| Data | Location | Contains | Sensitivity |
|------|----------|----------|-------------|
| Memory `.md` files | `memory/` | Knowledge, context, sessions | **HIGH** — user's accumulated knowledge |
| Transcripts | `workspace/tasks/{id}/session.jsonl` | Full LLM prompts, responses, tool calls | **HIGH** — complete execution history |
| Conversations | `workspace/conversations/{id}/messages.jsonl` | Full dialogue history | **HIGH** — user's messages |
| Conversation metadata | `workspace/conversations/{id}/meta.json` | Titles, participant keys, sats totals | **MEDIUM** |
| Budget audit log | `workspace/budget.jsonl` | Per-service spending records | **MEDIUM** |
| Continuation state | `workspace/continuations/{id}.json` | Task descriptions, transcript paths | **MEDIUM** |
| Delivery queue | `workspace/delivery_queue/{id}.json` | Failed outbound messages | **HIGH** — message content |
| Schedules | `workspace/schedules/{id}.json` | Recurring task definitions | **LOW** |
| Config | `dolphin-milk.toml` | Wallet URL, model prefs, budget limits | **LOW** — no secrets |
| Tantivy search index | `memory/index/` | Full-text index of all memories | **HIGH** — searchable knowledge |
| Log output | stdout/files | Redacted task flow info | **LOW** — redacted by RedactingWriter |

### The Gap

The architectural intent (from JIT-MPC-MERGED-ARCHITECTURE.md) states: "Read agent data: No (encrypted with MPC-derived keys)." But the implementation has plaintext workspace files as the source of truth, with encrypted mirrors as secondary copies.

**Closing this gap is straightforward.** The `encrypt_with_wallet()` / `decrypt_with_wallet()` infrastructure already exists and works. It needs to be applied to every file I/O path. See [ZERO-KNOWLEDGE-ARCHITECTURE.md](ZERO-KNOWLEDGE-ARCHITECTURE.md) for the detailed plan.

---

## 3. Data Privacy During Execution

### The Fundamental Problem

During execution, the agent process must hold plaintext data in memory to:

1. **Construct LLM prompts** — system prompt + history + tools + recalled memories
2. **Parse LLM responses** — extract tool calls, text, reasoning
3. **Execute tool calls** — pass parsed arguments to tool functions
4. **Make decisions** — budget checks, done-signal detection, obligation tracking
5. **Record events** — write transcript entries (even if encrypted at rest, decrypted in memory)

The agent is a **trusted execution context**. It must see plaintext. The question is whether the *platform infrastructure around it* can be prevented from reading the agent's memory.

### What the Platform Can See During Execution

| Data in Process Memory | Accessible to Platform? | Notes |
|----------------------|------------------------|-------|
| Full LLM request bodies | **Yes** (platform runs the process) | System prompt + all context |
| Full LLM response bodies | **Yes** | Complete model output |
| Tool inputs and outputs | **Yes** | Every tool call |
| Decrypted memories | **Yes** | During recall |
| MPC share_B | **Yes** | Held by proxy process |
| BRC-31 session state | **Yes** | Auth nonces |
| Budget state | **Yes** | Spending records |

### Why This Matters

Without execution privacy, the platform can:

- Read every user message and agent response
- See what tools are called and with what arguments
- Read recalled memories and knowledge
- Profile user behavior and interests
- Potentially exfiltrate data (though detectable via attestation)

This is the **same trust model as ChatGPT, Claude, GitHub Copilot, and every other SaaS AI platform**. Users trust the platform not to abuse its access.

### Why We're Still Better Than Most

Even without solving execution privacy, Dolphin Milk's security posture is stronger than typical SaaS AI:

| Property | Dolphin Milk | Typical SaaS AI |
|----------|----------|-----------------|
| Fund security | MPC non-custody (Beta+) | N/A (no fund management) |
| Data at rest | Encrypted with user-derived keys | Platform-encrypted (platform holds keys) |
| Audit trail | On-chain, publicly verifiable | Internal logs only |
| Code verifiability | Open source, reproducible builds | Proprietary |
| Self-hosting option | Sovereign mode, zero platform trust | Usually not available |
| Data portability | Encrypted backups, wallet-synced | Platform lock-in |

---

## 4. Sovereign Mode: Zero Trust

If a user runs Dolphin Milk in sovereign mode (`bsv-wallet-cli` locally), **all platform trust assumptions disappear**:

- Full private key stays on user's machine (no MPC needed)
- All data stays on user's machine (no cloud persistence)
- LLM calls go directly from user's machine to x402 providers
- No platform involvement whatsoever

Mode switching (#20) is a config toggle: `deployment_mode = "sovereign"` vs `"hosted"`.

Sovereign mode is the ultimate fallback. If a user doesn't trust the platform, they can run everything locally with zero compromise in functionality.

---

## 5. MPC vs TEE: Comparative Analysis

### Cost Comparison

| Deployment | MPC (our approach) | TEE (hardware enclaves) |
|-----------|-------------------|------------------------|
| Cloudflare Workers | **$5-19/mo** | Not available (no TEE) |
| Standard VPS/Cloud VM | **$10-25/mo** | N/A |
| AWS Nitro Enclaves | N/A | **$50-200/mo** (Nitro instance required) |
| Azure Confidential VMs (SEV-SNP) | N/A | **$40-150/mo** (10-20% premium) |
| Bare-metal SGX | N/A | **$100-500/mo** |

**MPC is 5-50x cheaper because it runs on commodity hardware.**

### Security Properties Comparison

| Property | MPC Only | TEE Only | MPC + TEE |
|----------|----------|----------|-----------|
| Fund non-custody | **Yes** (Beta+) | Yes (key in enclave) | **Yes** |
| Data privacy at rest | **Yes** (encrypt everything) | Yes (sealed storage) | **Yes** |
| Data privacy during execution | **No** | **Yes** | **Yes** |
| Hardware lock-in | **None** | Intel/AMD/ARM specific | Partial (TEE for execution only) |
| Side-channel resistance | **N/A** (no hardware boundary) | Vulnerable (Spectre, etc.) | MPC for signing, TEE for execution |
| Verifiability | Open source + on-chain hash | Attestation | Both |
| Sovereign fallback | **Yes** (full local mode) | No (need TEE hardware) | **Yes** |

### Where MPC Wins

1. **Cost:** $5-19/mo vs $50-500/mo
2. **Hardware flexibility:** Any cloud, any VPS, Cloudflare Workers, bare metal
3. **Fund security:** Mathematical non-custody without hardware trust
4. **Key management:** Threshold resharing, key refresh, no fund migration needed
5. **Audit trail:** On-chain proofs independent of execution environment

### Where TEE Wins

1. **Execution privacy:** Hardware-enforced memory encryption
2. **Attestation:** Hardware proof of what code is running
3. **All-in-one:** Fund security + data privacy in one boundary

### The Optimal Architecture: MPC + Confidential Containers

Use MPC for what it's best at (fund security on cheap hardware) and the cheapest form of TEE for what MPC can't do (execution privacy):

```
┌────────────────────────────────────────────────────┐
│  FUND SECURITY (MPC)                                │
│  ├─ CF Workers ($5/mo per KSS node)                 │
│  ├─ No hardware requirements                        │
│  ├─ 16ms signing latency                            │
│  └─ Mathematical non-custody at 2-of-3              │
│                                                     │
│  EXECUTION PRIVACY (Confidential Container)         │
│  ├─ Azure Confidential Containers (~$15-30/mo)      │
│  ├─ AMD SEV-SNP memory encryption                   │
│  ├─ Standard Docker container, no code changes       │
│  └─ Attestation proves binary matches on-chain hash │
│                                                     │
│  DATA PRIVACY AT REST (Wallet-native encryption)    │
│  ├─ BRC-42 + AES-256-GCM                            │
│  ├─ Keys derived from MPC share (no KSS needed)     │
│  ├─ Platform stores only ciphertext                  │
│  └─ User can decrypt with sovereign wallet           │
│                                                     │
│  Total: ~$25-50/mo for full security stack           │
│  vs $100-500/mo for TEE-only approach                │
└────────────────────────────────────────────────────┘
```

---

## 6. Threat Model Summary

### Threats Mitigated

| Threat | Mitigation | Phase |
|--------|-----------|-------|
| Platform steals funds | MPC threshold signing (2-of-3+) | Beta |
| Platform reads data at rest | Wallet-native encryption on all files | Alpha (with work) |
| Platform reads data in transit | TLS 1.3 + BRC-31 mutual auth | Already done |
| Platform modifies agent code | Reproducible builds + binary hash on-chain | M2 (#96) |
| Prompt injection | 4-layer defense (sanitize.rs) | Already done |
| Log leakage | RedactingWriter with 6 pattern matchers | Already done |
| Cross-agent message interception | BRC-78 end-to-end encryption | Already done |
| Budget theft via hidden fees | On-chain proofs + MPC fee transparency | Already done |
| Key extraction | Joint key never exists — computed implicitly | Already done |
| Nonce reuse (key leak) | Atomic FIFO presignature consumption | Already done |
| Self-message loop attack | 3-layer defense + Reply Obligation Protocol | Already done |

### Threats Remaining

| Threat | Impact | Current Status | Path to Mitigation |
|--------|--------|---------------|-------------------|
| Platform reads execution memory | Can see all prompts/responses | **Open** | Confidential containers or client-side execution |
| Platform reads plaintext workspace files | Can see transcripts, memories | **Open** | Encrypt all workspace I/O (see ZK Architecture) |
| LLM provider sees prompts | Inherent to LLM inference | **Open** | Unsolvable without FHE (5-10yr+) |
| x402 relay sees request/response | Transit point for LLM calls | **Open** | Direct LLM connections from agent |
| Traffic analysis (timing, sizes) | Metadata leaks behavior patterns | **Open** | Padding + batching (low priority) |
| Browser DKG window (120s) | Key exposure during wallet creation | **Open** (Beta) | Minimize window, memory clearing |
| BRC-31 auth on KSS not fully implemented | Some KSS endpoints rely on URL obscurity | **Open** | Complete BRC-31 auth on all KSS endpoints |

---

## 7. Progressive Security Roadmap

```
ALREADY DONE:
  ✓ BRC-31 mutual authentication
  ✓ BRC-78 end-to-end message encryption
  ✓ BRC-18 on-chain audit trail
  ✓ BRC-52 budget delegation and containment
  ✓ 4-layer prompt injection defense
  ✓ Log redaction (6 pattern matchers)
  ✓ Memory encryption (.enc mirrors)
  ✓ MPC threshold signing (2-of-2)
  ✓ Cert-driven rate limiting
  ✓ Content moderation engine

MILESTONE 1 (Hosted: Billing):
  → Independent KSS operator (2-of-3, no single-party signing)
  → MPC smoke test validates non-custody
  → Browser DKG for user wallet creation

MILESTONE 2 (Hosted: Infrastructure):
  → Encrypt all workspace I/O at rest (#149) — platform stores only ciphertext
  → Make .enc memory files the source of truth, tantivy in-memory only
  → Reproducible builds + binary hash on-chain (#96)
  → WASM brain extraction (#150, #151) — verifiable execution code
    - Three trust domains: deployer (LobsterFarm) ≠ operator (CF) ≠ LLM provider
    - WASM hash on-chain, reproducible builds, open source
    - Platform deploys the module but doesn't run it
    - CF runs it but doesn't deploy it
    - Neither can tamper with it
  → See ZERO-KNOWLEDGE-ARCHITECTURE.md

CONFIDENTIAL CONTAINERS (optional upgrade, post-M2):
  → Run agent in AMD SEV-SNP VM / Azure Confidential Container
  → Hardware-attested binary matches on-chain hash
  → Platform infrastructure cannot read agent memory
  → For users who need hardware-enforced privacy from Cloudflare itself
  → Not needed for most users (WASM-in-CF-Container is the preferred path)

MILESTONE 4 (Enterprise):
  → Direct LLM connections (#152) — eliminate x402 relay as plaintext transit
  → HSM integration, ZK proofs

MILESTONE 7 (Decentralization):
  → 3-of-5 across independent operators
  → Mathematical non-custody
  → Full zero-knowledge platform
```
