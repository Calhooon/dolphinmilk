//! BRC-48 state tokens — spendable on-chain state via Pay-to-Push-Drop.
//!
//! State tokens are UTXOs that carry arbitrary data in their locking script.
//! Unlike OP_RETURN (BRC-18), these are **spendable** — the worm can update
//! state by spending the old token and creating a new one (spend-and-recreate).
//!
//! Script template:
//! ```text
//! <data1> <data2> ... OP_DROP [OP_2DROP ...] <pubkey> OP_CHECKSIG
//! ```
//!
//! BRC-46 baskets organize tokens by purpose:
//! - `worm-state`:  current task status tokens
//! - `worm-budget`: budget allocation tokens
//! - `worm-proofs`: references to BRC-18 proof txids

use bsv::primitives::ec::PublicKey;
use bsv::script::templates::{LockPosition, PushDrop};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::DmError;
use crate::memory::encrypt;
use crate::wallet::WalletBackend;

/// Well-known BRC-46 basket names.
pub const BASKET_STATE: &str = "dm-state";
pub const BASKET_BUDGET: &str = "dm-budget";
pub const BASKET_PROOFS: &str = "dm-proofs";
pub const BASKET_REVOCATION: &str = "dm-revocation";

/// Minimum satoshis for a state token output (must be non-dust).
pub const MIN_TOKEN_SATS: u64 = 1;

/// Types of state tokens the worm manages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TokenType {
    /// Active task commitment (task hash, priority, deadline).
    TaskCommitment,
    /// Budget allocation (amount, purpose, expiry).
    BudgetAllocation,
    /// Capability declaration (tool list, version).
    CapabilityDeclaration,
    /// Checkpoint (memory hash, context size).
    Checkpoint,
}

impl std::fmt::Display for TokenType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TaskCommitment => write!(f, "task_commitment"),
            Self::BudgetAllocation => write!(f, "budget_allocation"),
            Self::CapabilityDeclaration => write!(f, "capability_declaration"),
            Self::Checkpoint => write!(f, "checkpoint"),
        }
    }
}

/// BRC-46 basket for a given token type.
impl TokenType {
    pub fn basket(&self) -> &'static str {
        match self {
            Self::TaskCommitment | Self::CapabilityDeclaration | Self::Checkpoint => BASKET_STATE,
            Self::BudgetAllocation => BASKET_BUDGET,
        }
    }
}

/// A state token with on-chain metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateToken {
    pub token_type: TokenType,
    /// Structured data stored in the token (serialized to JSON in script).
    pub data: Value,
    /// Transaction ID when the token was created/last updated.
    pub txid: Option<String>,
    /// Output index within the transaction.
    pub vout: Option<u32>,
    /// Satoshis locked in this token.
    pub satoshis: u64,
}

/// Result of creating or updating a state token on-chain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenResult {
    pub txid: String,
    pub token: StateToken,
}

impl StateToken {
    /// Create a new in-memory state token (not yet on-chain).
    pub fn new(token_type: TokenType, data: Value) -> Self {
        Self {
            token_type,
            data,
            txid: None,
            vout: None,
            satoshis: MIN_TOKEN_SATS,
        }
    }

    /// Create with a custom satoshi amount.
    pub fn with_sats(token_type: TokenType, data: Value, satoshis: u64) -> Self {
        Self {
            token_type,
            data,
            txid: None,
            vout: None,
            satoshis: satoshis.max(MIN_TOKEN_SATS),
        }
    }

    /// Whether this token has been committed on-chain.
    pub fn is_on_chain(&self) -> bool {
        self.txid.is_some()
    }
}

/// Protocol ID for state token encryption via wallet.
fn state_protocol_id() -> serde_json::Value {
    serde_json::json!([2, "dolphin milk state"])
}

/// Decrypt token data that was encrypted via the wallet.
///
/// Returns the deserialized JSON value.
pub async fn decrypt_token_data_with_wallet(
    wallet: &dyn WalletBackend,
    encrypted: &[u8],
    counterparty: &str,
) -> Result<Value, DmError> {
    let plaintext = if encrypt::is_legacy_format(encrypted) {
        // Legacy format: extract key_id, derive key, decrypt locally
        let key_id = encrypt::legacy_extract_key_id(encrypted)?;
        let aes_key = encrypt::legacy_derive_key(wallet, &key_id).await?;
        encrypt::legacy_decrypt(&aes_key, encrypted)?
    } else {
        encrypt::decrypt_with_wallet(
            wallet,
            encrypted,
            &state_protocol_id(),
            "tokens",
            counterparty,
        )
        .await?
    };
    serde_json::from_slice(&plaintext)
        .map_err(|e| DmError::wallet(format!("Failed to deserialize decrypted token data: {e}")))
}

