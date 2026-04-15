# src/onchain/
> On-chain state tokens, immutable proofs, and per-service budget tracking.

## Overview

Three modules that handle everything blockchain-facing beyond raw wallet calls. `proofs.rs` creates immutable BRC-18 OP_RETURN proof hashes (~200 sats each) with 12 proof types, optional compliance metadata, on-chain proof chain reading, and memory tamper detection. `state.rs` manages spendable BRC-48 PushDrop state tokens (spend-and-recreate pattern) plus UTXO lifecycle management (stale token sweeping, basket status). `budget.rs` tracks per-service spending with six-tier limits, enforcement modes, and JSONL audit persistence.

## Files

| File | Lines | Purpose |
|------|-------|---------|
| `mod.rs` | 5 | Re-exports `budget`, `proofs`, `state` |
| `budget.rs` | 775 | `BudgetTracker` — per-service spending accumulator with JSONL log, six-tier limits (task/hourly/daily/weekly/monthly/lifetime), enforcement modes (strict/advisory), time-windowed queries, log replay for restart recovery, merge support, certificate-derived limit overrides, and report generation |
| `proofs.rs` | 804 | BRC-18 OP_RETURN proof chain — `ProofCommitment` with SHA-256 hash linking, `create_proof()` via wallet, 12 proof types with convenience constructors, `ComplianceMetadata` for regulatory tagging, on-chain proof reading (`read_proof_chain`, `verify_state_against_proofs`), memory tamper detection (`compute_memory_hash`, `verify_memory_hash`) |
| `state.rs` | 1043 | BRC-48 PushDrop state tokens — `StateToken` CRUD, `create_token()`/`create_token_in_basket()`/`update_token()` via wallet, optional encryption, 4 BRC-46 baskets, PushDrop field parsing, UTXO lifecycle management (`sweep_stale_tokens`, `basket_status`), consistency checks (`check_consistency`, `compute_state_root`), active token summary reading, legacy encryption support |

## Key Exports

### budget.rs

