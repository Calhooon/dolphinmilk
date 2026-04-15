# Zero-Knowledge Platform Architecture

**Date:** 2026-03-23
**Status:** Discussion / Design
**Goal:** Make the hosted platform unable to see user data — at rest OR during execution

---

## The Problem

Today, the hosted platform can read user data in three places:

```
1. AT REST:     Plaintext workspace files (transcripts, memories, conversations)
2. IN TRANSIT:  x402 relay sees LLM request/response bodies
3. IN EXECUTION: Platform runs the container, can read process memory
```

Solving #1 is straightforward (encrypt everything). Solving #2 is achievable (direct connections). Solving #3 is the hard one — and it's what every SaaS AI platform has punted on.

**If we solve all three, we have something no one else has: an autonomous AI agent platform where the platform operator is mathematically and architecturally prevented from accessing user data, while still being cheaper and more flexible than TEE-only alternatives.**

---

## Phase 1: Encrypt Everything at Rest

### What Changes

Every file I/O path gets wrapped with wallet-native encryption. The existing `encrypt_with_wallet()` / `decrypt_with_wallet()` infrastructure already works — it just needs to be applied consistently.

### Implementation Plan

#### 1A. Transcripts (HIGH priority)

**Current:** Append-only plaintext JSONL at `workspace/tasks/{id}/session.jsonl`.

**Change:** Encrypt each JSONL line before appending. Decrypt each line on load.

```rust
// Before (transcript.rs)
file.write_all(line.as_bytes())?;

// After
let encrypted = wallet.encrypt(
    line.as_bytes(),
    &[2, "worm transcript"],
    &task_id.to_string(),
    "self"
).await?;
let encoded = BASE64.encode(&encrypted);
file.write_all(format!("{}\n", encoded).as_bytes())?;
```

**Performance:** <1ms per event (local AES, no KSS round-trip per ADR-006). A 50-event task adds <50ms of encryption overhead on a 20-second operation.

**HMAC integrity:** Compute HMAC over the encrypted ciphertext, not the plaintext. Integrity verification doesn't require decryption. This is actually more secure — it's encrypt-then-MAC.

**Event polling (SSE):** Events are decrypted in-memory when loaded. The `events_since(index)` API returns decrypted events to the authenticated client. The file on disk stays encrypted.

#### 1B. Conversations (HIGH priority)

**Current:** Plaintext `messages.jsonl` + `meta.json`.

**Change:** Same per-line encryption for messages. Encrypt `meta.json` as a single blob.

```
Protocol: [2, "worm conversation"]
Key ID:   "conv-{conversation_id}"
Counter:  "self"
```

**Note:** Conversations already encrypt when synced to wallet. This just makes the local copy encrypted too.

#### 1C. Memory Entries (HIGH priority)

**Current:** Plaintext `.md` files are source of truth, `.enc` mirrors are secondary.

**Change:** Flip the model. `.enc` files become the source of truth. No plaintext `.md` files written to disk.

```
MemoryStore::store()   → encrypt → write .enc only
MemoryStore::read()    → read .enc → decrypt → return
MemoryStore::search()  → tantivy index (in-memory, rebuilt from decrypted .enc at boot)
```

**Boot sequence:**
1. Load all `.enc` files
2. Decrypt each with wallet
3. Build tantivy BM25 index in memory
4. Index is ephemeral — never written to disk

**Performance:** 100 memory entries × <1ms decrypt = <100ms boot overhead. Acceptable.

**Constraint:** Wallet must be available at boot. In hosted mode this is always true (MPC proxy starts first). In sovereign mode the wallet is local.

#### 1D. Budget Audit Log (MEDIUM priority)

**Current:** Plaintext JSONL at `workspace/budget.jsonl`.

**Change:** Per-line encryption, same pattern as transcripts.

```
Protocol: [2, "worm budget"]
Key ID:   "audit"
Counter:  "self"
```

#### 1E. Continuation State, Delivery Queue, Schedules (MEDIUM priority)

