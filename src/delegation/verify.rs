//! Delegation cert verification — the 8-step algorithm from `DELEGATION-DESIGN.md` §4.
//!
//! This module is pure logic. External dependencies (wallet signature verification,
//! on-chain UTXO status, current time) are injected via traits so the module can
//! be unit-tested in complete isolation.

use super::canonicalize::{canonicalize_cert, compute_cert_hash, compute_purpose_hash};
use super::narrowing::check_chain_narrowing;
use super::types::{
    DelegationCert, DelegationError, VerifiedDelegation, CLOCK_SKEW_TOLERANCE_SECS,
    DELEGATION_VERSION_V1,
};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use std::collections::{BTreeMap, BTreeSet, HashSet};

// ── Injected dependencies ─────────────────────────────────────────

/// Verifies BRC-77 signatures. In production this wraps a `WalletBackend`.
/// In tests it's a mock.
#[async_trait]
pub trait SignatureVerifier: Send + Sync {
    /// Verify a BRC-77 ECDSA signature over `canonical_body` by the key `signer`.
    ///
    /// Returns `Ok(true)` on valid signature, `Ok(false)` on mismatch,
    /// `Err(...)` on wallet/network failure.
    async fn verify_cert_signature(
        &self,
        canonical_body: &[u8],
        signature_hex: &str,
        signer: &str,
    ) -> Result<bool, DelegationError>;
}

/// Checks whether an on-chain outpoint has been spent. Used for revocation.
#[async_trait]
pub trait RevocationChecker: Send + Sync {
    /// Returns `Ok(true)` if the outpoint has been spent (cert revoked),
    /// `Ok(false)` if still unspent (cert valid).
    ///
    /// Null/empty/self-signed revocation outpoints always return `Ok(false)`
    /// (no revocation mechanism configured for this cert).
    async fn is_outpoint_spent(&self, outpoint: &str) -> Result<bool, DelegationError>;
}

/// Clock abstraction so tests can control "now".
pub trait Clock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

/// Default wall-clock implementation.
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

// ── Verification entry point ──────────────────────────────────────

/// Inputs to [`verify_delegation_chain`]. Bundled into a struct so the function
/// signature stays under clippy's `too_many_arguments` limit and stays clear.
pub struct VerifyInputs<'a> {
    /// The cert chain, oldest-first (root at index 0). The leaf is the cert
    /// that directly applies to the current task.
    pub chain: &'a [DelegationCert],
    /// The task description this delegation is for. Used to compute
    /// the expected purpose_hash.
    pub task_description: &'a str,
    /// My own identity key (the subject of the leaf cert).
    pub my_identity_key: &'a str,
    /// The set of trusted root certifier identity keys (from `config.trust.certifiers`).
    pub trusted_certifiers: &'a HashSet<String>,
}

