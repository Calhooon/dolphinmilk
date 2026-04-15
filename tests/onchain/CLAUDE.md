# tests/onchain

> 226 tests across 12 files covering the on-chain subsystem: budget tracking, BRC-18 proofs (12 proof types), BRC-48 state tokens, compliance metadata, state verification, consistency checks, proof audit, state provenance, and basket health monitoring.

## Overview

Tests for the modules in `src/onchain/` plus `src/budget.rs`, escalation proofs, and related config types. These tests verify the economic and cryptographic guarantees that make the agent's spending auditable and tamper-evident: multi-window budget enforcement, SHA-256 proof commitments with chain linking, Pay-to-Push-Drop script construction, regulatory compliance metadata, on-chain state verification, consistency checks against wallet state, full proof audit with independent hash recomputation, state root provenance, and basket UTXO health monitoring. All tests are offline — no wallet or network required (mock wallet via mockito for async tests).

## Files

| File | Tests | Description |
|------|------:|-------------|
| test_budget.rs | 43 | `BudgetTracker` with 6-window limits (task/hour/day/week/month/lifetime), JSONL persistence, replay, advisory vs strict enforcement, certificate overrides, `BudgetReport` serialization |
| test_state.rs | 32 | `build_push_drop_script` opcode correctness, `StateToken` lifecycle, `TokenType` basket assignment, `parse_push_drop_fields` roundtrip, `LifecycleConfig` defaults/TOML/hot-reload, legacy encryption, `created_at` enrichment |
| test_proofs.rs | 23 | `ProofCommitment` construction/verification, tamper detection (data, timestamp, hash, prev_hash), OP_RETURN script building, JSON roundtrips, chain linking, `conversation_integrity_proof` |
| test_proof_audit.rs | 23 | Full audit of all 12 proof types: independent `recompute_hash()` verification, tamper detection across 4 fields, deterministic hashing, full 12-type chain linking |
| test_verify_state.rs | 22 | `parse_op_return_hash` validation (known value, all-zeros, short, wrong prefix, too-long, invalid hex), `verify_state_against_proofs` consistency/divergence detection, `read_proof_chain` via mock wallet (4 async tests), `VerificationResult`/`OnChainProof`/`StateDivergence` serde, `verify_interval` config |
| test_compliance.rs | 17 | `ComplianceMetadata` tag strings and serde, `decision_proof_with_compliance`, `ComplianceConfig` defaults/TOML/hot-reload, WORM mode, env var parsing, retention math |
| test_escalation_proofs.rs | 16 | `ProofType::Escalation` display/serde, `escalation_proof()` data format, `EscalationReason::trigger_type_str()` for all 4 variants, `EscalationDetector::build_proof_data()`, chain linking, tamper detection |
| test_consistency_check.rs | 16 | `check_consistency()` comparing in-memory loop state against on-chain basket counts, `ConsistencyResult`/`ConsistencyCheck`/`ActiveTokenSummary` serde, `read_active_token_summary()` via mockito, high-count tolerance, boundary cases, zero-sats edge case |
| test_custody_proof.rs | 12 | `ProofType::Custody` display/serde, `custody_proof()` with 12 fields, self vs external customer key, certificate serial presence/absence, chain linking, deterministic hashing, tamper detection |
| test_message_proofs.rs | 11 | `ProofType::MessageSend`/`MessageReceive` display/serde, `message_send_proof()` and `message_receive_proof()` data format, chain linking, `ProofCommitment` serialization roundtrips |
| test_state_provenance.rs | 7 | `compute_state_root()` SHA-256 over 4 token txids: determinism, input sensitivity, None handling, partial inputs, hex format validation |
| test_basket_health.rs | 4 | Basket UTXO health in system prompt: `dm-*` basket names, section appears when populated, skipped when empty, high counts render as data without false-alarm warnings |

## Key types under test

### Budget (`src/budget.rs`)

| Type | Purpose |
|------|---------|
| `BudgetTracker` | Tracks satoshi spending across 6 time windows with JSONL persistence |
| `BudgetLimits` | Per-task, hourly, daily, weekly, monthly, lifetime caps + enforcement mode |
| `SpendingEntry` | Single spending record (timestamp, service, operation, sats, details) |
| `BudgetReport` | Serializable snapshot with per-service breakdown, remaining limits, balance |