/// Check whether a byte slice looks like encrypted token data.
///
/// Returns `true` if the data starts with legacy magic bytes OR is not
/// valid JSON (wallet-encrypted blobs are opaque binary, not JSON).
pub fn is_encrypted_token_data(data: &[u8]) -> bool {
    if data.len() < 4 {
        return false;
    }
    // Legacy format check
    if data[..4] == encrypt::ENCRYPT_VERSION {
        return true;
    }
    // Wallet ciphertext won't be valid JSON
    serde_json::from_slice::<Value>(data).is_err()
}

/// Build a Pay-to-Push-Drop locking script (BRC-48) using the BSV SDK.
///
/// Format: `<data_fields...> OP_DROP/OP_2DROP... <pubkey> OP_CHECKSIG`
///
/// Uses the SDK's `PushDrop` which handles minimal encoding (OP_0 for
/// empty pushes, OP_1..OP_16 for small values) and proper drop counting.
///
/// Returns the script as a hex string.
pub fn build_push_drop_script(
    data_fields: &[&[u8]],
    owner_pubkey_hex: &str,
) -> Result<String, DmError> {
    if data_fields.is_empty() {
        return Err(DmError::wallet(
            "At least one data field required for Push-Drop",
        ));
    }

    let pubkey = PublicKey::from_hex(owner_pubkey_hex)
        .map_err(|e| DmError::wallet(format!("Invalid pubkey: {e}")))?;

    let fields: Vec<Vec<u8>> = data_fields.iter().map(|f| f.to_vec()).collect();

    let pushdrop = PushDrop::new(pubkey, fields).with_position(LockPosition::After);
    let locking_script = pushdrop.lock();

    Ok(locking_script.to_hex())
}

/// Create a state token on-chain via the wallet.
///
/// Cost: ~200-500 sats per token creation (transaction fee + 1 sat output).
///
/// When `encrypt` is `true`, the token data JSON is encrypted via the wallet
/// before being placed in the script. The token type remains plaintext
/// (visible via basket name anyway, useful for filtering).
pub async fn create_token(
    wallet: &dyn WalletBackend,
    token: StateToken,
    encrypt_data: bool,
    counterparty: &str,
) -> Result<TokenResult, DmError> {
    create_token_in_basket(wallet, token, encrypt_data, counterparty, None).await
}

/// Create a state token in a specific basket (overriding the default for the token type).
pub async fn create_token_in_basket(
    wallet: &dyn WalletBackend,
    token: StateToken,
    encrypt_data: bool,
    counterparty: &str,
    basket_override: Option<&str>,
) -> Result<TokenResult, DmError> {
    // Get identity key for the OP_CHECKSIG lock
    let pubkey = wallet.get_identity_key().await?;

    // Enrich token data with created_at timestamp for lifecycle management
    let enriched_data = {
        let mut d = token.data.clone();
        if let Some(obj) = d.as_object_mut() {
            obj.insert(
                "created_at".to_string(),
                serde_json::json!(chrono::Utc::now().timestamp()),
            );
        }
        d
    };

    // Serialize token data as JSON bytes
    let data_json = serde_json::to_vec(&enriched_data)
        .map_err(|e| DmError::wallet(format!("Failed to serialize token data: {e}")))?;

    // Optionally encrypt the data via wallet
    let data_bytes = if encrypt_data {
        encrypt::encrypt_with_wallet(
            wallet,
            &data_json,
            &state_protocol_id(),
            "tokens",
            counterparty,
        )
        .await?
    } else {
        data_json
    };

    // Serialize token type as bytes
    let type_bytes = token.token_type.to_string().into_bytes();

    // Store created_at as a separate unencrypted field so sweep_stale_tokens
    // can determine token age even when data is encrypted.
    let created_at_ts = chrono::Utc::now().timestamp().to_string();
    let created_at_bytes = created_at_ts.into_bytes();

    // Build the script: [type, data, created_at] OP_DROP OP_2DROP <pubkey> OP_CHECKSIG
    let data_fields: Vec<&[u8]> = vec![&type_bytes, &data_bytes, &created_at_bytes];
    let script = build_push_drop_script(&data_fields, &pubkey)?;

    let basket = basket_override.unwrap_or_else(|| token.token_type.basket());

    let output = serde_json::json!({
        "lockingScript": script,
        "satoshis": token.satoshis,
        "outputDescription": "dolphin milk state",
        "basket": basket,
    });

    let result = wallet
        .create_action(&[output], "dolphin milk state", false, false)
        .await?;

    let mut updated_token = token;
    updated_token.txid = Some(result.txid.clone());
    updated_token.vout = Some(0); // first output

    Ok(TokenResult {
        txid: result.txid,
        token: updated_token,
    })
}