/// Verify a delegation cert chain and return a `VerifiedDelegation` with
/// effective (narrowed-at-every-hop) caveats.
///
/// Implements the 8-step algorithm from `DELEGATION-DESIGN.md` §4:
///
/// 1. Structural sanity (version, subject, required fields).
/// 2. Signature verification for every cert in the chain.
/// 3. Root trust (leaf's declared root must be in `trusted_certifiers`).
/// 4. Time (not expired, not too-future-dated).
/// 5. Purpose binding (leaf purpose_hash matches task description).
/// 6. Chain walk + narrowing enforcement (via [`check_chain_narrowing`]).
/// 7. Revocation (every cert in the chain).
/// 8. Effective caveat computation (intersection of all chain entries).
pub async fn verify_delegation_chain(
    inputs: VerifyInputs<'_>,
    signer: &dyn SignatureVerifier,
    revocation: &dyn RevocationChecker,
    clock: &dyn Clock,
) -> Result<VerifiedDelegation, DelegationError> {
    let VerifyInputs {
        chain,
        task_description,
        my_identity_key,
        trusted_certifiers,
    } = inputs;

    if chain.is_empty() {
        return Err(DelegationError::Other("empty delegation chain".into()));
    }
    let leaf = chain.last().expect("chain non-empty checked above");

    // Step 1 — Structural sanity
    for cert in chain {
        if cert.version != DELEGATION_VERSION_V1 {
            return Err(DelegationError::UnsupportedVersion {
                expected: DELEGATION_VERSION_V1.into(),
                got: cert.version.clone(),
            });
        }
    }
    if leaf.subject != my_identity_key {
        return Err(DelegationError::WrongSubject {
            expected: my_identity_key.into(),
            got: leaf.subject.clone(),
        });
    }

    // Step 2 — Signature verification for every cert in the chain
    for cert in chain {
        let wire = cert.to_value();
        let canonical = canonicalize_cert(&wire)?;
        let sig_ok = signer
            .verify_cert_signature(&canonical, &cert.signature, &cert.certifier)
            .await?;
        if !sig_ok {
            return Err(DelegationError::InvalidSignature);
        }
    }

    // Step 3 — Root trust
    if !trusted_certifiers.contains(&leaf.root_certifier) {
        return Err(DelegationError::UnknownRootCertifier(
            leaf.root_certifier.clone(),
        ));
    }

    // Step 4 — Time
    let now = clock.now();
    for cert in chain {
        if now >= cert.expires_at {
            return Err(DelegationError::Expired {
                expired_at: cert.expires_at,
            });
        }
        let earliest_valid = cert.issued_at - chrono::Duration::seconds(CLOCK_SKEW_TOLERANCE_SECS);
        if now < earliest_valid {
            return Err(DelegationError::NotYetValid {
                issued_at: cert.issued_at,
            });
        }
    }

    // Step 5 — Purpose binding
    let expected_purpose = compute_purpose_hash(task_description);
    if leaf.purpose_hash != expected_purpose {
        return Err(DelegationError::PurposeMismatch {
            cert: leaf.purpose_hash.clone(),
            task: expected_purpose,
        });
    }

    // Step 6 — Chain parent-hash linkage + narrowing
    //
    // Each cert at index i > 0 must have parent_cert_hash == hash(chain[i-1]).
    for i in 1..chain.len() {
        let parent = &chain[i - 1];
        let child = &chain[i];
        let parent_hash = compute_cert_hash(&parent.to_value())?;
        match &child.parent_cert_hash {
            None => return Err(DelegationError::ParentCertMissing),
            Some(claimed) if *claimed != parent_hash => {
                return Err(DelegationError::ParentHashMismatch);
            }
            Some(_) => {}
        }
    }
    // The root (index 0) must either have no parent_cert_hash or it must be absent.
    if chain[0].parent_cert_hash.is_some() {
        return Err(DelegationError::Malformed(
            "root cert (chain[0]) has parent_cert_hash set".into(),
        ));
    }
    // Narrowing enforcement (hop-by-hop) + depth check.
    let chain_depth = check_chain_narrowing(chain)?;

    // Step 7 — Revocation (every cert in the chain)
    for cert in chain {
        if !is_null_outpoint(&cert.revocation_outpoint) {
            let spent = revocation
                .is_outpoint_spent(&cert.revocation_outpoint)
                .await?;
            if spent {
                return Err(DelegationError::Revoked {
                    outpoint: cert.revocation_outpoint.clone(),
                });
            }
        }
    }

    // Step 8 — Effective caveat computation
    //
    // Capabilities: intersection of all chain entries.
    // Budget: minimum.
    // Expiry: earliest.
    // Capability args: intersection per-tool; if ANY cert in chain scopes a tool,
    //                  the effective scoping is the intersection.
    let effective_capabilities = intersect_capabilities(chain);
    let effective_capability_args = intersect_capability_args(chain);
    let effective_budget_cap_sats = chain.iter().map(|c| c.budget_cap_sats).min().unwrap();
    let effective_expires_at = chain.iter().map(|c| c.expires_at).min().unwrap();

    Ok(VerifiedDelegation {
        cert: leaf.clone(),
        effective_capabilities,
        effective_capability_args,
        effective_budget_cap_sats,
        effective_expires_at,
        chain_depth,
        root_certifier: leaf.root_certifier.clone(),
    })
}