**Current:** Plaintext JSON files.

**Change:** Encrypt full file as single blob before writing, decrypt on read.

```
Protocol: [2, "worm state"]
Key ID:   "continuation-{id}" / "delivery-{id}" / "schedule-{id}"
Counter:  "self"
```

#### 1F. Config (NO CHANGE)

`dolphin-milk.toml` stays plaintext. It contains no user data — only operational parameters (wallet URL, model preferences, budget limits). It's needed before the wallet is available for decryption.

#### 1G. Tantivy Search Index (NO CHANGE to disk, change behavior)

**Current:** Written to disk at `memory/index/`.

**Change:** Keep index in memory only. Never write to disk. Rebuild from decrypted `.enc` files at boot.

If boot time becomes an issue with thousands of memories, we can write an encrypted serialized index as a single blob.

### Phase 1 Effort Estimate

| Component | Files Changed | Effort | Risk |
|-----------|--------------|--------|------|
| Transcripts | `session/transcript.rs` | 1 day | Low |
| Conversations | `session/conversation.rs` | 1 day | Low |
| Memory entries | `memory/store.rs`, `memory/sync.rs` | 1.5 days | Medium (source of truth flip) |
| Budget log | `onchain/budget.rs` | 0.5 day | Low |
| Continuations/delivery/schedules | 3 files | 0.5 day | Low |
| Tantivy (in-memory only) | `memory/search.rs` | 0.5 day | Low |
| Tests | across all | 1 day | Low |
| **Total** | | **6 days** | |

### Phase 1 Result

After Phase 1, the agent container's disk contains **only ciphertext**. An attacker with filesystem access sees encrypted blobs. They would need the MPC-derived decryption key to read anything.

**What this doesn't solve:** The agent process still holds decrypted data in RAM during execution. Platform has process memory access.

---

## Phase 2: Execution Privacy

### The Core Challenge

The agent must process plaintext to function. It constructs prompts, parses responses, executes tools, makes decisions. You can't encrypt what the CPU is actively computing on — the CPU needs plaintext bits.

There are exactly four approaches to this problem. Here's the honest analysis of each.

### Approach A: Confidential Containers (AMD SEV-SNP)

**How it works:**
- The agent container runs inside an AMD SEV-SNP (Secure Encrypted Virtualization - Secure Nested Paging) VM
- The CPU encrypts all VM memory with a per-VM key that the hypervisor cannot access
- The host OS, hypervisor, and cloud provider cannot read the VM's memory
- The VM produces a hardware attestation report proving what code is running

**What changes in Dolphin Milk:** Almost nothing. It's a standard Docker container running in a confidential VM instead of a regular VM.

