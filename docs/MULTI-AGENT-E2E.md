# Multi-Agent E2E Progress

> Milestone 34 — Verify all building blocks for multi-agent communication with real sats.
> DolphinMilkShake depends on every gate passing.

**Started:** 2026-04-09
**Milestone:** [M34 Multi-Agent E2E](https://github.com/Calgooon/rust-bsv-worm/milestone/34)

---

## Gate Status

| Gate | Issue | Title | Status | Tests | Commit |
|------|-------|-------|--------|-------|--------|
| 1 | [#310](https://github.com/Calgooon/rust-bsv-worm/issues/310) | MessageBox probe | **CLOSED** | 4 live | `b8cbb91` |
| 2 | [#311](https://github.com/Calgooon/rust-bsv-worm/issues/311) | Cross-wallet messaging | **CLOSED** | 6 live | `dd3e22f` |
| 3 | [#312](https://github.com/Calgooon/rust-bsv-worm/issues/312) | overlay_lookup in runner | **CLOSED** | 12 unit | `dab050d` |
| 4 | [#313](https://github.com/Calgooon/rust-bsv-worm/issues/313) | Agent ingestion from MessageBox | **CLOSED** | 1 E2E (real sats) | `6937a68` |
| 5 | [#314](https://github.com/Calgooon/rust-bsv-worm/issues/314) | Two-agent handshake | **CLOSED** | 1 E2E (real sats) | `961bb5f` |
| 6 | [#315](https://github.com/Calgooon/rust-bsv-worm/issues/315) | Payment-in-message (BEEF) | **CLOSED** | 3 live | `720300e` |
| 7 | [#316](https://github.com/Calgooon/rust-bsv-worm/issues/316) | Captain→Coral pipeline | OPEN | — | — |
| 8 | [#317](https://github.com/Calgooon/rust-bsv-worm/issues/317) | DolphinMilkShake configs | **CLOSED** | — | (DMS repo) |

**7/8 closed. 1 remaining (#316 Captain→Coral pipeline).**

### Real sats spent on E2E testing
- Gate 4 (ingestion): ~98K sats, 15 on-chain txs
- Gate 5 (handshake): ~168K sats (final passing run), 31 on-chain proofs across 2 agents
- Gate 5 (all attempts): ~800K+ sats across 4 test runs (debugging auth + query issues)
- Gate 6 (payment): ~1K sats, balance transfer verified
- Wallet B funding: 4M sats transferred from wallet A (4 x 1M)
- **Total E2E testing cost: ~1.1M sats (~$0.18)**

### Bugs discovered during testing

| Issue | Title | Status | Impact |
|-------|-------|--------|--------|
| [#318](https://github.com/Calgooon/rust-bsv-worm/issues/318) | MessageBox BRC-31 session churn | OPEN | Multi-agent messaging unreliable (~70% success) |
| [#319](https://github.com/Calgooon/rust-bsv-worm/issues/319) | UI 403 on test ports (8081/8082) | OPEN | Can't watch agents during tests |
| [#320](https://github.com/Calgooon/rust-bsv-worm/issues/320) | overlay_lookup BEEF parsing returns 0 for recent registrations | OPEN | Agents can't discover each other by identity key |

### Fixes shipped during testing
- Query normalization: empty/malformed overlay queries default to `findAll` (12 tests)
- Tool name detection in audit events fixed
- Parent key auto-set restored (no dev mode — every agent has auth)

---

## Dependency Graph

```
#310 MessageBox probe ──────────┐
                                ├──► #311 Cross-wallet messaging ──┐
#312 overlay_lookup in runner ──┘                                  │
                                                                   ├──► #313 Agent ingestion
                                                                   │        │
                                                                   │        ├──► #314 Two-agent handshake ──► #316 Pipeline
                                                                   │        │
                                                                   └──► #315 Payment-in-message ────────────►
                                                                   
#317 DolphinMilkShake configs (independent) ✅
```

---

## What's Verified

### Overlay (shipped in #309)
- Agent self-registration on startup: `register_on_overlay()` builds 6-field AGENT PushDrop, submits BEEF
- Check-then-register: `check_registered()` queries `ls_agent` by identity key
- Lookup: `overlay_lookup()` queries any overlay service, parses BEEF → structured AgentRecords
- Tool: `overlay_lookup` registered in runner + MCP, discoverable via `search_tools`
- **Live verified:** 2 mainnet registration txids, 4/4 BEEF outputs decoded from live overlay
- **22 unit + 4 live tests**

### MessageBox Relay
- Relay alive at `messagebox.babbage.systems`
- BRC-31 handshake succeeds, server identity key matches
- All 4 inboxes accessible (task_inbox, results_inbox, status_inbox, dolphin_milk_coordination)
- Self-delivery quote: 0 sats (free)
- **4 live probe tests**

### Cross-Wallet Messaging
- Plain JSON: wallet A (3322) → relay → wallet B (3323)
- BRC-77 signed: signature verified on receive (`signature_valid=Some(true)`)
- BRC-78 encrypted: ciphertext decrypted on receive, body matches
- Signed + encrypted: both layers work together
- Message hash: identical SHA-256 on sender and receiver sides
- Full round-trip: A→B→A with reply referencing original
- **6 live tests** (run with `--test-threads=1`)

### Quality Baseline
- **3,340+ Rust tests** pass (unit + integration)
- **0 clippy warnings**
- cargo fmt clean

---

## What's Next

### Gate 4: Agent Ingestion (#313)
Start a dolphin-milk server, send it a MessageBox message from an external wallet. Verify the heartbeat picks it up, spawns a task, processes with LLM, and replies. **~5K sats.**

### Gate 5: Two-Agent Handshake (#314)
Two running dolphin-milk servers discover each other via overlay, exchange tasks via MessageBox, process, and reply. Verify proof chains on both sides. **~10K sats.**

### Gate 6: Payment-in-Message (#315)
Agent A creates a BSV transaction, includes BEEF in message body. Agent B internalizes via wallet. Verify balance changes. **~1K sats.**

### Gate 7: Captain→Coral Pipeline (#316)
Full DolphinMilkShake research cycle: Captain commissions scraping, Coral scrapes Reddit, returns results, Captain records provenance. **~20K sats.**

---

## Infrastructure

| Component | URL / Port | Status |
|-----------|-----------|--------|
| Wallet A | localhost:3322 | Running, funded |
| Wallet B | localhost:3323 | Running, funded |
| Overlay | rust-overlay.dev-a3e.workers.dev | Live, 4 agents registered |
| MessageBox | messagebox.babbage.systems | Live, BRC-31 auth works |
| x402 LLM | openai-chat.x402agency.com | Live (verified via Playwright E2E) |

## Running the Tests

```bash
# Unit tests (free, no wallet needed)
cargo test --test test_overlay_lookup --test test_overlay_registration --test test_overlay

# Live overlay tests (needs wallet on 3322, costs ~500 sats)
cargo test --test test_overlay_live -- --ignored --nocapture

# MessageBox probe (needs wallet on 3322, free)
cargo test --test test_messagebox_probe -- --ignored --nocapture

# Cross-wallet messaging (needs wallets on 3322 + 3323, ~100 sats)
cargo test --test test_cross_wallet_messaging -- --ignored --test-threads=1 --nocapture

# All overlay tests together
cargo test --test test_overlay_lookup --test test_overlay_registration --test test_overlay \
  && cargo test --test test_overlay_live --test test_messagebox_probe --test test_cross_wallet_messaging \
     -- --ignored --test-threads=1 --nocapture
```