// ── Helpers ───────────────────────────────────────────────────────

fn is_null_outpoint(outpoint: &str) -> bool {
    outpoint.is_empty()
        || outpoint.chars().all(|c| c == '0')
        || outpoint == "000000000000000000000000000000000000000000000000000000000000000000000000"
}

fn intersect_capabilities(chain: &[DelegationCert]) -> Vec<String> {
    if chain.is_empty() {
        return Vec::new();
    }
    let mut acc: BTreeSet<String> = chain[0].capabilities.iter().cloned().collect();
    for cert in &chain[1..] {
        let next: BTreeSet<String> = cert.capabilities.iter().cloned().collect();
        acc = acc.intersection(&next).cloned().collect();
    }
    acc.into_iter().collect()
}

fn intersect_capability_args(chain: &[DelegationCert]) -> Option<BTreeMap<String, Vec<String>>> {
    // Collect all tools that appear in ANY cert's capability_args.
    let mut all_scoped_tools: BTreeSet<String> = BTreeSet::new();
    for cert in chain {
        if let Some(ref args) = cert.capability_args {
            for tool in args.keys() {
                all_scoped_tools.insert(tool.clone());
            }
        }
    }
    if all_scoped_tools.is_empty() {
        return None;
    }

    let mut effective: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for tool in &all_scoped_tools {
        // For each cert in the chain, gather the allowed-args set for this tool.
        // If a cert does NOT mention this tool in its capability_args, it imposes
        // no restriction (universe = "all args"), so we skip it for intersection.
        let mut tool_args: Option<BTreeSet<String>> = None;
        for cert in chain {
            if let Some(ref args_map) = cert.capability_args {
                if let Some(args) = args_map.get(tool) {
                    let set: BTreeSet<String> = args.iter().cloned().collect();
                    tool_args = Some(match tool_args {
                        None => set,
                        Some(prev) => prev.intersection(&set).cloned().collect(),
                    });
                }
            }
        }
        if let Some(set) = tool_args {
            effective.insert(tool.clone(), set.into_iter().collect());
        }
    }

    if effective.is_empty() {
        None
    } else {
        Some(effective)
    }
}

/// Apply a verified delegation's caveats to a set of available tools and a base budget.
/// Returns the sandboxed `(allowed_tools, effective_task_budget)`.
pub fn apply_caveats(
    available_tools: &[String],
    base_task_budget_sats: u64,
    verified: &VerifiedDelegation,
) -> (Vec<String>, u64) {
    let cert_caps: BTreeSet<&str> = verified
        .effective_capabilities
        .iter()
        .map(|s| s.as_str())
        .collect();
    let allowed: Vec<String> = available_tools
        .iter()
        .filter(|t| cert_caps.contains(t.as_str()))
        .cloned()
        .collect();
    let effective_budget = base_task_budget_sats.min(verified.effective_budget_cap_sats);
    (allowed, effective_budget)
}