/// Parse PushDrop data fields from a locking script hex string.
///
/// PushDrop format: [push data1] [push data2] ... OP_2DROP/OP_DROP <pubkey> OP_CHECKSIG
/// We extract the pushed data fields before the first OP_DROP (0x75) or OP_2DROP (0x6d).
pub fn parse_push_drop_fields(script_hex: &str) -> Option<Vec<String>> {
    let bytes = hex::decode(script_hex).ok()?;
    let mut fields = Vec::new();
    let mut i = 0;

    while i < bytes.len() {
        let op = bytes[i];
        match op {
            // Direct push: 1-75 bytes
            1..=75 => {
                let len = op as usize;
                if i + 1 + len > bytes.len() {
                    break;
                }
                fields.push(hex::encode(&bytes[i + 1..i + 1 + len]));
                i += 1 + len;
            }
            // OP_PUSHDATA1
            0x4c => {
                if i + 1 >= bytes.len() {
                    break;
                }
                let len = bytes[i + 1] as usize;
                if i + 2 + len > bytes.len() {
                    break;
                }
                fields.push(hex::encode(&bytes[i + 2..i + 2 + len]));
                i += 2 + len;
            }
            // OP_PUSHDATA2
            0x4d => {
                if i + 2 >= bytes.len() {
                    break;
                }
                let len = u16::from_le_bytes([bytes[i + 1], bytes[i + 2]]) as usize;
                if i + 3 + len > bytes.len() {
                    break;
                }
                fields.push(hex::encode(&bytes[i + 3..i + 3 + len]));
                i += 3 + len;
            }
            // OP_DROP (0x75), OP_2DROP (0x6d) -- end of data fields
            0x75 | 0x6d => break,
            // OP_0 (0x00) -- empty push
            0x00 => {
                fields.push(String::new());
                i += 1;
            }
            // Any other opcode means we've hit script operations -- stop
            _ => break,
        }
    }

    if fields.is_empty() {
        None
    } else {
        Some(fields)
    }
}

/// Extract the `created_at` Unix timestamp from an output's token data.
///
/// Token data is expected to be JSON with an optional `created_at` field (Unix seconds).
/// Falls back to looking at the outpoint string for a rough creation indicator.
fn extract_created_at_from_output(output: &Value) -> Option<i64> {
    // Try parsing PushDrop data fields from the locking script
    if let Some(script_hex) = output
        .get("lockingScript")
        .and_then(|v| v.as_str())
        .or_else(|| output.get("locking_script").and_then(|v| v.as_str()))
    {
        if let Some(fields) = parse_push_drop_fields(script_hex) {
            // New format (3+ fields): 3rd field is unencrypted created_at timestamp
            if fields.len() >= 3 {
                if let Ok(bytes) = hex::decode(&fields[2]) {
                    if let Ok(ts_str) = String::from_utf8(bytes) {
                        if let Ok(ts) = ts_str.parse::<i64>() {
                            return Some(ts);
                        }
                    }
                }
            }
            // Legacy fallback: try to decode the 2nd field (data) as JSON
            for field_hex in &fields {
                if let Ok(bytes) = hex::decode(field_hex) {
                    if let Ok(json) = serde_json::from_slice::<Value>(&bytes) {
                        if let Some(ts) = json.get("created_at").and_then(|v| v.as_i64()) {
                            return Some(ts);
                        }
                    }
                }
            }
        }
    }
    None
}

/// Extract the token type string from an output's PushDrop data.
fn extract_token_type_from_output(output: &Value) -> Option<String> {
    let script_hex = output
        .get("lockingScript")
        .and_then(|v| v.as_str())
        .or_else(|| output.get("locking_script").and_then(|v| v.as_str()))?;
    let fields = parse_push_drop_fields(script_hex)?;
    // First field is the token type label
    let type_hex = fields.first()?;
    let bytes = hex::decode(type_hex).ok()?;
    String::from_utf8(bytes).ok()
}

