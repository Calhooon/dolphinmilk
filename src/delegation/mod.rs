//! Scoped cross-agent delegation via caveat-extended BRC-52 certificates.
//!
//! See [`docs/DELEGATION-DESIGN.md`](../../docs/DELEGATION-DESIGN.md) for the full spec.
//!
//! # Module layout
//!
//! - [`types`] — `DelegationCert`, `VerifiedDelegation`, `DelegationError`, `DelegationContext`
//! - [`canonicalize`] — deterministic JSON serialization for sign/verify
//! - [`narrowing`] — multi-hop chain narrowing rules
//! - [`verify`] — 8-step verification algorithm with injected dependencies
//! - [`cache`] — in-memory verifier cache with positive/negative TTLs
//! - [`revocation`] — on-chain UTXO status checking (test doubles for Phase 1;
//!   wallet-backed impl lands in Phase 3)
//!
//! # Usage sketch (Phase 3 integration, not yet wired in)
//!
//! ```ignore
//! use dolphin_milk::delegation::{
//!     verify::{verify_delegation_chain, VerifyInputs, SystemClock},
//!     types::DelegationCert,
//! };
//!
//! let chain: Vec<DelegationCert> = parse_message_chain(&msg_body)?;
//! let trusted = config.trust.certifiers.iter().cloned().collect();
//! let inputs = VerifyInputs {
//!     chain: &chain,
//!     task_description: &task,
//!     my_identity_key: &my_key,
//!     trusted_certifiers: &trusted,
//! };
//! let verified = verify_delegation_chain(inputs, &signer, &revoker, &SystemClock).await?;
//! // verified.effective_capabilities, verified.effective_budget_cap_sats, etc.
//! ```

pub mod cache;
pub mod canonicalize;
pub mod narrowing;
pub mod revocation;
pub mod types;
pub mod verify;
pub mod wallet_verifier;

// ── Re-exports for convenience ────────────────────────────────────

pub use cache::{CachedVerdict, VerifierCache};
pub use canonicalize::{canonicalize_cert, compute_cert_hash, compute_purpose_hash};
pub use narrowing::{check_chain_narrowing, check_narrowing};
pub use revocation::{
    parse_outpoint, NullRevocationChecker, OverlayRevocationChecker, StaticRevocationChecker,
};
pub use types::{
    delegation_protocol_id, DelegationCert, DelegationContext, DelegationError, PaymentTerms,
    VerifiedDelegation, CERT_TYPE_DELEGATION, CLOCK_SKEW_TOLERANCE_SECS, DELEGATION_VERSION_V1,
    MAX_CHAIN_DEPTH,
};
pub use verify::{
    apply_caveats, verify_delegation_chain, Clock, RevocationChecker, SignatureVerifier,
    SystemClock, VerifyInputs,
};
pub use wallet_verifier::WalletSignatureVerifier;
