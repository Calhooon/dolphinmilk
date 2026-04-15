//! Content moderation engine — cert-driven policy for filtering agent inputs and outputs.
//!
//! Separate from `sanitize.rs` (which handles injection defense / credential leak detection).
//! This module provides configurable policy-driven content filtering: PII detection, keyword
//! blocking, and regex pattern matching. The policy is resolved from BRC-52 certificate fields
//! with fallback to `ModerationConfig` from `dolphin-milk.toml`.
//!
//! Precedence (highest wins): cert fields -> config -> defaults (disabled).
//! Cert fields can only tighten policy — a parent can force-enable moderation or escalate
//! "flag" to "block", but the agent's local config cannot downgrade a cert-mandated mode.

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::certificates::CertModerationPolicy;
use crate::config::ModerationConfig;

// ---------------------------------------------------------------------------
// Mode helpers
// ---------------------------------------------------------------------------

/// Moderation mode for a detection category.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Off,
    Flag,
    Block,
}

impl Mode {
    /// Parse a mode string. Unknown values → Off.
    pub fn parse(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "block" => Mode::Block,
            "flag" => Mode::Flag,
            _ => Mode::Off,
        }
    }

    /// Return the stricter of two modes.
    /// Strictness order: Off < Flag < Block.
    pub fn stricter(self, other: Self) -> Self {
        match (self, other) {
            (Mode::Block, _) | (_, Mode::Block) => Mode::Block,
            (Mode::Flag, _) | (_, Mode::Flag) => Mode::Flag,
            _ => Mode::Off,
        }
    }
}

// ---------------------------------------------------------------------------
// Result types
// ---------------------------------------------------------------------------

/// A single match found by the moderation engine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModerationMatch {
    /// Name of the pattern that matched (e.g. "pii_ssn", "keyword_secret").
    pub pattern_name: String,
    /// The text that was matched.
    pub matched_text: String,
    /// Byte offset of the match start in the input.
    pub position: usize,
}

/// Result of moderating a piece of content.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ModerationResult {
    /// Content passed all checks.
    Pass,
    /// Content matched one or more flag-level rules. Logged, execution continues.
    Flagged(Vec<ModerationMatch>),
    /// Content matched one or more block-level rules. Logged, execution stopped.
    Blocked(Vec<ModerationMatch>),
}

impl ModerationResult {
    pub fn is_pass(&self) -> bool {
        matches!(self, ModerationResult::Pass)
    }

    pub fn is_blocked(&self) -> bool {
        matches!(self, ModerationResult::Blocked(_))
    }

    pub fn is_flagged(&self) -> bool {
        matches!(self, ModerationResult::Flagged(_))
    }
}

// ---------------------------------------------------------------------------
// Built-in PII patterns
// ---------------------------------------------------------------------------

/// SSN: 3 digits - 2 digits - 4 digits (with dashes).
const PII_SSN_PATTERN: &str = r"\b\d{3}-\d{2}-\d{4}\b";

/// Credit card: 4 groups of 4 digits, optionally separated by spaces or dashes.
const PII_CREDIT_CARD_PATTERN: &str = r"\b\d{4}[\s-]?\d{4}[\s-]?\d{4}[\s-]?\d{4}\b";

/// Email address.
const PII_EMAIL_PATTERN: &str = r"\b[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}\b";

// ---------------------------------------------------------------------------
// ModerationEngine
// ---------------------------------------------------------------------------

/// Compiled moderation engine. Build once at task start, call `moderate()` per content.
pub struct ModerationEngine {
    /// Master enable switch (resolved from cert + config).
    enabled: bool,
    /// PII detection mode (resolved from cert + config).
    pii_mode: Mode,
    /// Profanity detection mode (resolved from cert + config).
    profanity_mode: Mode,
    /// Compiled PII regexes: (pattern_name, regex).
    pii_patterns: Vec<(String, Regex)>,
    /// Compiled custom block patterns: (pattern_string, regex).
    custom_block_patterns: Vec<(String, Regex)>,
    /// Compiled custom flag patterns: (pattern_string, regex).
    custom_flag_patterns: Vec<(String, Regex)>,
    /// Lowercased keywords that block content.
    block_keywords: Vec<String>,
}

