//! Certificate-driven policy enforcement — budget limits, rate limits,
//! moderation policy, tool approval, and tag policy readers.

use std::sync::Arc;

use super::lifecycle::CertificateManager;
use super::types::*;
use crate::wallet::WalletBackend;

/// Read budget limits from the current agent-authorization certificate.
///
/// Returns all `None` if no certificate exists or if budget fields are absent/unparseable.
pub async fn read_cert_budget_limits(wallet: Arc<dyn WalletBackend>) -> CertBudgetLimits {
    let mgr = CertificateManager::new(wallet);
    let certs = match mgr.list_all().await {
        Ok(c) => c,
        Err(_) => return CertBudgetLimits::none(),
    };
    for entry in &certs {
        let cert = entry.get("certificate").unwrap_or(entry);
        let cert_type = cert
            .get("certificateType")
            .or_else(|| cert.get("type"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if cert_type != CERT_TYPE_AGENT_AUTH {
            continue;
        }
        let fields = cert.get("fields");
        let parse_field = |name: &str| -> Option<u64> {
            fields
                .and_then(|f| f.get(name))
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<u64>().ok())
        };
        let enforcement = fields
            .and_then(|f| f.get("budget_enforcement"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        return CertBudgetLimits {
            per_task: parse_field("budget_per_task"),
            per_hour: parse_field("budget_per_hour"),
            per_day: parse_field("budget_per_day"),
            per_week: parse_field("budget_per_week"),
            per_month: parse_field("budget_per_month"),
            lifetime: parse_field("budget_lifetime"),
            enforcement,
        };
    }
    CertBudgetLimits::none()
}

/// Read rate limit overrides from the current agent-authorization certificate.
///
/// Returns all `None` if no certificate exists or if rate limit fields are absent/unparseable.
/// Follows the same pattern as `read_cert_budget_limits()`.
pub async fn read_cert_rate_limits(
    wallet: Arc<dyn WalletBackend>,
) -> crate::x402::rate_limit::CertRateLimits {
    use crate::x402::rate_limit::CertRateLimits;

    let mgr = CertificateManager::new(wallet);
    let certs = match mgr.list_all().await {
        Ok(c) => c,
        Err(_) => return CertRateLimits::none(),
    };
    for entry in &certs {
        let cert = entry.get("certificate").unwrap_or(entry);
        let cert_type = cert
            .get("certificateType")
            .or_else(|| cert.get("type"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if cert_type != CERT_TYPE_AGENT_AUTH {
            continue;
        }
        let fields = cert.get("fields");
        let default_rpm = fields
            .and_then(|f| f.get("rate_limit_rpm_default"))
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<u32>().ok());
        let enabled = fields
            .and_then(|f| f.get("rate_limit_enabled"))
            .and_then(|v| v.as_str())
            .map(|s| s.eq_ignore_ascii_case("true"));
        return CertRateLimits {
            default_rpm,
            enabled,
        };
    }
    CertRateLimits::none()
}

/// Read moderation policy from the current agent-authorization certificate.
///
/// Returns all `None` if no certificate exists or if moderation fields are absent.
/// Follows the same pattern as `read_cert_budget_limits()`.
pub async fn read_cert_moderation_policy(wallet: Arc<dyn WalletBackend>) -> CertModerationPolicy {
    let mgr = CertificateManager::new(wallet);
    let certs = match mgr.list_all().await {
        Ok(c) => c,
        Err(_) => return CertModerationPolicy::none(),
    };
    for entry in &certs {
        let cert = entry.get("certificate").unwrap_or(entry);
        let cert_type = cert
            .get("certificateType")
            .or_else(|| cert.get("type"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if cert_type != CERT_TYPE_AGENT_AUTH {
            continue;
        }
        let fields = cert.get("fields");
        let enabled = fields
            .and_then(|f| f.get("moderation_enabled"))
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<bool>().ok());
        let pii_mode = fields
            .and_then(|f| f.get("moderation_pii"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let profanity_mode = fields
            .and_then(|f| f.get("moderation_profanity"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        return CertModerationPolicy {
            enabled,
            pii_mode,
            profanity_mode,
        };
    }
    CertModerationPolicy::none()
}

/// Read tool approval requirements from the current agent-authorization certificate.
///
/// Returns `tools: None` if no certificate exists or if the `approval_tools` field is absent.
pub async fn read_cert_tool_approval(wallet: Arc<dyn WalletBackend>) -> CertToolApproval {
    let mgr = CertificateManager::new(wallet);
    let certs = match mgr.list_all().await {
        Ok(c) => c,
        Err(_) => return CertToolApproval::none(),
    };
    for entry in &certs {
        let cert = entry.get("certificate").unwrap_or(entry);
        let cert_type = cert
            .get("certificateType")
            .or_else(|| cert.get("type"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if cert_type != CERT_TYPE_AGENT_AUTH {
            continue;
        }
        let fields = cert.get("fields");
        let tools = fields
            .and_then(|f| f.get("approval_tools"))
            .and_then(|v| v.as_str())
            .map(|s| {
                s.split(',')
                    .map(|t| t.trim().to_string())
                    .filter(|t| !t.is_empty())
                    .collect::<Vec<String>>()
            });
        return CertToolApproval { tools };
    }
    CertToolApproval::none()
}

/// Read tag policy from the current agent-authorization certificate.
///
/// Returns `tags_required: None` if no certificate exists or if the field is absent.
pub async fn read_cert_tag_policy(wallet: Arc<dyn WalletBackend>) -> CertTagPolicy {
    let mgr = CertificateManager::new(wallet);
    let certs = match mgr.list_all().await {
        Ok(c) => c,
        Err(_) => return CertTagPolicy::none(),
    };
    for entry in &certs {
        let cert = entry.get("certificate").unwrap_or(entry);
        let cert_type = cert
            .get("certificateType")
            .or_else(|| cert.get("type"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if cert_type != CERT_TYPE_AGENT_AUTH {
            continue;
        }
        let fields = cert.get("fields");
        let tags_required = fields
            .and_then(|f| f.get("tags_required"))
            .and_then(|v| v.as_str())
            .map(|s| s.eq_ignore_ascii_case("true"));
        return CertTagPolicy { tags_required };
    }
    CertTagPolicy::none()
}