### Proofs (`src/onchain/proofs.rs`)

| Type | Purpose |
|------|---------|
| `ProofCommitment` | SHA-256 hash commitment over (type, data, timestamp, prev_hash) |
| `ProofResult` | Commitment + txid after on-chain broadcast |
| `ProofType` | 12 variants: Decision, TaskCompletion, BudgetSnapshot, MemoryCommitment, CapabilityProof, ConversationIntegrity, Escalation, ConversationBreak, MessageSend, MessageReceive, Custody, CertificateRevocation |
| `ComplianceMetadata` | Regulations, retention days, classification — appended to proof data |
| `OnChainProof` | Parsed proof from blockchain: txid + hash |
| `StateDivergence` | Single field mismatch between in-memory and on-chain state |
| `VerificationResult` | Consistency check result: consistent flag, chain length, divergences, last hash |
| `build_op_return_script()` | Constructs `OP_FALSE OP_RETURN <32-byte hash>` hex script |
| `parse_op_return_hash()` | Extracts 32-byte hash from OP_RETURN script hex |
| `verify_state_against_proofs()` | Compares in-memory iteration/hash against on-chain proof chain |
| `read_proof_chain()` | Reads proof UTXOs from wallet via `listOutputs` (async, mockito-tested) |
| `decision_proof()` | Helper: creates Decision commitment with formatted DECISION/REASONING data |
| `task_completion_proof()` | Helper: creates TaskCompletion commitment with TASK/RESULT data |
| `memory_commitment_proof()` | Helper: creates MemoryCommitment with entry count and hash list |
| `conversation_integrity_proof()` | Helper: creates ConversationIntegrity with conversation ID, head hash, message count |
| `escalation_proof()` | Helper: creates Escalation commitment with trigger, iteration, budget, error count, tools |
| `custody_proof()` | Helper: creates Custody commitment with 12 fields (customer key, task hash, iterations, duration, tools, sats, result hash, chain head/length, agent key, cert serial) |
| `message_send_proof()` | Helper: creates MessageSend commitment with recipient, message box, delivery cost, signed/encrypted flags |
| `message_receive_proof()` | Helper: creates MessageReceive commitment with sender, message box |
| `decision_proof_with_compliance()` | Helper: creates Decision commitment with appended compliance metadata tag string |
| `capability_proof()` | Helper: creates CapabilityProof with tool name, args/result hashes, sats, txid |
| `chain_break_proof()` | Helper: creates ConversationBreak with expected and actual chain hashes |

### State tokens (`src/onchain/state.rs`)

| Type | Purpose |
|------|---------|
| `StateToken` | BRC-48 token with type, JSON data, satoshis, optional txid/vout |
| `TokenType` | 4 variants: TaskCommitment, BudgetAllocation, CapabilityDeclaration, Checkpoint |
| `TokenResult` | Token + txid after on-chain creation |
| `build_push_drop_script()` | Constructs Pay-to-Push-Drop locking script with OP_DROP/OP_2DROP + OP_CHECKSIG |
| `parse_push_drop_fields()` | Extracts hex-encoded fields from a PushDrop script |
| `is_encrypted_token_data()` | Detects legacy-encrypted token payloads |
| `BASKET_STATE`, `BASKET_BUDGET`, `BASKET_PROOFS`, `BASKET_REVOCATION` | BRC-46 basket name constants (`dm-state`, `dm-budget`, `dm-proofs`, `dm-revocation`) |
| `MIN_TOKEN_SATS` | Minimum satoshis per token output |

### Escalation (`src/runner/escalation.rs`)

| Type | Purpose |
|------|---------|
| `EscalationDetector` | Builds escalation events and proof data from agent loop state |
| `EscalationReason` | 4 variants: ToolLoop, BudgetThreshold, ErrorThreshold, ExplicitUncertainty |
| `EscalationProofData` | Serializable proof data: reason, trigger_type, iteration |

### Config types

| Type | Purpose |
|------|---------|
| `LifecycleConfig` | Token sweep parameters: max age hours, auto-sweep toggle, interval, verify_interval |
| `ComplianceConfig` | WORM mode, retention days, regulation list, classification labels |