/// Extract outpoint (txid, vout) from a wallet output JSON.
fn extract_outpoint(output: &Value) -> Option<(String, u32)> {
    // Wallet returns "outpoint" as "txid.vout"
    if let Some(op) = output.get("outpoint").and_then(|v| v.as_str()) {
        let parts: Vec<&str> = op.split('.').collect();
        if parts.len() == 2 {
            let txid = parts[0].to_string();
            let vout: u32 = parts[1].parse().ok()?;
            return Some((txid, vout));
        }
    }
    // Fallback: separate fields
    let txid = output.get("txid").and_then(|v| v.as_str())?.to_string();
    let vout = output.get("vout").and_then(|v| v.as_u64())? as u32;
    Some((txid, vout))
}

/// Sweep stale tokens from a basket. Tokens older than `max_age_secs` are relinquished.
/// When `token_type_filter` is `Some`, only tokens matching that type string are swept.
/// When `compliance_retention_secs` is `Some`, tokens within the retention period are preserved.
/// Returns count of tokens swept.
pub async fn sweep_stale_tokens(
    wallet: &dyn WalletBackend,
    basket: &str,
    max_age_secs: i64,
    token_type_filter: Option<&str>,
) -> Result<usize, DmError> {
    sweep_stale_tokens_with_retention(wallet, basket, max_age_secs, token_type_filter, None).await
}

/// Sweep stale tokens with optional compliance retention enforcement.
pub async fn sweep_stale_tokens_with_retention(
    wallet: &dyn WalletBackend,
    basket: &str,
    max_age_secs: i64,
    token_type_filter: Option<&str>,
    compliance_retention_secs: Option<i64>,
) -> Result<usize, DmError> {
    let cutoff = chrono::Utc::now().timestamp() - max_age_secs;
    let mut swept = 0usize;
    let mut offset = 0u64;
    let limit = 100u64;

    loop {
        let result = wallet
            .list_outputs(basket, "locking scripts", limit, offset)
            .await?;
        let outputs = match result.get("outputs").and_then(|v| v.as_array()) {
            Some(arr) => arr.clone(),
            None => break,
        };
        if outputs.is_empty() {
            break;
        }

        for output in &outputs {
            // Check token type filter
            if let Some(filter) = token_type_filter {
                if let Some(ref tt) = extract_token_type_from_output(output) {
                    if tt != filter {
                        continue;
                    }
                } else {
                    continue; // Can't determine type, skip
                }
            }

            // Check age
            if let Some(created_at) = extract_created_at_from_output(output) {
                // Compliance retention: skip tokens still within retention period
                if let Some(retention_secs) = compliance_retention_secs {
                    let retention_cutoff = chrono::Utc::now().timestamp() - retention_secs;
                    if created_at > retention_cutoff {
                        continue; // Within retention period, do not sweep
                    }
                }

                if created_at < cutoff {
                    if let Some((txid, vout)) = extract_outpoint(output) {
                        match wallet.relinquish_output(basket, &txid, vout).await {
                            Ok(_) => {
                                swept += 1;
                                tracing::debug!("Swept stale token {txid}:{vout} from {basket}");
                            }
                            Err(e) => {
                                tracing::warn!("Failed to sweep token {txid}:{vout}: {e}");
                            }
                        }
                    }
                }
            }
        }

        if outputs.len() < limit as usize {
            break;
        }
        offset += limit;
    }

    if swept > 0 {
        tracing::info!("Swept {swept} stale tokens from basket '{basket}'");
    }
    Ok(swept)
}

/// Get lifecycle status for a basket: count of tokens and oldest age in hours.
pub async fn basket_status(wallet: &dyn WalletBackend, basket: &str) -> (u64, Option<f64>) {
    let now = chrono::Utc::now().timestamp();

    // Use totalOutputs from first page for count — avoids O(n) pagination.
    // Only scan first page for oldest created_at (best-effort age estimate).
    let result = match wallet.list_outputs(basket, "locking scripts", 100, 0).await {
        Ok(r) => r,
        Err(_) => return (0, None),
    };

    let count = result
        .get("totalOutputs")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    let outputs = match result.get("outputs").and_then(|v| v.as_array()) {
        Some(arr) => arr,
        None => return (count, None),
    };

    // Scan first page for oldest created_at
    let mut oldest_created_at: Option<i64> = None;
    for output in outputs {
        if let Some(ts) = extract_created_at_from_output(output) {
            match oldest_created_at {
                Some(prev) if ts < prev => oldest_created_at = Some(ts),
                None => oldest_created_at = Some(ts),
                _ => {}
            }
        }
    }

    let oldest_age_hours = oldest_created_at.map(|ts| (now - ts) as f64 / 3600.0);
    (count, oldest_age_hours)
}