impl ModerationEngine {
    /// Construct a moderation engine from config, optionally overridden by cert policy.
    ///
    /// Cert fields tighten policy: if cert says `moderation_enabled = true`, config
    /// cannot disable. If cert says `pii_mode = "block"`, config cannot downgrade to "flag".
    pub fn new(config: &ModerationConfig, cert: Option<&CertModerationPolicy>) -> Self {
        // Resolve enabled: cert can force-enable, cannot force-disable.
        let enabled = match cert.and_then(|c| c.enabled) {
            Some(true) => true,
            _ => config.enabled,
        };

        // Resolve modes: take the stricter of cert and config.
        let config_pii = Mode::parse(&config.pii_mode);
        let cert_pii = cert
            .and_then(|c| c.pii_mode.as_deref())
            .map(Mode::parse)
            .unwrap_or(Mode::Off);
        let pii_mode = config_pii.stricter(cert_pii);

        let config_profanity = Mode::parse(&config.profanity_mode);
        let cert_profanity = cert
            .and_then(|c| c.profanity_mode.as_deref())
            .map(Mode::parse)
            .unwrap_or(Mode::Off);
        let profanity_mode = config_profanity.stricter(cert_profanity);

        // Compile PII regexes.
        let mut pii_patterns = Vec::new();
        if pii_mode != Mode::Off {
            if let Ok(re) = Regex::new(PII_SSN_PATTERN) {
                pii_patterns.push(("pii_ssn".to_string(), re));
            }
            if let Ok(re) = Regex::new(PII_CREDIT_CARD_PATTERN) {
                pii_patterns.push(("pii_credit_card".to_string(), re));
            }
            if let Ok(re) = Regex::new(PII_EMAIL_PATTERN) {
                pii_patterns.push(("pii_email".to_string(), re));
            }
        }

        // Compile custom patterns.
        let custom_block_patterns: Vec<(String, Regex)> = config
            .custom_block_patterns
            .iter()
            .filter_map(|p| Regex::new(p).ok().map(|re| (p.clone(), re)))
            .collect();
        let custom_flag_patterns: Vec<(String, Regex)> = config
            .custom_flag_patterns
            .iter()
            .filter_map(|p| Regex::new(p).ok().map(|re| (p.clone(), re)))
            .collect();

        // Lowercase keywords for case-insensitive matching.
        let block_keywords: Vec<String> = config
            .custom_block_keywords
            .iter()
            .map(|k| k.to_lowercase())
            .collect();

        Self {
            enabled,
            pii_mode,
            profanity_mode,
            pii_patterns,
            custom_block_patterns,
            custom_flag_patterns,
            block_keywords,
        }
    }

    /// Construct a disabled engine (always passes).
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            pii_mode: Mode::Off,
            profanity_mode: Mode::Off,
            pii_patterns: Vec::new(),
            custom_block_patterns: Vec::new(),
            custom_flag_patterns: Vec::new(),
            block_keywords: Vec::new(),
        }
    }

    /// Check whether moderation is enabled.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Moderate a piece of content. Returns `Pass`, `Flagged`, or `Blocked`.
    ///
    /// When disabled, always returns `Pass` (zero overhead).
    pub fn moderate(&self, content: &str) -> ModerationResult {
        if !self.enabled {
            return ModerationResult::Pass;
        }

        let mut blocked_matches = Vec::new();
        let mut flagged_matches = Vec::new();

        // --- PII patterns ---
        for (name, re) in &self.pii_patterns {
            for m in re.find_iter(content) {
                let mm = ModerationMatch {
                    pattern_name: name.clone(),
                    matched_text: m.as_str().to_string(),
                    position: m.start(),
                };
                match self.pii_mode {
                    Mode::Block => blocked_matches.push(mm),
                    Mode::Flag => flagged_matches.push(mm),
                    Mode::Off => {}
                }
            }
        }

        // --- Profanity patterns (placeholder for future profanity word lists) ---
        // The profanity_mode field is resolved and available for future expansion.
        // Currently, profanity detection is represented by the mode field only.
        let _ = self.profanity_mode;

        // --- Custom block patterns ---
        for (pat_str, re) in &self.custom_block_patterns {
            for m in re.find_iter(content) {
                blocked_matches.push(ModerationMatch {
                    pattern_name: format!("custom_block:{}", pat_str),
                    matched_text: m.as_str().to_string(),
                    position: m.start(),
                });
            }
        }

        // --- Custom flag patterns ---
        for (pat_str, re) in &self.custom_flag_patterns {
            for m in re.find_iter(content) {
                flagged_matches.push(ModerationMatch {
                    pattern_name: format!("custom_flag:{}", pat_str),
                    matched_text: m.as_str().to_string(),
                    position: m.start(),
                });
            }
        }

        // --- Keyword blocking (case-insensitive) ---
        if !self.block_keywords.is_empty() {
            let lower = content.to_lowercase();
            for kw in &self.block_keywords {
                // Find all occurrences of the keyword in the lowered content.
                let mut search_start = 0;
                while let Some(pos) = lower[search_start..].find(kw.as_str()) {
                    let abs_pos = search_start + pos;
                    let end = abs_pos + kw.len();
                    blocked_matches.push(ModerationMatch {
                        pattern_name: format!("keyword:{}", kw),
                        matched_text: content[abs_pos..end].to_string(),
                        position: abs_pos,
                    });
                    search_start = end;
                }
            }
        }

        // --- Resolve result ---
        if !blocked_matches.is_empty() {
            tracing::error!(count = blocked_matches.len(), "moderation: content BLOCKED");
            ModerationResult::Blocked(blocked_matches)
        } else if !flagged_matches.is_empty() {
            tracing::warn!(count = flagged_matches.len(), "moderation: content FLAGGED");
            ModerationResult::Flagged(flagged_matches)
        } else {
            ModerationResult::Pass
        }
    }
}