| Export | Description |
|--------|-------------|
| `BudgetTracker` | Main struct. Holds `HashMap<String, ServiceBudget>`, entries vec, optional JSONL log path, limits, and task-scoped counter |
| `BudgetTracker::new()` | Constructor with optional JSONL log path and `BudgetLimits`. Used directly for task-level trackers (no replay needed) |
| `BudgetTracker::from_config()` | Creates tracker from `BudgetConfig` + workspace path, replays existing `budget.jsonl` to restore totals across restarts |
| `BudgetTracker::replay_log()` | Replays JSONL audit log to restore entries and per-service totals. Best-effort: malformed lines are skipped. `task_sats` intentionally left at zero (per-task counter starts fresh) |
| `BudgetTracker::record()` | Records a spending event (service, operation, sats, details JSON). Appends to JSONL log best-effort |
| `BudgetTracker::check_limit()` | Pre-flight check: returns `Err(WormError::Budget)` if spending `sats` would exceed any limit. Checks in order: task → hourly → daily → weekly → monthly → lifetime. Weekly/monthly/lifetime are skipped when set to 0 (unlimited) |
| `BudgetTracker::is_advisory()` | Returns `true` if enforcement mode is `"advisory"` (warn but don't block) |
| `BudgetTracker::report()` | Generates `BudgetReport` with balance=0. Delegates to `report_with_balance()` |
| `BudgetTracker::report_with_balance()` | Generates `BudgetReport` with per-service breakdown, all time-windowed totals (hourly through lifetime), remaining limits, and provided wallet balance |
| `BudgetTracker::reset_task()` | Resets task-scoped counter to 0 (all other totals preserved) |
| `BudgetTracker::task_sats()` | Returns current task spending counter |
| `BudgetTracker::hourly_sats()` | Total sats spent in the last hour (time-windowed) |
| `BudgetTracker::daily_sats()` | Total sats spent in the last 24 hours (time-windowed) |
| `BudgetTracker::weekly_sats()` | Total sats spent in the last 7 days (168 hours, time-windowed) |
| `BudgetTracker::monthly_sats()` | Total sats spent in the last 30 days (720 hours, time-windowed) |
| `BudgetTracker::lifetime_sats()` | Total sats across all time (sum of all entries) |
| `BudgetTracker::entries()` | Access raw `&[SpendingEntry]` for detailed reporting |
| `BudgetTracker::services_used()` | Sorted list of distinct service names with recorded spending |
| `BudgetTracker::set_last_task_sats()` | Sets `task_sats` to the last completed task's spending. Used by the global tracker so `GET /budget` returns meaningful per-task data between tasks (otherwise always 0 since each runner has its own tracker) |
| `BudgetTracker::apply_cert_limits()` | Overrides budget limits with BRC-52 certificate-derived values (non-zero fields only). Accepts per_task, per_hour, per_day, per_week, per_month, lifetime, and enforcement. Called from `runner/lifecycle.rs` after cert is loaded at task start |
| `BudgetTracker::limits()` | Returns the current effective `&BudgetLimits` (may be cert-derived or config-derived) |
| `BudgetTracker::insert_entry()` | Inserts a pre-built `SpendingEntry` directly. Updates in-memory state but does NOT append to JSONL log. Used for testing with specific timestamps |
| `BudgetTracker::merge_from()` | Merges entries from a task-level tracker into a global tracker. Updates in-memory state + appends to JSONL log. Does NOT update `task_sats` (entries belong to a different task context) |
| `SpendingEntry` | Single spending record: timestamp, service, operation, sats, optional details JSON (`skip_serializing_if = "Value::is_null"`) |
| `ServiceBudget` | Per-service accumulator: `total_sats` and `count` |
| `ServiceReport` | Serializable per-service report: `total_sats` and `count` |
| `BudgetLimits` | Six-tier limits struct with enforcement mode. Defaults: 250K task, 2.5M hourly, 25M daily, 100M weekly, 500M monthly, 0 lifetime (unlimited), `"strict"` enforcement. Created from `BudgetConfig` via `From` impl |
| `BudgetReport` | Serializable report: task_sats, total_operations, services map, hourly/daily/weekly/monthly/lifetime sats, limits, balance |
| `LimitsReport` | Serializable limits with remaining values: max + remaining for each of six tiers, plus enforcement mode |

### proofs.rs

| Export | Description |
|--------|-------------|
| `ProofType` | Enum with 12 variants: `Decision`, `TaskCompletion`, `BudgetSnapshot`, `MemoryCommitment`, `CapabilityProof`, `ConversationIntegrity`, `CertificateRevocation`, `Escalation`, `ConversationBreak`, `MessageSend`, `MessageReceive`, `Custody`. Has `Display` and `Serialize`/`Deserialize` |
| `ProofCommitment` | Hash-chained proof: `SHA-256(prev_hash_bytes \|\| data \|\| timestamp)`. Methods: `new()`, `with_timestamp()`, `verify()`, `hash_hex()` |
| `ProofResult` | Txid + commitment after on-chain creation |
| `ComplianceMetadata` | Optional regulatory metadata: `regulations` (vec of framework IDs), `retention_days`, `classification`. `to_tag_string()` formats as proof tag line |
| `create_proof()` | Async — builds OP_RETURN script, broadcasts via `wallet.create_action()` to `worm-proofs` basket |
| `build_op_return_script()` | `OP_FALSE OP_RETURN <32-byte hash>` → hex string. Uses BSV SDK `Script` |
| `decision_proof()` | Convenience: formats `DECISION: ...\nREASONING: ...` |
| `decision_proof_with_compliance()` | Convenience: decision proof with appended `ComplianceMetadata` tag string |
| `task_completion_proof()` | Convenience: formats `TASK: ...\nRESULT: ...` |
| `memory_commitment_proof()` | Convenience: formats `ENTRIES: N\nHASHES: ...` |
| `capability_proof()` | Convenience: formats tool name, args/result hashes (sha256-prefixed), sats, payment txid |
| `conversation_integrity_proof()` | Convenience: formats conv_id, head_hash, message count |
| `escalation_proof()` | Convenience: formats reason, trigger type, iteration, budget spent/cap, error count, last decision, tools attempted |
| `chain_break_proof()` | Convenience: formats conversation ID, expected vs actual hash, break point message index |
| `message_send_proof()` | Convenience: formats message hash, recipient, box name, delivery cost, signed/encrypted flags |
| `message_receive_proof()` | Convenience: formats message hash, sender, box name |
| `custody_proof()` | Convenience: formats customer key, task hash, iterations, duration, tools used, total sats, result hash, proof chain head/length, agent key, cert serial |
| `OnChainProof` | A proof read back from the blockchain: txid + hash (hex-encoded) |
| `StateDivergence` | Divergence between in-memory state and on-chain: field name, in_memory value, on_chain value |
| `VerificationResult` | Result of state verification: consistent flag, chain_length, divergences vec, last_proof_hash |
| `parse_op_return_hash()` | Parses the 32-byte hash from a BRC-18 OP_RETURN locking script hex. Expects `00 6a 20 <32 bytes>` (35 bytes total). Returns hex string or `None` |
| `read_proof_chain()` | Async — reads the agent's proof chain from blockchain via `wallet.list_outputs("worm-proofs")`. Paginates (100 per page), parses OP_RETURN hashes. Returns newest first |
| `verify_state_against_proofs()` | Compares on-chain proof chain against in-memory state. Checks chain length vs iteration count and last proof hash consistency. Returns `VerificationResult` with any divergences |
| `compute_memory_hash()` | SHA-256 of `content \|\| created_at \|\| category` for memory tamper detection. Returns hex string |
| `verify_memory_hash()` | Recomputes memory hash and compares against expected value. Returns bool |

### state.rs

| Export | Description |
|--------|-------------|
| `TokenType` | Enum: `TaskCommitment`, `BudgetAllocation`, `CapabilityDeclaration`, `Checkpoint`. Each maps to a BRC-46 basket via `.basket()` |
| `StateToken` | In-memory token: type, JSON data, optional txid/vout, satoshis. `new()` creates at `MIN_TOKEN_SATS` (1 sat). `is_on_chain()` checks txid presence |
| `StateToken::with_sats()` | Create with custom satoshi amount (clamped to `MIN_TOKEN_SATS` minimum) |
| `TokenResult` | Txid + token after on-chain creation |
| `create_token()` | Async — delegates to `create_token_in_basket()` with the token type's default basket |
| `create_token_in_basket()` | Async — enriches data with `created_at` timestamp, serializes to JSON, optionally encrypts via wallet (protocol `[2, "worm state"]`), builds PushDrop script, broadcasts via `wallet.create_action()`. Accepts optional `basket_override` to place the token in a non-default basket |
| `update_token()` | Async — spend-and-recreate: creates new token then relinquishes old one (best-effort) |
| `build_push_drop_script()` | `<fields...> OP_DROP/OP_2DROP <pubkey> OP_CHECKSIG` → hex string. Uses BSV SDK `PushDrop` with `LockPosition::After` |
| `parse_push_drop_fields()` | Parses PushDrop data fields from a locking script hex string. Handles direct push (1-75), OP_PUSHDATA1/2, OP_0. Stops at OP_DROP/OP_2DROP |
| `sweep_stale_tokens()` | Async — relinquishes tokens older than `max_age_secs` from a basket. Optional `token_type_filter`. Paginates via `list_outputs` (limit 100) |
| `sweep_stale_tokens_with_retention()` | Async — same as above but with optional `compliance_retention_secs` to preserve tokens within retention period |
| `basket_status()` | Async — returns `(u64, Option<f64>)` as `(count, oldest_age_hours)` for a basket. Uses `totalOutputs` from first page for count (avoids O(n) pagination). Scans only first page for oldest `created_at` (best-effort age estimate). `None` when no tokens have `created_at` |
| `decrypt_token_data_with_wallet()` | Async — decrypts token data, auto-detects legacy vs wallet-native format |
| `is_encrypted_token_data()` | Checks for legacy magic bytes or non-JSON binary |
| `ActiveTokenSummary` | Summary of active BRC-48 tokens read from a basket: basket name, count, distinct token_types |
| `read_active_token_summary()` | Async — paginates through `list_outputs` to count tokens and extract their types. Accepts a `limit` cap |
| `ConsistencyResult` | Result of BRC-48 consistency check: `consistent` flag, `checks` vec of `ConsistencyCheck` entries |
| `ConsistencyCheck` | Single field check: field name, status (`"ok"` or `"diverged"`), in_memory value, on_chain value |
| `check_consistency()` | Compares in-memory basket health against expected BRC-48 token counts. Flags **missing** tokens (empty basket when agent should have tokens) as divergence. High counts are reported but NOT flagged — token accumulation is intentional historical state |
| `compute_state_root()` | SHA-256 state root from active token txids: `SHA-256(task_commitment_txid \|\| budget_allocation_txid \|\| capability_declaration_txid \|\| checkpoint_txid)`. Missing tokens contribute empty strings |
| `BASKET_STATE` / `BASKET_BUDGET` / `BASKET_PROOFS` / `BASKET_REVOCATION` | Basket name constants: `"worm-state"`, `"worm-budget"`, `"worm-proofs"`, `"worm-revocation"` |
| `MIN_TOKEN_SATS` | Minimum satoshis for a state token output (1 sat) |

## BRC-46 Basket Organization

| Basket | Token Types | Purpose |
|--------|-------------|---------|
| `worm-state` | TaskCommitment, CapabilityDeclaration, Checkpoint | Historical agent state — tokens accumulate across tasks as on-chain provenance |
| `worm-budget` | BudgetAllocation | Budget tracking tokens — accumulate across tasks/iterations |
| `worm-proofs` | (BRC-18 OP_RETURN outputs) | Immutable proof chain — NEVER swept |
| `worm-revocation` | (revocation UTXOs) | Certificate/token revocation |

**Token accumulation is intentional.** TaskCommitment and CapabilityDeclaration tokens persist across tasks as historical on-chain state. They document what the agent committed to and what capabilities it had for each task. The heartbeat sweep only cleans up old Checkpoint tokens (superseded by newer ones) and BudgetAllocation tokens older than 7 days. The consistency check flags only **missing** tokens (empty basket), not high counts.

## Budget Limit Tiers

Six-tier spending limits with two enforcement modes:

| Tier | Default | Window | Skipped when |
|------|---------|--------|-------------|
| `max_per_task` | 250K sats | Current task | Never (always enforced) |
| `max_per_hour` | 2.5M sats | Rolling 1 hour | Never (always enforced) |
| `max_per_day` | 25M sats | Rolling 24 hours | Never (always enforced) |
| `max_per_week` | 100M sats | Rolling 7 days | Set to 0 (unlimited) |
| `max_per_month` | 500M sats | Rolling 30 days | Set to 0 (unlimited) |
| `max_lifetime` | 0 (unlimited) | All time | Set to 0 (unlimited) |

**Enforcement modes**: `"strict"` (default) rejects spending that would exceed limits. `"advisory"` logs warnings but allows the spend. `is_advisory()` checks the current mode. Both modes are settable via config or BRC-52 certificate overrides.

## Proof Types (proofs.rs)

12 proof types covering the full agent lifecycle:

| ProofType | Display | Data Format | Created By |
|-----------|---------|-------------|------------|
| `Decision` | `decision` | `DECISION: ...\nREASONING: ...` | Runner per-iteration |
| `TaskCompletion` | `task_completion` | `TASK: ...\nRESULT: ...` | Runner at task end |
| `BudgetSnapshot` | `budget_snapshot` | Balance + allocations | Runner periodic |
| `MemoryCommitment` | `memory_commitment` | `ENTRIES: N\nHASHES: ...` | Memory system |
| `CapabilityProof` | `capability_proof` | Tool name, args/result hashes, sats, txid | Runner after tool call |
| `ConversationIntegrity` | `conversation_integrity` | Conv ID, head hash, message count | Conversation system |
| `CertificateRevocation` | `certificate_revocation` | Revocation record | Certificate manager |
| `Escalation` | `escalation` | Reason, trigger, iteration, budget, errors, tools | Runner on escalation |
| `ConversationBreak` | `conversation_break` | Conv ID, expected/actual hash, break point | Hash chain verifier |
| `MessageSend` | `message_send` | Hash, recipient, box, cost, signed/encrypted | Messaging system |
| `MessageReceive` | `message_receive` | Hash, sender, box | Messaging system |
| `Custody` | `custody` | Customer key, task hash, iterations, duration, tools, sats, result hash, chain head/length, agent key, cert | Runner at task end |

## On-Chain Proof Reading (BRC Read Loop)

`proofs.rs` supports reading the agent's own proof chain back from the blockchain and verifying in-memory state consistency:

1. **`parse_op_return_hash()`**: Extracts the 32-byte SHA-256 hash from a BRC-18 OP_RETURN locking script. Validates the exact 35-byte format (`00 6a 20 <32 bytes>`)
2. **`read_proof_chain()`**: Async — paginates through the `worm-proofs` basket via `wallet.list_outputs()`, parses each output's locking script to extract the proof hash. Returns `Vec<OnChainProof>` (txid + hash), newest first
3. **`verify_state_against_proofs()`**: Compares on-chain proof chain against in-memory state. Detects two divergence types: (a) empty chain when iterations > 0, (b) last proof hash mismatch. Returns `VerificationResult` with `consistent` flag, chain length, divergences list, and last proof hash

## Memory Tamper Detection

Hash-based integrity checking for memory entries:

- **`compute_memory_hash()`**: `SHA-256(content || created_at || category)` → deterministic hex string stored alongside each memory entry
- **`verify_memory_hash()`**: Recomputes and compares against expected hash. Returns `true` if content is untampered

## Usage Patterns

### Budget tracking (runner calls per-iteration)
```rust
// Pre-flight check before spending
tracker.check_limit(estimated_sats)?;
// Record after successful payment
tracker.record("llm", "think", actual_sats, json!({"model": "gpt-5"}));
// Reset task counter between tasks
tracker.reset_task();
// Merge task-level entries into global tracker (server)
global_tracker.merge_from(task_tracker.entries());
```

### Proof chain (runner creates per-iteration)
```rust
// Chain proofs via prev_hash for verifiable ordering
let commitment = decision_proof(&decision, &reasoning, prev_hash.as_deref());
let result = create_proof(&wallet, commitment).await?;
// Pass result.commitment.hash_hex() as prev_hash to next proof

// With compliance metadata (when compliance mode enabled)
let compliance = ComplianceMetadata {
    regulations: vec!["SEC-17a-4".into()],
    retention_days: Some(2555),
    classification: Some("financial".into()),
};
let commitment = decision_proof_with_compliance(&decision, &reasoning, prev_hash, &compliance);
```

### Reading and verifying proof chain
```rust
// Read latest proofs from blockchain
let proofs = read_proof_chain(&wallet, 100).await?;

// Verify in-memory state against on-chain proofs
let result = verify_state_against_proofs(&proofs, iteration, sats_spent, last_proof_hash);
if !result.consistent {
    for div in &result.divergences {
        tracing::warn!("Divergence in {}: memory={} chain={}", div.field, div.in_memory, div.on_chain);
    }
}
```

### State tokens (runner manages lifecycle)
```rust
// Task start: create commitment + budget allocation
let token = StateToken::new(TokenType::TaskCommitment, json!({...}));
let result = create_token(&wallet, token, true, "self").await?;
// Per-iteration: update budget allocation (spend-and-recreate)
let updated = update_token(&wallet, &old_token, new_data, true, "self").await?;
// Task end: relinquish commitment
```

### UTXO lifecycle management
```rust
// Sweep stale tokens older than 24 hours
let swept = sweep_stale_tokens(&wallet, BASKET_STATE, 86400, Some("checkpoint")).await?;

// Sweep with compliance retention (preserve tokens within 7-year window)
let swept = sweep_stale_tokens_with_retention(
    &wallet, BASKET_STATE, 86400, Some("checkpoint"),
    Some(7 * 365 * 86400), // 7-year retention
).await?;

// Check basket health
let (count, oldest_hours) = basket_status(&wallet, BASKET_BUDGET).await;
```

### PushDrop field parsing
```rust
// Parse fields from an existing token's locking script
if let Some(fields) = parse_push_drop_fields(script_hex) {
    // fields[0] = token type (hex-encoded UTF-8)
    // fields[1] = token data (hex-encoded JSON or encrypted bytes)
    // fields[2] = created_at Unix timestamp (hex-encoded UTF-8, unencrypted)
}
```

### Memory tamper detection
```rust
// Compute hash when storing a memory entry
let hash = compute_memory_hash(&content, &created_at, &category);
// Later, verify integrity
assert!(verify_memory_hash(&content, &created_at, &category, &hash));
```

## Compliance Metadata (proofs.rs)

`ComplianceMetadata` is optionally attached to proofs when compliance mode is enabled:

| Field | Type | Example |
|-------|------|---------|
| `regulations` | `Vec<String>` | `["SEC-17a-4", "FINRA-3110"]` |
| `retention_days` | `Option<u64>` | `2555` (7 years) |
| `classification` | `Option<String>` | `"financial"`, `"operational"`, `"communication"` |

`to_tag_string()` produces: `COMPLIANCE: SEC-17a-4,FINRA-3110 | RETAIN: 2555d | CLASS: financial`

## UTXO Lifecycle (state.rs)

Token lifecycle management for preventing basket bloat:

1. **`created_at` enrichment**: `create_token_in_basket()` adds `created_at` two ways: inside the token data JSON (for decrypted access) and as a separate third PushDrop field (unencrypted, for age queries even when data is encrypted)
2. **Field extraction**: `extract_created_at_from_output()` parses PushDrop fields — tries the 3rd field first (new format), falls back to parsing JSON from the 2nd field (legacy format)
3. **Type extraction**: `extract_token_type_from_output()` reads the first PushDrop field as the token type label
4. **Outpoint extraction**: `extract_outpoint()` parses `"txid.vout"` format or separate `txid`/`vout` fields
5. **Sweeping**: `sweep_stale_tokens()` paginates through basket outputs, filters by type and age, relinquishes stale tokens (best-effort per token)
6. **Compliance retention**: `sweep_stale_tokens_with_retention()` adds a second age check — tokens within the retention window are preserved even if older than `max_age_secs`
7. **Status**: `basket_status()` returns total count (via `totalOutputs` from first page) and oldest token age in hours (first-page scan only, best-effort)

## Encryption

State tokens support optional encryption via the wallet:
- **Protocol ID**: `[2, "worm state"]`, key ID `"tokens"`, counterparty `"self"`
- **Legacy format**: Magic bytes `0x42421033`, 84-byte overhead (4 version + 32 key_id + 32 IV + 16 tag)
- **Detection**: `is_encrypted_token_data()` checks legacy magic or non-JSON binary
- **Decryption**: `decrypt_token_data_with_wallet()` auto-detects format and routes accordingly

## Related

- [`../CLAUDE.md`](../CLAUDE.md) — Parent module overview with file inventory
- `../wallet.rs` — All wallet HTTP calls used by `create_token()`, `update_token()`, `create_proof()`, `read_proof_chain()`
- `../runner/` — Primary consumer: creates proofs per-iteration, manages token lifecycle
- `../memory/encrypt.rs` — Legacy encryption primitives (`legacy_encrypt`, `legacy_decrypt`, `is_legacy_format`)
- `../server/handlers/budget.rs` — Serves `BudgetReport` via `GET /budget` and `GET /budget/detail`
- `../server/handlers/tasks.rs` — Serves proof txids via `GET /task/{id}/proofs` and verification via `GET /task/{id}/proofs/verify`
- `../heartbeat/` — Calls `sweep_stale_tokens()` and `basket_status()` for UTXO lifecycle management
- `../memory/` — Uses `compute_memory_hash()` and `verify_memory_hash()` for tamper detection