/// Update a state token by spending the old one and creating a new one.
///
/// This is the "spend-and-recreate" pattern: the on-chain state transitions
/// from old_token (spent) → new_token (created), forming an immutable audit trail.
pub async fn update_token(
    wallet: &dyn WalletBackend,
    old_token: &StateToken,
    new_data: Value,
    encrypt_data: bool,
    counterparty: &str,
) -> Result<TokenResult, DmError> {
    // Create the new token first
    let new_token = StateToken::new(old_token.token_type, new_data);
    let result = create_token(wallet, new_token, encrypt_data, counterparty).await?;

    // Relinquish the old token from its basket (best-effort)
    if let (Some(old_txid), Some(old_vout)) = (&old_token.txid, old_token.vout) {
        let basket = old_token.token_type.basket();
        match wallet.relinquish_output(basket, old_txid, old_vout).await {
            Ok(_) => {
                tracing::debug!(
                    "Relinquished old {} token: {}:{}",
                    old_token.token_type,
                    old_txid,
                    old_vout,
                );
            }
            Err(e) => {
                tracing::warn!(
                    "Failed to relinquish old {} token {}:{} (non-fatal): {e}",
                    old_token.token_type,
                    old_txid,
                    old_vout,
                );
            }
        }
    }

    Ok(result)
}

// ---------------------------------------------------------------------------
// BRC-48 Token Consistency Check (#196)
// ---------------------------------------------------------------------------

/// Summary of active BRC-48 tokens read from a basket.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveTokenSummary {
    pub basket: String,
    pub count: u64,
    pub token_types: Vec<String>,
}

/// Read active tokens from a basket and return a summary.
///
/// Paginates through `list_outputs` to count tokens and extract their types.
pub async fn read_active_token_summary(
    wallet: &dyn WalletBackend,
    basket: &str,
    limit: usize,
) -> Result<ActiveTokenSummary, DmError> {
    let mut count = 0u64;
    let mut token_types = Vec::new();
    let mut offset = 0u64;
    let page_size = 100u64;

    loop {
        let result = wallet
            .list_outputs(basket, "locking scripts", page_size, offset)
            .await?;
        let outputs = match result.get("outputs").and_then(|v| v.as_array()) {
            Some(arr) => arr.clone(),
            None => break,
        };
        if outputs.is_empty() {
            break;
        }

        for output in &outputs {
            if count as usize >= limit {
                break;
            }
            count += 1;
            if let Some(tt) = extract_token_type_from_output(output) {
                if !token_types.contains(&tt) {
                    token_types.push(tt);
                }
            }
        }

        if count as usize >= limit || outputs.len() < page_size as usize {
            break;
        }
        offset += page_size;
    }

    Ok(ActiveTokenSummary {
        basket: basket.to_string(),
        count,
        token_types,
    })
}

/// Result of comparing in-memory state against on-chain BRC-48 tokens.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsistencyResult {
    pub consistent: bool,
    pub checks: Vec<ConsistencyCheck>,
}

/// A single field check within a consistency result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsistencyCheck {
    pub field: String,
    /// "ok" or "diverged"
    pub status: String,
    pub in_memory: String,
    pub on_chain: String,
}

/// Compare in-memory basket health data against expected BRC-48 token counts.
///
/// Checks:
/// - `worm-state` should have 1-50 tokens. >50 suggests token accumulation.
/// - `worm-budget` should have 1-25 tokens. >25 suggests stale budget tokens.
/// - Missing baskets in `basket_health` are flagged if the agent should have tokens.
///
/// Only flags **missing** tokens as divergence — an empty basket when the agent
/// should have tokens indicates a real problem (tokens lost or wallet issue).
///
/// Token accumulation across tasks is expected and intentional: TaskCommitment
/// and CapabilityDeclaration tokens persist as historical on-chain state that
/// documents what the agent committed to and what capabilities it had. High
/// counts are reported for visibility but are NOT flagged as divergence.
pub fn check_consistency(
    iteration: u32,
    sats_spent: u64,
    basket_health: &std::collections::HashMap<String, u64>,
) -> ConsistencyResult {
    let mut checks = Vec::new();

    // Check worm-state basket: flag only if empty when it shouldn't be.
    // High counts are normal — tokens accumulate across tasks as historical state.
    let state_count = basket_health.get(BASKET_STATE).copied().unwrap_or(0);
    let state_status = if iteration > 0 && state_count == 0 {
        "diverged"
    } else {
        "ok"
    };
    checks.push(ConsistencyCheck {
        field: "worm-state_count".to_string(),
        status: state_status.to_string(),
        in_memory: format!("iteration={iteration}"),
        on_chain: format!("count={state_count}"),
    });

    // Check worm-budget basket: flag only if empty when it shouldn't be.
    let budget_count = basket_health.get(BASKET_BUDGET).copied().unwrap_or(0);
    let budget_status = if sats_spent > 0 && budget_count == 0 {
        "diverged"
    } else {
        "ok"
    };
    checks.push(ConsistencyCheck {
        field: "worm-budget_count".to_string(),
        status: budget_status.to_string(),
        in_memory: format!("sats_spent={sats_spent}"),
        on_chain: format!("count={budget_count}"),
    });

    let consistent = checks.iter().all(|c| c.status == "ok");
    ConsistencyResult { consistent, checks }
}

