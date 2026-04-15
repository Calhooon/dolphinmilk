//! Excess refund handling.
//!
//! When an x402 server overcharges and includes an excessRefund in the
//! response body, this module parses and internalizes it.
//!
//! The refund transaction may have multiple outputs (refund + server change).
//! We derive the expected receiving key from the BRC-29 derivation params,
//! parse the transaction via the BSV SDK, and find the output whose P2PKH
//! hash160 matches our derived key. This prevents internalizing the wrong
//! output when the server places outputs in an unexpected order.

use serde_json::Value;

use crate::error::X402Error;
use crate::payment::{hash160, payment_protocol};
use crate::traits::WalletApi;

/// Parsed refund info from server response.
#[derive(Debug)]
pub struct RefundInfo {
    pub transaction: String,
    pub derivation_prefix: String,
    pub derivation_suffix: String,
    pub sender_identity_key: String,
    pub satoshis: u64,
    /// Server-specified output index (if provided).
    pub output_index: Option<u32>,
}

/// Parse refund info from response body.
///
/// Looks for `excessRefund` or `refund` key in the JSON body.
pub fn parse_refund(body: &Value) -> Option<RefundInfo> {
    let refund = body
        .get("excessRefund")
        .or_else(|| body.get("refund"))
        .and_then(|v| v.as_object())?;

    // Check for already_refunded flag
    if refund
        .get("already_refunded")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        return None;
    }

    let transaction = refund.get("transaction")?.as_str()?.to_string();
    let derivation_prefix = refund.get("derivationPrefix")?.as_str()?.to_string();
    let derivation_suffix = refund.get("derivationSuffix")?.as_str()?.to_string();
    let sender_identity_key = refund.get("senderIdentityKey")?.as_str()?.to_string();
    let satoshis = refund.get("satoshis")?.as_u64()?;
    let output_index = refund
        .get("outputIndex")
        .and_then(|v| v.as_u64())
        .map(|v| v as u32);

    Some(RefundInfo {
        transaction,
        derivation_prefix,
        derivation_suffix,
        sender_identity_key,
        satoshis,
        output_index,
    })
}

/// Internalize a refund transaction into the wallet.
///
/// Derives the expected refund key from the BRC-29 derivation parameters,
/// parses the transaction to find the matching output, then internalizes it.
/// Falls back to server-specified outputIndex, then to index 0.
pub async fn process_refund(
    wallet: &dyn WalletApi,
    refund: &RefundInfo,
) -> Result<Value, X402Error> {
    use base64::engine::general_purpose::STANDARD as BASE64;
    use base64::Engine;

    let tx_bytes = BASE64
        .decode(&refund.transaction)
        .map_err(|e| X402Error::payment(format!("Invalid base64 in refund tx: {e}")))?;

    // Determine the correct output index:
    // 1. Derive expected key and find matching P2PKH output (most reliable)
    // 2. Use server-specified outputIndex if provided
    // 3. Fall back to 0
    let output_index = match derive_and_find_output(wallet, refund, &tx_bytes).await {
        Some(idx) => {
            tracing::debug!(
                output_index = idx,
                "Refund: found matching P2PKH output via key derivation"
            );
            idx
        }
        None => {
            let fallback = refund.output_index.unwrap_or(0);
            tracing::warn!(
                fallback_index = fallback,
                "Refund: could not derive matching output, using fallback index"
            );
            fallback
        }
    };

    let outputs = vec![serde_json::json!({
        "outputIndex": output_index,
        "protocol": "wallet payment",
        "paymentRemittance": {
            "derivationPrefix": refund.derivation_prefix,
            "derivationSuffix": refund.derivation_suffix,
            "senderIdentityKey": refund.sender_identity_key,
        },
    })];

    wallet
        .internalize_action(
            &tx_bytes,
            &outputs,
            &format!("Refund: {} sats", refund.satoshis),
        )
        .await
}

/// Derive the expected refund public key and find its output index in the transaction.
///
/// Uses the BSV SDK to parse the AtomicBEEF/BEEF envelope and extract transaction
/// outputs. Computes hash160 of the derived key and matches against P2PKH scripts.
async fn derive_and_find_output(
    wallet: &dyn WalletApi,
    refund: &RefundInfo,
    tx_bytes: &[u8],
) -> Option<u32> {
    // Derive the public key we expect the refund to be locked to
    let protocol_id = payment_protocol();
    let key_id = format!("{} {}", refund.derivation_prefix, refund.derivation_suffix);

    let pubkey_hex = wallet
        .get_public_key(&protocol_id, &key_id, &refund.sender_identity_key, false)
        .await
        .ok()?;

    let pubkey_bytes = hex::decode(&pubkey_hex).ok()?;
    let expected_hash160 = hash160(&pubkey_bytes);

    // Parse the transaction from the BEEF envelope
    let tx = parse_tx_from_envelope(tx_bytes)?;

    // Find the output whose P2PKH hash160 matches
    find_p2pkh_output(&tx, &expected_hash160)
}