**Key delivery:**
1. Container boots and produces SEV-SNP attestation report
2. Attestation report contains a measurement of the binary (hash)
3. Independent KSS verifies the measurement matches the on-chain binary hash (#96)
4. KSS delivers MPC share to the container over TLS (TLS terminates inside the VM)
5. Platform infrastructure never sees the share — it was delivered directly to the attested VM

**The platform cannot:**
- Read process memory (hardware-encrypted by CPU)
- Extract MPC shares (delivered only to attested VMs)
- Decrypt stored data (keys exist only inside the VM)
- Tamper with the binary (attestation would change)

**The platform CAN:**
- Observe metadata: timing, data sizes, network destinations
- Kill the VM (denial of service, not data theft)
- Refuse to create VMs (censorship, not data access)

**Cost:**
| Provider | Confidential VM | Standard VM | Premium |
|----------|----------------|-------------|---------|
| Azure (DCasv5) | ~$0.04/hr ($29/mo) | ~$0.03/hr ($22/mo) | ~30% |
| GCP (C3D with SEV-SNP) | ~$0.05/hr ($36/mo) | ~$0.04/hr ($29/mo) | ~25% |
| AWS Nitro Enclaves | ~$0.05/hr ($36/mo) | ~$0.04/hr ($29/mo) | ~25% |

**Total hosted stack with confidential containers:**
- MPC signing: $5-19/mo (CF Workers, unchanged)
- Agent execution: ~$30-40/mo (confidential VM)
- Persistence: included in CF/cloud pricing
- **Total: ~$35-60/mo** vs $100-500/mo for dedicated TEE infrastructure

**Maturity:** Production-ready. Azure Confidential Containers are GA. GCP Confidential VMs are GA. AWS Nitro Enclaves are GA. Signal, Edgeless Systems, and others use these in production.

**Verdict: This is the most practical path.** Standard Docker container, ~25-30% cost premium, no code changes, hardware-enforced memory encryption.

### Approach B: WASM Agent in CF Container (PREFERRED)

**The key insight:** A Cloudflare Container is always online — 24/7, autonomous, no user device required. The "client-side" framing isn't about the user's browser; it's about the agent container being the **client** of platform services. The container holds all plaintext. Platform services (MPC, billing, persistence) only see encrypted data and signing requests.

**How it works:**

```
┌─────────────────────────────────────────────────┐
│  CF CONTAINER (user's agent — the "client")      │
│                                                  │
│  ┌─────────────────────────────────────────┐     │
│  │  dolphin-milk-core (WASM module)            │     │
│  │  ├─ Prompt construction                 │     │
│  │  ├─ LLM request building                │     │
│  │  ├─ Response parsing + tool extraction  │     │
│  │  ├─ Budget tracking (in-memory)         │     │
│  │  ├─ Context management                  │     │
│  │  ├─ Sanitization + moderation           │     │
│  │  └─ WASM hash verifiable on-chain       │     │
│  └───────────────┬─────────────────────────┘     │
│                  │ encrypted I/O only             │
│  ┌───────────────┴─────────────────────────┐     │
│  │  Native runtime (tools, HTTP, I/O)      │     │
│  │  ├─ Tool execution                      │     │
│  │  ├─ Encrypted file persistence          │     │
│  │  └─ HTTP server for UI                  │     │
│  └─────────────────────────────────────────┘     │
└───────────┬──────────────┬───────────────────────┘
            │              │
    encrypted blobs   signing requests only
            │              │
            ▼              ▼
    R2/D1 (ciphertext)   MPC KSS (CF Workers)
```

**The trust model — three distinct parties:**

| Party | Role | Sees Plaintext? |
|-------|------|----------------|
| **LobsterFarm** (platform operator) | Deploys WASM module, operates billing API | **No** — doesn't operate the container runtime |
| **Cloudflare** (infrastructure) | Operates CF Container runtime | **Theoretically yes** — but same trust as AWS/GCP/Azure for any workload |
| **LLM Provider** | Processes inference | **Yes** (prompts/responses) — unavoidable |

**Why this is a meaningful security improvement:**

LobsterFarm (us, the platform) **cannot read execution memory** because we don't operate the V8/container runtime. Cloudflare does. This is the same trust model as:
- Trusting AWS not to read your EC2 instance memory
- Trusting Apple not to read your iPhone's process memory
- Trusting Cloudflare not to read your Worker's V8 isolate (they already handle HTTPS termination for millions of sites)

The difference vs. running your own server: you trust Cloudflare (a large infrastructure company with SOC 2, ISO 27001, GDPR compliance, and a business model built on trust) instead of trusting LobsterFarm (a startup). That's a strictly better trust posture for the user.

**What LobsterFarm cannot do:**
- Read agent memory during execution (don't operate the runtime)
- Read data at rest (encrypted with MPC-derived keys we don't hold alone)
- Sign transactions unilaterally (MPC 2-of-3 with independent KSS)
- Tamper with agent code (WASM hash on-chain, verifiable)
- Decrypt stored data (keys derived inside the container from MPC share)

**What Cloudflare could theoretically do:**
- Inspect container memory (they operate the runtime)
- Read WASM linear memory during execution
- But: strong legal/business/compliance incentives against this
- And: any inspection would require CF employees with infrastructure access, not API access

**WASM verification:**
The `dolphin-milk-core` WASM module is compiled from open source, with a reproducible build. The SHA-256 hash is published on-chain (#96). Anyone can verify:
1. Download the open source code
2. Compile with the same toolchain
3. Compare hash to on-chain record
4. If they match, the deployed module is the open source code — no backdoors

**WASM feasibility (validated):**

The `bsv-mpc-worker` already proves this pattern works:
- Rust → `wasm32-unknown-unknown` → CF Worker
- 1069KB module (393KB gzipped), well under 10MB CF limit
- 79.5MB RSS, under 128MB CF limit
- 1ms cold start, 16ms HTTPS round-trip
- BRC-31 auth, serde, crypto all working in WASM

The "agent brain" that moves to WASM:

| Module | WASM-Ready? | Notes |
|--------|------------|-------|
| `context/prompt.rs` — system prompt builder | **Yes, as-is** | Pure string construction |
| `context/manager.rs` — context window mgmt | **Yes** | Feature-gate `offload_to_file()` |
| `think.rs` — LLM inference | **Yes** | Abstract HTTP behind trait (`worker::Fetch` for WASM) |
| `runner/text_extract.rs` — tool call parsing | **Yes, as-is** | Pure string parsing |
| `sanitize.rs` — injection defense | **Yes, as-is** | `aho-corasick` + `regex` compile to WASM |
| `moderation.rs` — content moderation | **Yes, as-is** | `regex`-based |
| `onchain/budget.rs` — budget tracking | **Yes** | Feature-gate JSONL persistence |
| `error.rs`, `types.rs`, `config/schema.rs` | **Yes, as-is** | Pure data types |
| `session/events.rs` — event types | **Yes, as-is** | Pure enums |
| `tools/registry.rs` — tool definitions | **Yes, as-is** | Data structures only |

The "agent body" that stays native in the CF Container:

| Module | Why Native | Communication |
|--------|-----------|---------------|
| `server/` — Axum HTTP (49 routes) | TCP listeners, `tokio` | Calls WASM brain for processing |
| `memory/` — tantivy BM25 search | `memmap2`, threading | Receives encrypted queries from brain |
| `tools/` — bash, browser, file I/O | OS primitives | Brain dispatches tool calls to body |
| `heartbeat/` — scheduler | `tokio::select!`, filesystem | Triggers brain for scheduled tasks |
| `wallet.rs` — wallet HTTP client | `reqwest` | Brain requests signing via body |

**Estimated WASM module size:** ~2-4MB (larger than bsv-mpc-worker due to regex, aho-corasick, more serde types; well under 10MB limit).

**Effort:**

| Task | Effort |
|------|--------|
| Extract `dolphin-milk-core` crate (move brain modules) | 2-3 days |
| Abstract HTTP behind trait (`reqwest` native / `worker::Fetch` WASM) | 1-2 days |
| BRC-31 auth in WASM (pattern from bsv-mpc-worker) | 1 day |
| Replace tantivy with simple in-memory BM25 for WASM | 2-3 days |
| Feature-gate file I/O (budget, transcript) | 1 day |
| CF Worker entry point + brain↔body protocol | 1-2 days |
| Integration testing | 2-3 days |
| **Total** | **~2 weeks** |

**Autonomy:** Full. CF Container is online 24/7. No user device required. Agent runs autonomously.

**Cost:** CF Container pricing (same as #94 plan). No premium for confidential computing hardware. MPC signing on CF Workers at $5-19/mo.

**Verdict: This is the preferred approach.** It separates the platform operator from the infrastructure operator, encrypts all persistent data, makes the execution code verifiable on-chain, and runs on the same CF infrastructure we're already building on. No hardware lock-in, no cost premium, full autonomy.

### Approach C: Fully Homomorphic Encryption (FHE)

**How it works:**
- Encrypt the LLM prompt
- The LLM processes the encrypted prompt and produces an encrypted response
- Only the user can decrypt the response
- The LLM provider and platform never see plaintext

**Current state of FHE for LLM inference (as of 2026):**

| Operation | FHE Overhead | Practical? |
|-----------|-------------|-----------|
| Linear layers | 100-1000x | Research stage |
| Attention mechanism | 10,000-100,000x | Research stage |
| Full transformer forward pass | ~1,000,000x | Not feasible |
| Embedding lookup only | 10-100x | Feasible but limited value |

A single LLM call that takes 10 seconds in plaintext would take **~115 days under FHE.** Even with dedicated FHE accelerator hardware (which doesn't exist at scale), we're looking at hours per inference.

**Research trajectory:**
- Zama (TFHE-rs): Leading FHE library. Can run small classifiers, not transformers.
- CryptoLab: FHE attention mechanism proofs-of-concept. Not production.
- Intel HEXL: Hardware acceleration for FHE primitives. Helps with linear layers.
- Projected timeline for practical FHE-LLM: **2032-2035+** (optimistic).

**Verdict: Not feasible for 5-10+ years.** Tracking via academic papers, but not a planning input.

### Approach D: Secure Multi-Party Computation for Inference (MPC-ML)

**How it works:**
- Split the LLM model weights across multiple parties
- Each party computes on their share of the weights
- The user's prompt is secret-shared across parties
- No single party sees the full computation

**Current state:**
- CrypTen (Meta): 2-party computation for small models. 100-1000x overhead.
- MPCFormer: Secret-shared attention. 500x overhead on BERT-base.
- Iron: 3-party computation for neural networks. Still research.

**Even optimistically:** A 10-second LLM call would take ~2-3 hours under MPC-ML, plus massive network bandwidth between parties.

**Verdict: Not feasible.** Same timeline as FHE. Interesting research, not a product input.

### Approach Comparison

| Approach | Privacy From Platform? | Privacy From Infra? | Cost | Autonomy | Code Changes | Timeline |
|----------|----------------------|---------------------|------|----------|-------------|----------|
| **WASM in CF Container** | **Yes** | No (trust CF) | **$0 premium** | **Full** | **~2 weeks** | **Now** |
| Confidential Containers | **Yes** | **Yes** (hardware) | +25-30% | **Full** | None | Now |
| FHE | Yes | Yes | 1,000,000x | Full | Requires FHE LLM | 2032+ |
| MPC-ML | Yes | Partial | 500x | Full | Requires model splitting | 2030+ |

**The WASM-in-CF-Container approach is preferred.** It provides platform-operator privacy at zero cost premium, with full autonomy, on infrastructure we're already using. Confidential containers are an upgrade path for users who don't trust Cloudflare (or any infrastructure provider) — but for most users, trusting CF is the same level of trust they already place in AWS/GCP/Azure for all their other workloads.

**The honest remaining trust:** Cloudflare (infrastructure operator) could theoretically inspect container memory, just like AWS could inspect EC2 memory. This is the baseline trust assumption of all cloud computing. For users who can't accept this, sovereign mode eliminates all external trust.

---

## Phase 3: Eliminate the x402 Relay

### The Problem

Currently, LLM requests go through an x402 relay (`openai-chat.x402agency.com`). This relay:
- Adds BRC-31 authentication
- Handles BSV payment (accepts payment, forwards to LLM provider)
- Sees the full request and response bodies

Even with Phase 1 (encrypted at rest) and Phase 2 (confidential containers), the x402 relay is a plaintext transit point.

### The Solution: Direct LLM Connections

From inside the confidential container, the agent connects directly to LLM providers:

```
BEFORE:
  Agent → x402 relay (sees plaintext) → LLM provider (sees plaintext)

AFTER:
  Agent (in confidential container) → LLM provider (sees plaintext, unavoidable)
  Agent → x402 payment service (sees only payment data, not prompts)
```

**How this works:**
1. Agent constructs the LLM request inside the confidential container
2. Agent sends the request directly to the LLM provider (OpenAI, Anthropic) over TLS
3. TLS terminates inside the container — the platform cannot MITM
4. Payment is handled separately: the agent sends a payment proof to an x402 settlement service that sees only the payment metadata (amount, recipient, tx hash), not the prompt/response

**Requirements:**
- LLM providers must accept API key auth or x402 payment tokens
- Payment settlement must be separable from request proxying
- The agent must have outbound internet access from the confidential container

**Residual trust:** The LLM provider (OpenAI, Anthropic) always sees the prompt. This is inherent to how LLMs work. The prompt must be plaintext for the model to process it. The only mitigation is the LLM provider's privacy policy and contractual commitments.

**But:** The platform (LobsterFarm) no longer sees prompts. Only the LLM provider does. The user's trust boundary shifts from "trust LobsterFarm + LLM provider" to "trust only LLM provider."

### Phase 3 Effort

| Component | Change | Effort |
|-----------|--------|--------|
| Direct LLM HTTP client | New module, TLS from inside container | 2 days |
| Payment separation | Split x402 into auth + payment settlement | 3 days |
| Provider API key management | Encrypted key storage, user-supplied or platform-provisioned | 2 days |
| Tests | E2E with direct connections | 1 day |
| **Total** | | **8 days** |

---

## The Full Zero-Knowledge Stack

After all three phases, with WASM agent in CF Container:

```
┌──────────────────────────────────────────────────────────────────┐
│  THREE TRUST DOMAINS                                              │
│                                                                   │
│  ┌────────────────────────────┐                                   │
│  │  LOBSTERFARM (platform)    │  Cannot read agent data.          │
│  │  ├─ Billing API            │  Only sees: credit amounts,       │
│  │  ├─ Template registry      │  API call counts, metadata.       │
│  │  ├─ Dashboard/UI routing   │                                   │
│  │  └─ Deploys WASM module    │  WASM hash on-chain = verifiable  │
│  └────────────┬───────────────┘                                   │
│               │ encrypted only                                    │
│               ▼                                                   │
│  ┌────────────────────────────────────────────────┐               │
│  │  CLOUDFLARE (infrastructure)                    │               │
│  │                                                 │               │
│  │  ┌──────────────────────────────────────┐       │               │
│  │  │  CF CONTAINER (agent runtime)         │       │               │
│  │  │                                      │       │               │
│  │  │  ┌──────────────────────────────┐    │       │               │
│  │  │  │  dolphin-milk-core (WASM brain)  │    │       │               │
│  │  │  │  ├─ Decrypts user messages   │    │       │               │
│  │  │  │  ├─ Constructs LLM prompts   │    │       │               │
│  │  │  │  ├─ Parses responses         │────┼───────┼──→ LLM Provider
│  │  │  │  ├─ Tracks budget            │    │       │    (sees prompts,
│  │  │  │  ├─ MPC signs (16ms)         │────┼───────┼──→ KSS (CF Worker)
│  │  │  │  └─ Encrypts all output      │    │       │    (sees signing
│  │  │  └──────────────────────────────┘    │       │     requests only)
│  │  │       ↓ ciphertext only              │       │               │
│  │  │  Native body (tools, HTTP, I/O)      │       │               │
│  │  └──────────────┬───────────────────────┘       │               │
│  │                 │ ciphertext                     │               │
│  │  ┌──────────────┴───────────────────────┐       │               │
│  │  │  R2 (encrypted blobs)                 │       │               │
│  │  │  D1 (encrypted structured state)      │       │               │
│  │  └──────────────────────────────────────┘       │               │
│  │                                                 │               │
│  │  CF sees: container runtime, network traffic    │               │
│  │  CF trust: SOC 2, ISO 27001, GDPR, business    │               │
│  │  model built on not snooping                    │               │
│  └─────────────────────────────────────────────────┘               │
└──────────────────────────────────────────────────────────────────┘
```

### What Each Party Sees

| Party | Sees Plaintext? | What They Know |
|-------|----------------|----------------|
| **User** | Yes | Everything (their data) |
| **LLM Provider** | Yes (prompts/responses only) | Task content during inference |
| **LobsterFarm** (platform) | **No** | Metadata: credit amounts, API call counts, template selections |
| **Cloudflare** (infrastructure) | **Theoretically** (operates runtime) | Same trust as AWS/GCP — business model prevents snooping |
| **MPC KSS nodes** | **No** | Signing requests only (amount + tx hash) |
| **R2/D1** (persistence) | **No** | Encrypted blobs and encrypted rows |
| **Network observers** | **No** | TLS-encrypted traffic |

### What LobsterFarm (Platform) Cannot Do

| Action | Prevented By |
|--------|-------------|
| Read user messages | Doesn't operate the container runtime (CF does) |
| Read agent memories | Encrypted at rest with MPC-derived keys |
| Read LLM prompts/responses | Direct connection from container to LLM provider |
| Sign transactions unilaterally | MPC 2-of-3+ with independent KSS |
| Tamper with agent code | WASM hash on-chain, reproducible builds |
| Decrypt stored data | Keys derived inside container from MPC share |
| Access historical data | All persistence encrypted, platform has no decryption keys |

### What Cloudflare (Infrastructure) Could Theoretically Do

| Action | Likelihood | Mitigation |
|--------|-----------|-----------|
| Inspect container memory | Extremely low (SOC 2, ISO 27001, business model) | Same trust as all cloud providers |
| Read R2/D1 contents | Would see ciphertext only (encrypted before storage) | Wallet-native encryption |
| Intercept network traffic | TLS prevents MITM | Certificate pinning possible |

### What Neither Party Can Do

| Action | Why |
|--------|-----|
| Reconstruct the full private key | MPC — joint key never exists in memory |
| Decrypt data without the container running | Keys are ephemeral, derived from MPC share at boot |
| Modify agent behavior undetectably | WASM hash on-chain, open source, reproducible builds |

### Sovereign Mode: Zero External Trust

For users who trust no one:
- Run `bsv-wallet-cli` locally (full private key on their machine)
- Run Dolphin Milk locally (all data on their machine)
- Connect directly to LLM providers (no relay)
- Zero platform involvement, zero Cloudflare involvement
- Same agent code, different config: `deployment_mode = "sovereign"`

---

## Comparison: Our Architecture vs Alternatives

### vs TEE-Only (e.g., running everything in SGX/Nitro)

| | Our Architecture (MPC + WASM + CF) | TEE-Only |
|---|---|---|
| Fund security | MPC on CF Workers ($5-19/mo) | Key sealed in enclave ($50-500/mo) |
| Execution privacy | WASM in CF Container (trust CF, not platform) | Hardware enclave (trust no one) |
| Data at rest | Wallet-native AES-256-GCM | Sealed storage |
| Hardware lock-in | **None** | Intel SGX / AMD SEV / AWS Nitro |
| Signing latency | 16ms (MPC optimized) | Varies by TEE impl |
| Cost | **$5-19/mo** (MPC) + CF Container pricing | **$100-500/mo** |
| Key management | Threshold resharing, multi-party, key refresh | Single-enclave key |
| Sovereign fallback | Full local mode, zero external trust | Need TEE hardware locally |
| Scalability | CF Workers (auto-scale, global edge) | TEE instances (manual scaling) |
| Verifiability | WASM hash on-chain + open source | Hardware attestation |
| Upgrade path | Add confidential containers later if needed | Already at ceiling |

**We're 5-25x cheaper.** The tradeoff: we trust Cloudflare (infrastructure) while TEE trusts hardware. For most users, trusting CF is the same trust they already place in AWS/GCP for everything else. For the rare user who trusts no infrastructure provider, we offer sovereign mode — which TEE-only can't match.

### vs Current SaaS AI (ChatGPT, Claude, Copilot, etc.)

| | Our Architecture | Typical SaaS AI |
|---|---|---|
| **Platform sees your data** | **No** (WASM in CF Container, encrypted persistence) | **Yes** (platform runs your code, holds your data) |
| Fund management | MPC non-custody | N/A |
| Data at rest | User-encrypted (platform can't read) | Platform-encrypted (platform holds keys) |
| Data during execution | Platform separated from runtime | Platform IS the runtime |
| Audit trail | On-chain, publicly verifiable | Internal logs, trust the provider |
| Code verifiability | Open source + WASM hash on-chain | Proprietary black box |
| Self-hosting | Full sovereign mode | Usually not available |
| Data portability | Encrypted backups, wallet-synced | Platform lock-in |
| LLM provider sees prompts | Yes (same, unavoidable) | Yes (same) |

**We provide security properties that SaaS AI platforms architecturally cannot.** They run your data on their infrastructure with their keys. We run your data on neutral infrastructure (CF) with your keys (MPC-derived). The platform operator (us) never touches plaintext.

---

## Implementation Phases and Timeline

| Phase | What | Effort | Prerequisite | Security Gain |
|-------|------|--------|-------------|---------------|
| **1: Encrypt at rest** | Encrypt all workspace files | 6 days | None — can start now | Platform AND Cloudflare can't read stored data |
| **2: WASM brain extraction** | Extract `dolphin-milk-core` crate, compile to WASM | ~2 weeks | None — can parallel with M1 | Verifiable execution (hash on-chain) |
| **3: Direct LLM** | Agent connects directly to LLM providers | 8 days | Phase 2 (WASM brain does HTTP) | Platform can't see prompts |

**Phase 1 can happen in any round.** It's independent of the milestone plan. Doing it early means the persistence layer (#86, #88, #90) is designed for ciphertext from day one.

**Phase 2 can parallel with M1 (Hosted: Billing).** The WASM extraction creates the `dolphin-milk-core` crate that becomes the verifiable execution unit.

**Phase 3 follows Phase 2.** The WASM brain handles direct LLM connections. The x402 relay becomes payment-only.

**Confidential containers are an optional Phase 4 upgrade** for users who need hardware-enforced privacy from Cloudflare itself. Not needed for most users. Can be added later without architectural changes — just deploy the same container to a confidential VM instead of a standard one.

### The Honest Residual

After all three phases, exactly one party still sees user plaintext: **the LLM provider.** This is mathematically unavoidable without FHE (which is 5-10+ years away from practical LLM inference).

The honest statement to users:

> "Your data is encrypted at rest with keys only you control. Your agent runs as verified open-source code (WASM hash on-chain) on Cloudflare infrastructure — we never see your data. Your prompts go directly to the LLM provider. The only party that sees your content is the AI model you chose. If you want zero external trust, run in sovereign mode on your own hardware."

---

## Open Questions for Discussion

1. **Phase 1 priority:** Should we encrypt everything at rest before or during the hosted infrastructure work (M2)? Doing it first means the persistence layer (#86, #88, #90) is designed for ciphertext from day one.

2. **Confidential container provider:** Azure (Confidential Containers), GCP (Confidential VMs), or AWS (Nitro Enclaves)? Azure has the most mature container story. AWS has the most mature enclave story. GCP is in between.

3. **Attestation flow:** How does the user verify the attestation? Options: (a) Platform publishes attestation report, user verifies against on-chain hash. (b) KSS verifies attestation before delivering share. (c) Both.

4. **Direct LLM billing:** If the agent connects directly to LLM providers, how does payment work? Options: (a) Pre-funded API keys stored encrypted in the container. (b) x402 payment tokens generated inside the container, sent alongside the request. (c) The x402 relay remains but sees only payment data, not request bodies (request goes direct, payment goes through relay).

5. **Sovereign-hosted continuum:** Instead of a binary sovereign/hosted toggle, could we offer a spectrum? e.g., "hosted with confidential containers" → "hosted with client-side key custody" → "self-hosted with cloud MPC" → "fully sovereign."

6. **FHE monitoring:** Should we track FHE-LLM research as a long-term option? If FHE becomes practical, it would eliminate the LLM provider as a trust point. This is a 2032+ timeline but would be the ultimate endgame.
