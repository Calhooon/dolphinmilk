//! Shared types for the wallet module.

use serde_json::Value;

/// "Anyone" key: secp256k1 generator point G (private key = 1).
/// Used for fund_from_woc internalization.
pub const ANYONE_KEY: &str = "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";

/// Result from createAction.
pub struct CreateActionResult {
    pub txid: String,
    pub tx: Vec<u8>,
    pub raw: Value,
}