### Consistency checks (`src/onchain/state.rs`)

| Type | Purpose |
|------|---------|
| `check_consistency()` | Compares in-memory loop state (iteration, sats_spent) against on-chain basket token counts (takes `HashMap<String, u64>`) |
| `ConsistencyResult` | Result: consistent flag + array of `ConsistencyCheck` entries |
| `ConsistencyCheck` | Single check: field name, status (ok/diverged), in_memory value, on_chain value |
| `ActiveTokenSummary` | Basket snapshot: basket name, UTXO count, token type list |
| `read_active_token_summary()` | Reads active token counts from wallet via `listOutputs` (async, mockito-tested) |

### State provenance (`src/onchain/state.rs`)

| Type | Purpose |
|------|---------|
| `compute_state_root()` | SHA-256 hash over 4 token txids (task_commitment, budget_allocation, capability_declaration, checkpoint) — deterministic fingerprint of on-chain token state |

### System prompt (`src/context/prompt.rs`)

| Type | Purpose |
|------|---------|
| `PromptContext.basket_health` | `HashMap<String, usize>` — basket name to UTXO count, rendered in system prompt |
| `PromptContext.spendable_output_count` | `u64` — total spendable outputs in default basket, rendered when > 0 |

## Test patterns

### Budget enforcement (test_budget.rs)

Tests set specific `BudgetLimits` with one window tight and others large, then verify the correct error message fires:

```rust
let limits = BudgetLimits {
    max_per_task: 500,
    max_per_hour: 10_000_000,
    ..Default::default()
};
let mut tracker = BudgetTracker::new(None, limits);
tracker.record("llm", "think", 400, Value::Null);
let err = tracker.check_limit(200).unwrap_err();
assert!(err.to_string().contains("Task budget limit reached"));
```

Time-window expiration tested via `insert_entry()` with backdated timestamps (e.g., `Utc::now() - Duration::hours(192)` for 8-day-old entries outside the 7-day weekly window).

### Proof chain linking (test_proofs.rs)

Chain of commitments where each references the previous hash:

```rust
let c1 = ProofCommitment::with_timestamp(ProofType::Decision, "step 1", "t1", None);
let c2 = ProofCommitment::with_timestamp(ProofType::Decision, "step 2", "t2", Some(&c1.hash_hex()));
assert_eq!(c2.prev_hash.as_deref(), Some(c1.hash_hex().as_str()));
// Same data+timestamp but different prev_hash → different hash
let c2_no_chain = ProofCommitment::with_timestamp(ProofType::Decision, "step 2", "t2", None);
assert_ne!(c2.hash, c2_no_chain.hash);
```

### PushDrop script verification (test_state.rs)

Script opcode assertions verify correct Bitcoin script structure:

- 1 field: `<data> OP_DROP <pubkey> OP_CHECKSIG`
- 2 fields: `<f1> <f2> OP_2DROP <pubkey> OP_CHECKSIG`
- 3 fields: `<f1> <f2> OP_2DROP <f3> OP_DROP <pubkey> OP_CHECKSIG`
- 4 fields: `<f1> <f2> OP_2DROP <f3> <f4> OP_2DROP <pubkey> OP_CHECKSIG`
- 100+ byte data: `OP_PUSHDATA1` prefix

### Legacy encryption (test_state.rs)

Tests use hardcoded keys (`0xDE` at byte 0, `0xAD` at byte 31) and SHA-256 of `b"brc48-state-token-v1"` as key ID. Verifies no plaintext leaks into script bytes and wrong-key decryption fails.

### State verification (test_verify_state.rs)

Tests `verify_state_against_proofs()` which compares in-memory agent state against on-chain proof chain:

```rust
let proofs = vec![
    OnChainProof { txid: "tx1".into(), hash: "aabbccdd".into() },
];
let result = verify_state_against_proofs(&proofs, 1, 500, Some("aabbccdd"));
assert!(result.consistent);
```

Divergence detection: empty chain with non-zero iteration produces `chain_length` divergence; mismatched last hash produces `last_proof_hash` divergence. The `read_proof_chain()` function is tested with mockito against wallet `listOutputs`, including limit enforcement and non-OP_RETURN filtering.