/// Parse a Transaction from AtomicBEEF or BEEF bytes using the BSV SDK.
fn parse_tx_from_envelope(data: &[u8]) -> Option<bsv_rs::transaction::Transaction> {
    if data.len() < 4 {
        return None;
    }

    // AtomicBEEF: 01 01 01 01 + 32-byte reversed txid + BEEF
    if data[..4] == [0x01, 0x01, 0x01, 0x01] && data.len() > 36 {
        let beef_data = &data[36..];
        if let Ok(beef) = bsv_rs::transaction::Beef::from_binary(beef_data) {
            // The last tx in the BEEF is the newest (the refund tx)
            return beef.txs.last().and_then(|btx| btx.tx().cloned());
        }
    }

    // Plain BEEF: 01 00 BE EF
    if data[..4] == [0x01, 0x00, 0xBE, 0xEF] {
        if let Ok(beef) = bsv_rs::transaction::Beef::from_binary(data) {
            return beef.txs.last().and_then(|btx| btx.tx().cloned());
        }
    }

    // Raw transaction
    bsv_rs::transaction::Transaction::from_binary(data).ok()
}

/// Find the output index of a P2PKH script matching the expected hash160.
///
/// P2PKH format: OP_DUP(76) OP_HASH160(a9) PUSH20(14) <hash160> OP_EQUALVERIFY(88) OP_CHECKSIG(ac)
fn find_p2pkh_output(
    tx: &bsv_rs::transaction::Transaction,
    expected_hash160: &[u8],
) -> Option<u32> {
    for (i, output) in tx.outputs.iter().enumerate() {
        let script_bytes = output.locking_script.to_binary();
        if script_bytes.len() == 25
            && script_bytes[0] == 0x76
            && script_bytes[1] == 0xa9
            && script_bytes[2] == 0x14
            && script_bytes[23] == 0x88
            && script_bytes[24] == 0xac
            && script_bytes[3..23] == *expected_hash160
        {
            return Some(i as u32);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_refund_with_output_index() {
        let body = serde_json::json!({
            "excessRefund": {
                "transaction": "AQAAAA==",
                "derivationPrefix": "prefix",
                "derivationSuffix": "suffix",
                "senderIdentityKey": "02abcd",
                "satoshis": 5000,
                "outputIndex": 1
            }
        });
        let refund = parse_refund(&body).unwrap();
        assert_eq!(refund.output_index, Some(1));
    }

    #[test]
    fn test_parse_refund_without_output_index() {
        let body = serde_json::json!({
            "excessRefund": {
                "transaction": "AQAAAA==",
                "derivationPrefix": "prefix",
                "derivationSuffix": "suffix",
                "senderIdentityKey": "02abcd",
                "satoshis": 5000
            }
        });
        let refund = parse_refund(&body).unwrap();
        assert_eq!(refund.output_index, None);
    }

    #[test]
    fn test_find_p2pkh_output_first() {
        use bsv_rs::transaction::{Transaction, TransactionOutput};

        let hash160_bytes = [0xAA; 20];
        let mut script = vec![0x76, 0xa9, 0x14];
        script.extend_from_slice(&hash160_bytes);
        script.extend_from_slice(&[0x88, 0xac]);

        let mut tx = Transaction::new();
        let output = TransactionOutput::new(
            1000,
            bsv_rs::script::LockingScript::from_binary(&script).unwrap(),
        );
        tx.outputs.push(output);

        assert_eq!(find_p2pkh_output(&tx, &hash160_bytes), Some(0));
    }

    #[test]
    fn test_find_p2pkh_output_second() {
        use bsv_rs::transaction::{Transaction, TransactionOutput};

        let wrong_hash = [0xBB; 20];
        let right_hash = [0xCC; 20];

        let mut wrong_script = vec![0x76, 0xa9, 0x14];
        wrong_script.extend_from_slice(&wrong_hash);
        wrong_script.extend_from_slice(&[0x88, 0xac]);

        let mut right_script = vec![0x76, 0xa9, 0x14];
        right_script.extend_from_slice(&right_hash);
        right_script.extend_from_slice(&[0x88, 0xac]);

        let mut tx = Transaction::new();
        tx.outputs.push(TransactionOutput::new(
            200_000,
            bsv_rs::script::LockingScript::from_binary(&wrong_script).unwrap(),
        ));
        tx.outputs.push(TransactionOutput::new(
            100_000,
            bsv_rs::script::LockingScript::from_binary(&right_script).unwrap(),
        ));

        assert_eq!(find_p2pkh_output(&tx, &right_hash), Some(1));
        assert_eq!(find_p2pkh_output(&tx, &wrong_hash), Some(0));
    }

    #[test]
    fn test_find_p2pkh_output_no_match() {
        use bsv_rs::transaction::Transaction;
        let tx = Transaction::new();
        let hash = [0xDD; 20];
        assert_eq!(find_p2pkh_output(&tx, &hash), None);
    }

    #[test]
    fn test_parse_tx_from_raw_bytes() {
        // Minimal valid raw tx: version(4) + varint(0 inputs) + varint(1 output) + output + locktime(4)
        let hash160 = [0xEE; 20];
        let mut tx_bytes = vec![0x01, 0x00, 0x00, 0x00]; // version 1
        tx_bytes.push(0x00); // 0 inputs
        tx_bytes.push(0x01); // 1 output
        tx_bytes.extend_from_slice(&1000u64.to_le_bytes()); // satoshis
        tx_bytes.push(0x19); // script length = 25
        tx_bytes.extend_from_slice(&[0x76, 0xa9, 0x14]); // P2PKH prefix
        tx_bytes.extend_from_slice(&hash160);
        tx_bytes.extend_from_slice(&[0x88, 0xac]); // P2PKH suffix
        tx_bytes.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]); // locktime

        let tx = parse_tx_from_envelope(&tx_bytes);
        assert!(tx.is_some());
        let tx = tx.unwrap();
        assert_eq!(tx.outputs.len(), 1);
        assert_eq!(find_p2pkh_output(&tx, &hash160), Some(0));
    }
}