// ── Unit tests ────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;
    use std::sync::{Arc, Mutex};

    // ── Mock dependencies ─────────────────────────────────────────

    #[derive(Clone, Default)]
    struct MockSigner {
        // If set, always return this result regardless of inputs.
        force_result: Arc<Mutex<Option<bool>>>,
    }

    impl MockSigner {
        fn always_ok() -> Self {
            let m = Self::default();
            *m.force_result.lock().unwrap() = Some(true);
            m
        }
        fn always_fail() -> Self {
            let m = Self::default();
            *m.force_result.lock().unwrap() = Some(false);
            m
        }
    }

    #[async_trait]
    impl SignatureVerifier for MockSigner {
        async fn verify_cert_signature(
            &self,
            _canonical_body: &[u8],
            _signature_hex: &str,
            _signer: &str,
        ) -> Result<bool, DelegationError> {
            Ok(self.force_result.lock().unwrap().unwrap_or(true))
        }
    }

    #[derive(Clone, Default)]
    struct MockRevocation {
        spent_outpoints: Arc<Mutex<HashSet<String>>>,
    }

    impl MockRevocation {
        fn none_spent() -> Self {
            Self::default()
        }
        fn mark_spent(&self, outpoint: &str) {
            self.spent_outpoints
                .lock()
                .unwrap()
                .insert(outpoint.to_string());
        }
    }

    #[async_trait]
    impl RevocationChecker for MockRevocation {
        async fn is_outpoint_spent(&self, outpoint: &str) -> Result<bool, DelegationError> {
            Ok(self.spent_outpoints.lock().unwrap().contains(outpoint))
        }
    }

    /// A clock that returns a fixed time. Used by chain tests that need
    /// deterministic `now()` (expiry ordering depends on it).
    #[allow(dead_code)]
    struct FixedClock(DateTime<Utc>);
    #[allow(dead_code)]
    impl Clock for FixedClock {
        fn now(&self) -> DateTime<Utc> {
            self.0
        }
    }

    // ── Test fixtures ─────────────────────────────────────────────

    const SUBJECT: &str = "02dead0000000000000000000000000000000000000000000000000000000000aa";
    const ISSUER: &str = "03beef0000000000000000000000000000000000000000000000000000000000bb";
    const ROOT: &str = "03aaaa0000000000000000000000000000000000000000000000000000000000cc";
    const TASK: &str = "fetch reddit top 10";

    fn trusted_set() -> HashSet<String> {
        let mut s = HashSet::new();
        s.insert(ROOT.to_string());
        s
    }

    fn good_cert() -> DelegationCert {
        DelegationCert {
            subject: SUBJECT.into(),
            certifier: ISSUER.into(),
            serial_number: "delegation-test-1".into(),
            revocation_outpoint: "aa".repeat(36),
            signature: "deadbeef".into(),
            version: DELEGATION_VERSION_V1.into(),
            capabilities: vec!["web_fetch".into(), "memory_store".into()],
            capability_args: None,
            budget_cap_sats: 50_000,
            expires_at: Utc::now() + Duration::minutes(30),
            purpose_hash: compute_purpose_hash(TASK),
            payment: None,
            parent_cert_hash: None,
            root_certifier: ROOT.into(),
            issued_at: Utc::now() - Duration::seconds(10),
        }
    }

    async fn verify_one(cert: DelegationCert) -> Result<VerifiedDelegation, DelegationError> {
        let chain = vec![cert];
        let trusted = trusted_set();
        let inputs = VerifyInputs {
            chain: &chain,
            task_description: TASK,
            my_identity_key: SUBJECT,
            trusted_certifiers: &trusted,
        };
        verify_delegation_chain(
            inputs,
            &MockSigner::always_ok(),
            &MockRevocation::none_spent(),
            &SystemClock,
        )
        .await
    }

    // ── Tests ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn happy_path_single_hop() {
        let v = verify_one(good_cert()).await.unwrap();
        assert_eq!(v.effective_capabilities.len(), 2);
        assert_eq!(v.effective_budget_cap_sats, 50_000);
        assert_eq!(v.chain_depth, 1);
        assert_eq!(v.root_certifier, ROOT);
    }

    #[tokio::test]
    async fn rejects_wrong_version() {
        let mut c = good_cert();
        c.version = "0".into();
        let err = verify_one(c).await.unwrap_err();
        assert!(matches!(err, DelegationError::UnsupportedVersion { .. }));
    }

    #[tokio::test]
    async fn rejects_wrong_subject() {
        let mut c = good_cert();
        c.subject = "02".repeat(33);
        let err = verify_one(c).await.unwrap_err();
        assert!(matches!(err, DelegationError::WrongSubject { .. }));
    }

    #[tokio::test]
    async fn rejects_bad_signature() {
        let chain = vec![good_cert()];
        let trusted = trusted_set();
        let inputs = VerifyInputs {
            chain: &chain,
            task_description: TASK,
            my_identity_key: SUBJECT,
            trusted_certifiers: &trusted,
        };
        let err = verify_delegation_chain(
            inputs,
            &MockSigner::always_fail(),
            &MockRevocation::none_spent(),
            &SystemClock,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, DelegationError::InvalidSignature));
    }

    #[tokio::test]
    async fn rejects_unknown_root_certifier() {
        let mut c = good_cert();
        c.root_certifier = "03".repeat(33);
        let err = verify_one(c).await.unwrap_err();
        assert!(matches!(err, DelegationError::UnknownRootCertifier(_)));
    }

    #[tokio::test]
    async fn rejects_expired() {
        let mut c = good_cert();
        c.expires_at = Utc::now() - Duration::seconds(1);
        let err = verify_one(c).await.unwrap_err();
        assert!(matches!(err, DelegationError::Expired { .. }));
    }

    #[tokio::test]
    async fn rejects_not_yet_valid() {
        let mut c = good_cert();
        // Issued 5 minutes in the future — way outside 60s clock skew tolerance
        c.issued_at = Utc::now() + Duration::minutes(5);
        let err = verify_one(c).await.unwrap_err();
        assert!(matches!(err, DelegationError::NotYetValid { .. }));
    }

    #[tokio::test]
    async fn accepts_within_clock_skew() {
        // Issued 30s in the future — within 60s tolerance
        let mut c = good_cert();
        c.issued_at = Utc::now() + Duration::seconds(30);
        assert!(verify_one(c).await.is_ok());
    }

    #[tokio::test]
    async fn rejects_purpose_mismatch() {
        let mut c = good_cert();
        c.purpose_hash = compute_purpose_hash("some other task");
        let err = verify_one(c).await.unwrap_err();
        assert!(matches!(err, DelegationError::PurposeMismatch { .. }));
    }

    #[tokio::test]
    async fn rejects_revoked() {
        let cert = good_cert();
        let outpoint = cert.revocation_outpoint.clone();
        let chain = vec![cert];
        let trusted = trusted_set();
        let rev = MockRevocation::none_spent();
        rev.mark_spent(&outpoint);
        let inputs = VerifyInputs {
            chain: &chain,
            task_description: TASK,
            my_identity_key: SUBJECT,
            trusted_certifiers: &trusted,
        };
        let err = verify_delegation_chain(inputs, &MockSigner::always_ok(), &rev, &SystemClock)
            .await
            .unwrap_err();
        assert!(matches!(err, DelegationError::Revoked { .. }));
    }

    #[tokio::test]
    async fn null_revocation_outpoint_not_checked() {
        let mut c = good_cert();
        c.revocation_outpoint = "0".repeat(72);
        // Revocation check would say "not spent" for any outpoint, but
        // null outpoints are skipped entirely — this just verifies the skip path.
        assert!(verify_one(c).await.is_ok());
    }

    #[tokio::test]
    async fn two_hop_chain_valid() {
        let root = good_cert();
        let root_hash = compute_cert_hash(&root.to_value()).unwrap();
        let mut child = good_cert();
        child.capabilities = vec!["web_fetch".into()]; // narrowed
        child.budget_cap_sats = 25_000; // narrowed
        child.expires_at = root.expires_at - Duration::minutes(1); // narrowed
        child.parent_cert_hash = Some(root_hash);
        // child.certifier = issuer of this hop (the agent that re-delegated)

        let chain = vec![root, child];
        let trusted = trusted_set();
        let inputs = VerifyInputs {
            chain: &chain,
            task_description: TASK,
            my_identity_key: SUBJECT,
            trusted_certifiers: &trusted,
        };
        let v = verify_delegation_chain(
            inputs,
            &MockSigner::always_ok(),
            &MockRevocation::none_spent(),
            &SystemClock,
        )
        .await
        .unwrap();
        assert_eq!(v.chain_depth, 2);
        assert_eq!(v.effective_capabilities, vec!["web_fetch".to_string()]);
        assert_eq!(v.effective_budget_cap_sats, 25_000);
    }

    #[tokio::test]
    async fn chain_with_wrong_parent_hash_rejected() {
        let root = good_cert();
        let mut child = good_cert();
        child.capabilities = vec!["web_fetch".into()];
        child.parent_cert_hash = Some("sha256:deadbeef".into());
        let chain = vec![root, child];
        let trusted = trusted_set();
        let inputs = VerifyInputs {
            chain: &chain,
            task_description: TASK,
            my_identity_key: SUBJECT,
            trusted_certifiers: &trusted,
        };
        let err = verify_delegation_chain(
            inputs,
            &MockSigner::always_ok(),
            &MockRevocation::none_spent(),
            &SystemClock,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, DelegationError::ParentHashMismatch));
    }

    #[tokio::test]
    async fn chain_with_missing_parent_hash_rejected() {
        let root = good_cert();
        let mut child = good_cert();
        child.capabilities = vec!["web_fetch".into()];
        // parent_cert_hash left None — should fail since chain has 2 hops
        let chain = vec![root, child];
        let trusted = trusted_set();
        let inputs = VerifyInputs {
            chain: &chain,
            task_description: TASK,
            my_identity_key: SUBJECT,
            trusted_certifiers: &trusted,
        };
        let err = verify_delegation_chain(
            inputs,
            &MockSigner::always_ok(),
            &MockRevocation::none_spent(),
            &SystemClock,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, DelegationError::ParentCertMissing));
    }

    #[tokio::test]
    async fn chain_root_with_parent_hash_rejected() {
        let mut root = good_cert();
        root.parent_cert_hash = Some("sha256:deadbeef".into());
        let chain = vec![root];
        let trusted = trusted_set();
        let inputs = VerifyInputs {
            chain: &chain,
            task_description: TASK,
            my_identity_key: SUBJECT,
            trusted_certifiers: &trusted,
        };
        let err = verify_delegation_chain(
            inputs,
            &MockSigner::always_ok(),
            &MockRevocation::none_spent(),
            &SystemClock,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, DelegationError::Malformed(_)));
    }

    #[tokio::test]
    async fn apply_caveats_intersects_tools_and_caps_budget() {
        let v = verify_one(good_cert()).await.unwrap();
        let available = vec![
            "web_fetch".to_string(),
            "memory_store".to_string(),
            "wallet_call".to_string(), // not in cert
        ];
        let (allowed, budget) = apply_caveats(&available, 100_000, &v);
        assert_eq!(allowed.len(), 2);
        assert!(allowed.contains(&"web_fetch".to_string()));
        assert!(allowed.contains(&"memory_store".to_string()));
        assert!(!allowed.contains(&"wallet_call".to_string()));
        // Base budget 100k > cert budget 50k → capped at 50k
        assert_eq!(budget, 50_000);
    }

    #[tokio::test]
    async fn apply_caveats_budget_takes_min() {
        let v = verify_one(good_cert()).await.unwrap();
        // Base budget 10k < cert budget 50k → capped at 10k
        let (_allowed, budget) = apply_caveats(&["web_fetch".to_string()], 10_000, &v);
        assert_eq!(budget, 10_000);
    }

    #[test]
    fn null_outpoint_detection() {
        assert!(is_null_outpoint(""));
        assert!(is_null_outpoint("0".repeat(72).as_str()));
        assert!(is_null_outpoint("0".repeat(64).as_str()));
        assert!(!is_null_outpoint(&"aa".repeat(36)));
    }

    #[test]
    fn intersect_capabilities_preserves_common() {
        let mut c1 = good_cert();
        c1.capabilities = vec!["a".into(), "b".into(), "c".into()];
        let mut c2 = good_cert();
        c2.capabilities = vec!["b".into(), "c".into(), "d".into()];
        let mut c3 = good_cert();
        c3.capabilities = vec!["b".into(), "c".into()];
        let result = intersect_capabilities(&[c1, c2, c3]);
        assert_eq!(result, vec!["b".to_string(), "c".to_string()]);
    }
}