### Escalation proofs (test_escalation_proofs.rs)

Full flow tested: `EscalationReason` → `build_event()` → `build_proof_data()` → `escalation_proof()`. Each `EscalationReason` variant maps to a `trigger_type_str()`:

- `ToolLoop { tool_name, call_count }` → `"tool_loop"`
- `BudgetThreshold { spent, limit, percent }` → `"budget_threshold"`
- `ErrorThreshold { error_count }` → `"error_threshold"`
- `ExplicitUncertainty { trigger_phrase }` → `"explicit_uncertainty"`

### Custody proofs (test_custody_proof.rs)

`custody_proof()` takes 12 parameters and embeds them as labeled fields in the proof data string:

```
CUSTODY_PROOF: v1
CUSTOMER_KEY: 02abc123 | self
TASK_HASH: sha256:aabbccdd
ITERATIONS: 5
DURATION_SECS: 120
TOOLS_USED: memory_search, file_read
TOTAL_SATS: 15000
RESULT_HASH: sha256:deadbeef
PROOF_CHAIN_HEAD: prev1234
PROOF_CHAIN_LENGTH: 3
AGENT_KEY: 02agentkey
AGENT_CERT: cert-serial-001 | none
```

### Proof audit (test_proof_audit.rs)

Independent verification of all 12 proof types with a `recompute_hash()` helper that re-derives SHA-256(prev_hash_bytes || data || timestamp) outside the ProofCommitment code path. Tests confirm that proof type is NOT included in hash computation (type-agnostic hashing). Full 12-type chain test links all proof types in sequence (c1 → c2 → ... → c12).

### Consistency checks (test_consistency_check.rs)

Tests `check_consistency()` which compares in-memory loop state against on-chain basket counts. Uses `BASKET_STATE`/`BASKET_BUDGET` constants (now "dm-state"/"dm-budget") for HashMap lookups, but check result field names are still legacy "worm-state_count"/"worm-budget_count". Divergence logic: empty basket + (iteration > 0 OR sats_spent > 0) = diverged. High token counts are expected (historical accumulation) and never flagged — only empty baskets when work has been done trigger divergence. Edge case: sats_spent=0 with no budget tokens is consistent.

### State provenance (test_state_provenance.rs)

Tests `compute_state_root()` which produces a deterministic SHA-256 fingerprint from 4 optional token txids. Any change to any input produces a different root. All-None inputs still produce a valid 64-char hex hash. Used for consistency checks and proof chains.

### Basket health rendering (test_basket_health.rs)

Tests that `build_system_prompt()` renders basket UTXO counts as raw data without false-alarm warnings. Basket names use "dm-*" prefix: "dm-state", "dm-budget", "dm-proofs", "dm-revocation". Token accumulation across tasks is expected — high counts are informational, not alerts.

## Helpers

| Helper | File | Purpose |
|--------|------|---------|
| `test_pubkey()` | test_state.rs | Returns secp256k1 generator point G (valid compressed pubkey) |
| `legacy_test_key()` | test_state.rs | 32-byte key with `0xDE` at [0] and `0xAD` at [31] |
| `legacy_test_key_id()` | test_state.rs | SHA-256 of `b"brc48-state-token-v1"` |
| `default_ctx()` | test_basket_health.rs | Factory for zeroed-out `PromptContext` with empty `basket_health` |
| `recompute_hash()` | test_proof_audit.rs | Independent SHA-256(prev_hash_bytes \|\| data \|\| timestamp) recomputation |
| `assert_hash_matches()` | test_proof_audit.rs | Verifies commitment hash matches `recompute_hash()` output |

## Running

