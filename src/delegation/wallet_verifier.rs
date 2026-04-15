//! [`SignatureVerifier`] implementation for delegation cert chains.
//!
//! Phase 1 left the verifier trait injected so pure-logic tests could mock
//! signing without a wallet. Phase 3 wires it to a concrete implementation
//! so the runner can verify inbound delegation certs against on-chain keys.
//!
//! # Why a local ProtoWallet instead of the HTTP wallet?
//!
//! Phase 2's `delegate_task` signs with:
//!
//! ```text
//! wallet.create_signature(
//!     canonical_body,
//!     protocol_id  = [2, "agent delegation v1"],
//!     key_id       = "delegation",
//!     counterparty = "anyone",
//! )
//! ```
//!
//! The `counterparty="anyone"` selection means the signer's wallet derives
//! a child signing key using BRC-42 DH against the publicly known "anyone"
//! key (secp256k1 generator point G). The derived child pubkey is
//! **publicly recoverable** by any verifier who knows the signer's identity
//! pubkey, via the same BRC-42 derivation with the roles swapped:
//!
//! - Signer:   `signer_priv.derive_child(anyone_pub, invoice)` → child_priv
//! - Verifier: `signer_pub.derive_child(anyone_priv, invoice)` → child_pub
//!
//! By Diffie–Hellman symmetry these two produce the same keypair, and the
//! verifier can check the ECDSA signature against `child_pub` without ever
//! touching the signer's wallet.
//!
//! The HTTP `WalletBackend::verify_signature` is the wrong abstraction here
//! because it always derives using the **caller's** root key. Coral calling
//! `verify_signature(counterparty=<captain_pub>)` makes its wallet compute
//! `captain_pub.derive_child(coral_priv, invoice)` — a shared-secret
//! derivation between Captain and Coral, which is **not** what Captain
//! signed with. (We hit this empirically: the wallet returns
//! `"Signature is not valid"`.)
//!
//! Instead, we construct a local `bsv::wallet::ProtoWallet::anyone()` — a
//! key-less wallet whose root key is the well-known anyone key — and call
//! its `verify_signature()` with `counterparty = Other(signer_pub)` and
//! `for_self = false`. The ProtoWallet's key deriver then computes
//! `signer_pub.derive_child(anyone_priv, invoice)`, which matches the
//! signing side and recovers the correct child pubkey. The ECDSA check
//! runs against that pubkey.
//!
//! This is a pure in-process operation — no wallet RPC, no network — so it
//! is both fast and decoupled from the local wallet's identity.

use async_trait::async_trait;

use super::types::DelegationError;
use super::verify::SignatureVerifier;

use bsv::primitives::PublicKey;
use bsv::wallet::{Counterparty, ProtoWallet, Protocol, SecurityLevel, VerifySignatureArgs};

/// [`SignatureVerifier`] that recovers the signer's BRC-42 child pubkey
/// locally and performs an ECDSA check — no wallet network calls required.
///
/// Stateless. Construct once per task (or once globally and clone).
#[derive(Default, Clone)]
pub struct WalletSignatureVerifier;

impl WalletSignatureVerifier {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl SignatureVerifier for WalletSignatureVerifier {
    async fn verify_cert_signature(
        &self,
        canonical_body: &[u8],
        signature_hex: &str,
        signer: &str,
    ) -> Result<bool, DelegationError> {
        // Hex-decode the signature. Malformed hex is treated as an invalid
        // signature rather than a hard error — the delegation will just be
        // rejected.
        let sig_bytes = match hex::decode(signature_hex) {
            Ok(b) => b,
            Err(e) => {
                tracing::debug!("delegation signature hex decode failed for signer {signer}: {e}");
                return Ok(false);
            }
        };

        // Parse the signer's compressed secp256k1 pubkey.
        let signer_pub = match PublicKey::from_hex(signer) {
            Ok(pk) => pk,
            Err(e) => {
                return Err(DelegationError::Other(format!(
                    "invalid signer pubkey hex for delegation verify: {e}"
                )));
            }
        };

        // Build a ProtoWallet rooted at the "anyone" key and verify the
        // signature against `signer_pub.derive_child(anyone_priv, invoice)`.
        let wallet = ProtoWallet::anyone();
        let protocol = Protocol::new(SecurityLevel::Counterparty, "agent delegation v1");

        let args = VerifySignatureArgs {
            data: Some(canonical_body.to_vec()),
            hash_to_directly_verify: None,
            signature: sig_bytes,
            protocol_id: protocol,
            key_id: "delegation".to_string(),
            counterparty: Some(Counterparty::Other(signer_pub)),
            for_self: Some(false),
        };

        // `ProtoWallet::verify_signature` returns `Err(WalletError("Signature
        // is not valid"))` on mismatch rather than `Ok(VerifySignatureResult
        // { valid: false })`. Flatten both failure modes to `Ok(false)` so
        // the verifier trait contract (Ok(false) = bad sig, Err = infra) is
        // preserved.
        match wallet.verify_signature(args) {
            Ok(res) => Ok(res.valid),
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("Signature is not valid") {
                    tracing::debug!("delegation signature rejected for signer {signer}: {msg}");
                    Ok(false)
                } else {
                    Err(DelegationError::Other(format!(
                        "ProtoWallet::verify_signature failed for signer {signer}: {msg}"
                    )))
                }
            }
        }
    }
}