// ---------------------------------------------------------------------------
// State provenance — link decisions to their input state (#202)
// ---------------------------------------------------------------------------

/// Compute a state root hash from the current active BRC-48 token txids.
///
/// The state root is SHA-256(task_commitment_txid || budget_allocation_txid ||
/// capability_declaration_txid || checkpoint_txid). Missing tokens contribute
/// empty strings.
pub fn compute_state_root(
    task_commitment_txid: Option<&str>,
    budget_allocation_txid: Option<&str>,
    capability_declaration_txid: Option<&str>,
    checkpoint_txid: Option<&str>,
) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(task_commitment_txid.unwrap_or("").as_bytes());
    hasher.update(budget_allocation_txid.unwrap_or("").as_bytes());
    hasher.update(capability_declaration_txid.unwrap_or("").as_bytes());
    hasher.update(checkpoint_txid.unwrap_or("").as_bytes());
    hex::encode(hasher.finalize())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_token_type_display() {
        assert_eq!(TokenType::TaskCommitment.to_string(), "task_commitment");
        assert_eq!(TokenType::BudgetAllocation.to_string(), "budget_allocation");
        assert_eq!(
            TokenType::CapabilityDeclaration.to_string(),
            "capability_declaration"
        );
        assert_eq!(TokenType::Checkpoint.to_string(), "checkpoint");
    }

    #[test]
    fn test_token_type_basket() {
        assert_eq!(TokenType::TaskCommitment.basket(), BASKET_STATE);
        assert_eq!(TokenType::BudgetAllocation.basket(), BASKET_BUDGET);
        assert_eq!(TokenType::CapabilityDeclaration.basket(), BASKET_STATE);
        assert_eq!(TokenType::Checkpoint.basket(), BASKET_STATE);
    }

    #[test]
    fn test_token_type_serialize_roundtrip() {
        for tt in &[
            TokenType::TaskCommitment,
            TokenType::BudgetAllocation,
            TokenType::CapabilityDeclaration,
            TokenType::Checkpoint,
        ] {
            let json = serde_json::to_string(tt).unwrap();
            let back: TokenType = serde_json::from_str(&json).unwrap();
            assert_eq!(&back, tt);
        }
    }

    #[test]
    fn test_state_token_new() {
        let t = StateToken::new(TokenType::TaskCommitment, json!({"task": "build proofs"}));
        assert_eq!(t.token_type, TokenType::TaskCommitment);
        assert_eq!(t.data["task"], "build proofs");
        assert!(!t.is_on_chain());
        assert_eq!(t.satoshis, MIN_TOKEN_SATS);
    }

    #[test]
    fn test_state_token_with_sats() {
        let t = StateToken::with_sats(TokenType::BudgetAllocation, json!({}), 500);
        assert_eq!(t.satoshis, 500);
    }

    #[test]
    fn test_state_token_with_sats_minimum() {
        let t = StateToken::with_sats(TokenType::BudgetAllocation, json!({}), 0);
        assert_eq!(t.satoshis, MIN_TOKEN_SATS);
    }

    #[test]
    fn test_state_token_is_on_chain() {
        let mut t = StateToken::new(TokenType::Checkpoint, json!({}));
        assert!(!t.is_on_chain());
        t.txid = Some("abc".to_string());
        assert!(t.is_on_chain());
    }

    #[test]
    fn test_state_token_serialize_roundtrip() {
        let mut t = StateToken::new(
            TokenType::TaskCommitment,
            json!({"task": "test", "priority": 1}),
        );
        t.txid = Some("deadbeef".to_string());
        t.vout = Some(0);

        let json = serde_json::to_string(&t).unwrap();
        let back: StateToken = serde_json::from_str(&json).unwrap();
        assert_eq!(back.token_type, TokenType::TaskCommitment);
        assert_eq!(back.txid, Some("deadbeef".to_string()));
        assert_eq!(back.vout, Some(0));
        assert_eq!(back.data["task"], "test");
    }

    // Valid secp256k1 pubkey (generator point G) for script tests.
    const TEST_PUBKEY: &str = "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";

    #[test]
    fn test_build_push_drop_script_single_field() {
        let data = b"hello";
        let script_hex = build_push_drop_script(&[data], TEST_PUBKEY).unwrap();
        let script = hex::decode(&script_hex).unwrap();

        // [5] hello OP_DROP [33] pubkey OP_CHECKSIG
        assert_eq!(script[0], 5); // push 5 bytes
        assert_eq!(&script[1..6], b"hello");
        assert_eq!(script[6], 0x75); // OP_DROP
        assert_eq!(script[7], 33); // push 33 bytes
        assert_eq!(*script.last().unwrap(), 0xac); // OP_CHECKSIG
        assert_eq!(script.len(), 42); // 1+5+1+1+33+1
    }

    #[test]
    fn test_build_push_drop_script_two_fields() {
        let d1 = b"type";
        let d2 = b"data";
        let script_hex = build_push_drop_script(&[d1, d2], TEST_PUBKEY).unwrap();
        let script = hex::decode(&script_hex).unwrap();

        // [4] type [4] data OP_2DROP [33] pubkey OP_CHECKSIG
        assert_eq!(script[0], 4);
        assert_eq!(&script[1..5], b"type");
        assert_eq!(script[5], 4);
        assert_eq!(&script[6..10], b"data");
        assert_eq!(script[10], 0x6d); // OP_2DROP
        assert_eq!(script[11], 33);
        assert_eq!(*script.last().unwrap(), 0xac); // OP_CHECKSIG
    }

    #[test]
    fn test_build_push_drop_script_three_fields() {
        let d1 = b"a";
        let d2 = b"b";
        let d3 = b"c";
        let script_hex = build_push_drop_script(&[d1, d2, d3], TEST_PUBKEY).unwrap();
        let script = hex::decode(&script_hex).unwrap();

        // 3 items: 1 OP_2DROP + 1 OP_DROP
        let two_drops = script.iter().filter(|&&b| b == 0x6d).count();
        let one_drops = script.iter().filter(|&&b| b == 0x75).count();
        assert_eq!(two_drops, 1, "3 fields: one OP_2DROP");
        assert_eq!(one_drops, 1, "3 fields: one OP_DROP");
        assert_eq!(*script.last().unwrap(), 0xac); // OP_CHECKSIG
    }

    #[test]
    fn test_build_push_drop_script_empty_fields_error() {
        let result = build_push_drop_script(&[], TEST_PUBKEY);
        assert!(result.is_err());
    }

    #[test]
    fn test_build_push_drop_script_invalid_pubkey() {
        let result = build_push_drop_script(&[b"data"], "not_hex");
        assert!(result.is_err());
    }

    #[test]
    fn test_build_push_drop_script_wrong_pubkey_length() {
        let result = build_push_drop_script(&[b"data"], "0200");
        assert!(result.is_err());
    }

    #[test]
    fn test_build_push_drop_script_valid_hex_output() {
        let data = b"test data for token";
        let script_hex = build_push_drop_script(&[data], TEST_PUBKEY).unwrap();
        // Must be valid hex
        assert!(hex::decode(&script_hex).is_ok());
    }

    #[test]
    fn test_token_result_serialize() {
        let t = StateToken::new(TokenType::Checkpoint, json!({"hash": "abc123"}));
        let result = TokenResult {
            txid: "tx123".to_string(),
            token: t,
        };
        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("tx123"));
        assert!(json.contains("Checkpoint"));
    }

    #[test]
    fn test_basket_constants() {
        assert_eq!(BASKET_STATE, "dm-state");
        assert_eq!(BASKET_BUDGET, "dm-budget");
        assert_eq!(BASKET_PROOFS, "dm-proofs");
    }

    // -----------------------------------------------------------------------
    // Token encryption tests (legacy format — still used for reading old data)
    // -----------------------------------------------------------------------

    fn legacy_test_key() -> [u8; 32] {
        let mut k = [0u8; 32];
        k[0] = 0xDE;
        k[31] = 0xAD;
        k
    }

    fn legacy_test_key_id() -> [u8; 32] {
        use sha2::Digest;
        sha2::Sha256::digest(b"brc48-state-token-v1").into()
    }

    #[test]
    fn test_legacy_encrypt_decrypt_token_data_roundtrip() {
        let key = legacy_test_key();
        let key_id = legacy_test_key_id();
        let data = json!({"task": "build proofs", "priority": 1});
        let data_json = serde_json::to_vec(&data).unwrap();

        let encrypted = crate::memory::encrypt::legacy_encrypt(&key, &key_id, &data_json).unwrap();
        let plaintext = crate::memory::encrypt::legacy_decrypt(&key, &encrypted).unwrap();
        let decrypted: Value = serde_json::from_slice(&plaintext).unwrap();
        assert_eq!(decrypted, data);
    }

    #[test]
    fn test_is_encrypted_token_data_true_for_legacy() {
        let key = legacy_test_key();
        let key_id = legacy_test_key_id();
        let encrypted = crate::memory::encrypt::legacy_encrypt(&key, &key_id, b"test").unwrap();
        assert!(is_encrypted_token_data(&encrypted));
    }

    #[test]
    fn test_is_encrypted_token_data_false_for_json() {
        // Valid JSON is plaintext, not encrypted
        assert!(!is_encrypted_token_data(b"{\"key\":\"value\"}"));
        assert!(!is_encrypted_token_data(&[]));
    }

    #[test]
    fn test_is_encrypted_token_data_true_for_binary() {
        // Binary blobs that aren't valid JSON are detected as encrypted
        assert!(is_encrypted_token_data(&[
            0x01, 0x02, 0x03, 0x04, 0x05, 0x06
        ]));
    }

    #[test]
    fn test_legacy_encrypted_data_not_readable_as_plaintext() {
        let key = legacy_test_key();
        let key_id = legacy_test_key_id();
        let secret = "super secret task description";
        let data = json!({"task": secret});
        let data_json = serde_json::to_vec(&data).unwrap();

        let encrypted = crate::memory::encrypt::legacy_encrypt(&key, &key_id, &data_json).unwrap();

        let encrypted_str = String::from_utf8_lossy(&encrypted);
        assert!(
            !encrypted_str.contains(secret),
            "Plaintext leaked in encrypted data"
        );
    }

    #[test]
    fn test_legacy_encrypted_data_in_push_drop_script_no_plaintext() {
        let key = legacy_test_key();
        let key_id = legacy_test_key_id();
        let secret = "super secret task";
        let data = json!({"task": secret});
        let data_json = serde_json::to_vec(&data).unwrap();

        let encrypted = crate::memory::encrypt::legacy_encrypt(&key, &key_id, &data_json).unwrap();

        let type_bytes = b"task_commitment";
        let script_hex =
            build_push_drop_script(&[type_bytes.as_ref(), &encrypted], TEST_PUBKEY).unwrap();
        let script_bytes = hex::decode(&script_hex).unwrap();

        let script_str = String::from_utf8_lossy(&script_bytes);
        assert!(!script_str.contains(secret), "Plaintext leaked in script");
    }

    #[test]
    fn test_legacy_encryption_size_overhead() {
        let key = legacy_test_key();
        let key_id = legacy_test_key_id();
        let data_json = serde_json::to_vec(&json!({})).unwrap();
        let encrypted = crate::memory::encrypt::legacy_encrypt(&key, &key_id, &data_json).unwrap();

        // Overhead: 4 (version) + 32 (key_id) + 32 (IV) + 16 (tag) = 84 bytes
        let overhead = encrypted.len() - data_json.len();
        assert_eq!(
            overhead, 84,
            "Encryption overhead should be exactly 84 bytes"
        );
    }

    #[test]
    fn test_legacy_wrong_key_fails_decrypt_token() {
        let key = legacy_test_key();
        let key_id = legacy_test_key_id();
        let data = json!({"task": "test"});
        let data_json = serde_json::to_vec(&data).unwrap();

        let encrypted = crate::memory::encrypt::legacy_encrypt(&key, &key_id, &data_json).unwrap();

        let mut wrong_key = key;
        wrong_key[0] = 0xFF;

        let result = crate::memory::encrypt::legacy_decrypt(&wrong_key, &encrypted);
        assert!(result.is_err(), "Wrong key should fail decryption");
    }
}