```bash
# All onchain tests
cargo test --test test_budget --test test_proofs --test test_proof_audit --test test_state \
  --test test_state_provenance --test test_compliance --test test_verify_state \
  --test test_consistency_check --test test_basket_health --test test_custody_proof \
  --test test_escalation_proofs --test test_message_proofs

# Individual files
cargo test --test test_budget
cargo test --test test_proofs
cargo test --test test_proof_audit
cargo test --test test_state
cargo test --test test_state_provenance
cargo test --test test_compliance
cargo test --test test_verify_state
cargo test --test test_consistency_check
cargo test --test test_basket_health
cargo test --test test_custody_proof
cargo test --test test_escalation_proofs
cargo test --test test_message_proofs

# Pattern match across all test files
cargo test budget
cargo test proof_chain
cargo test push_drop
cargo test compliance
cargo test advisory
cargo test custody
cargo test escalation
cargo test message_send
cargo test verify_state
cargo test consistency
cargo test state_root
cargo test basket_health
cargo test audit
```

## Gotchas

- **Valid secp256k1 pubkeys only.** `build_push_drop_script` validates pubkey structure. Use `test_pubkey()` (generator point G), never random hex.
- **Legacy encryption keys are hardcoded.** `legacy_test_key()` and `legacy_test_key_id()` must not change — they test a specific encryption format.
- **`tempfile` for JSONL tests.** Budget persistence tests use `tempdir()` for isolated filesystem. The `TempDir` must stay in scope.
- **`insert_entry()` for time travel.** Budget window expiration tests use `insert_entry()` with backdated `SpendingEntry` timestamps rather than sleeping.
- **`max_lifetime = 0` means unlimited.** Zero is the sentinel for "no limit" on weekly/monthly/lifetime windows. Tests verify this explicitly.
- **`task_sats()` resets independently.** After `reset_task()`, `task_sats()` returns 0 but `hourly_sats()` and `daily_sats()` still include old entries.
- **Replay does not count toward `task_sats()`.** `replay_log()` restores entries for hourly/daily/weekly/monthly/lifetime windows but the per-task counter stays at 0.
- **Async tests use mockito.** `test_verify_state.rs` has 4 `#[tokio::test]` tests using `mockito::Server::new_async()` for wallet `listOutputs` mocking.
- **`parse_op_return_hash` expects exact format.** Rejects wrong opcodes, wrong push length, too-long payloads, and invalid hex. The prefix must be `006a20` (OP_FALSE OP_RETURN PUSH32).
- **`verify_interval = 0` disables verification.** The `LifecycleConfig.verify_interval` field defaults to 10 (every 10 iterations); 0 means disabled.
- **Proof hash is type-agnostic.** `ProofType` is NOT included in the SHA-256 computation — only data, timestamp, and prev_hash. Two proofs with different types but identical data/timestamp/prev_hash produce the same hash (verified in `test_proof_audit.rs`).
- **`compute_state_root()` accepts all-None.** Four None txids still produce a valid 64-char hex hash (SHA-256 of empty concat). Tests in `test_state_provenance.rs` verify this explicitly.
- **Consistency checks flag emptiness, not excess.** `check_consistency()` only flags empty baskets when iteration > 0 or sats_spent > 0. High token counts (hundreds or thousands) are normal historical accumulation.
- **Basket constants rebranded.** `BASKET_STATE` etc. are now "dm-*" (post Dolphin Milk rebrand), but `check_consistency()` field names are still hardcoded as legacy "worm-state_count"/"worm-budget_count". Tests assert the legacy field names.
- **`read_active_token_summary()` also uses mockito.** `test_consistency_check.rs` has 1 `#[tokio::test]` for wallet mock, building real PushDrop scripts via `build_push_drop_script()`.

## Related

- [tests/CLAUDE.md](../CLAUDE.md) -- Parent test directory overview, all 13 domain groups
- [src/onchain/](../../src/onchain/) -- Source modules: `budget.rs`, `proofs.rs`, `state.rs`
- [src/runner/escalation.rs](../../src/runner/) -- `EscalationDetector`, `EscalationReason`, `EscalationProofData`
- [src/context/prompt.rs](../../src/context/) -- `build_system_prompt`, `PromptContext` with `basket_health`
- [src/config/CLAUDE.md](../../src/config/CLAUDE.md) -- `BudgetConfig`, `LifecycleConfig`, `ComplianceConfig`
- [src/memory/CLAUDE.md](../../src/memory/CLAUDE.md) -- `legacy_encrypt`/`legacy_decrypt` used in state token encryption tests
- [tests/features/](../features/) -- `test_verification.rs` for higher-level proof chain integrity tests
