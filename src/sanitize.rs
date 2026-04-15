//! Content sanitization for external messages — injection defense.
//!
//! External messages arrive via `POST /message` from other agents and are
//! injected into the LLM conversation. Without sanitization, an attacker
//! could embed prompt injection instructions in the message body.
//!
//! This module provides:
//!   - `sanitize_external_content()` — strip control chars, collapse newlines, truncate
//!   - `wrap_with_boundary()` — random-ID boundary markers that prevent injection escape
//!   - `EXTERNAL_TOOL_ALLOWLIST` — restricted tool set for external-origin tasks

use aho_corasick::AhoCorasick;
use rand::Rng;
use std::collections::HashSet;
use std::sync::LazyLock;

/// Zero-width and invisible Unicode characters used to bypass pattern detection.
const ZERO_WIDTH_CHARS: &[char] = &[
    '\u{200B}', // Zero Width Space
    '\u{200C}', // Zero Width Non-Joiner
    '\u{200D}', // Zero Width Joiner
    '\u{FEFF}', // Zero Width No-Break Space (BOM)
    '\u{2060}', // Word Joiner
    '\u{00AD}', // Soft Hyphen
];

/// Multi-word patterns that indicate prompt injection attempts.
///
/// These are intentionally multi-word phrases to avoid false positives on
/// common words like "ignore" or "system" in isolation.
pub const INJECTION_PATTERNS: &[&str] = &[
    "ignore all previous instructions",
    "ignore the above instructions",
    "ignore your instructions",
    "disregard your instructions",
    "disregard all previous",
    "forget everything above",
    "forget your instructions",
    "you are now",
    "new instructions:",
    "override instructions:",
    "system prompt:",
    "[system]",
    "[inst]",
    "<!-- inject",
    "<|im_start|>",
    "<|im_end|>",
    "\\n\\nhuman:",
    "\\n\\nassistant:",
];

/// Compiled Aho-Corasick automaton for O(n) multi-pattern matching.
/// Case-insensitive matching for all patterns.
static INJECTION_AC: LazyLock<AhoCorasick> = LazyLock::new(|| {
    aho_corasick::AhoCorasickBuilder::new()
        .ascii_case_insensitive(true)
        .build(INJECTION_PATTERNS)
        .expect("failed to build Aho-Corasick automaton")
});

/// A detected injection pattern match.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InjectionMatch {
    /// The pattern that matched.
    pub pattern: String,
    /// Byte offset in the (stripped) input where the match starts.
    pub offset: usize,
}

/// Strip zero-width and invisible Unicode characters from input.
///
/// These characters are used to bypass pattern matching by inserting
/// invisible characters between pattern words (e.g., "ignore\u{200B}all").
pub fn strip_zero_width(input: &str) -> String {
    input
        .chars()
        .filter(|c| !ZERO_WIDTH_CHARS.contains(c))
        .collect()
}

/// Detect injection patterns in the given text using Aho-Corasick.
///
/// Zero-width characters are stripped before matching to prevent bypass.
/// Returns a list of all matches found.
pub fn detect_injections(input: &str) -> Vec<InjectionMatch> {
    let stripped = strip_zero_width(input);
    INJECTION_AC
        .find_iter(&stripped)
        .map(|mat| InjectionMatch {
            pattern: INJECTION_PATTERNS[mat.pattern().as_usize()].to_string(),
            offset: mat.start(),
        })
        .collect()
}

/// Maximum length for external message content (characters).
pub const MAX_EXTERNAL_CONTENT_LEN: usize = 2000;

/// Truncation suffix appended when content exceeds the limit.
const TRUNCATION_SUFFIX: &str = "...[truncated]";

/// Tools allowed when processing external-origin tasks.
/// These are safe tools that don't allow arbitrary code execution,
/// file system access, or spending beyond discovery.
pub const EXTERNAL_TOOL_ALLOWLIST: &[&str] = &[
    "memory_search",
    "memory_store",
    "check_inbox",
    "send_message",
    "list_conversations",
    "read_conversation",
    "search_tools",
    "continue_task",
    "discover_services",
    "discover_endpoints",
];

/// Returns the external tool allowlist as a `HashSet<String>`.
pub fn external_tool_allowlist() -> HashSet<String> {
    EXTERNAL_TOOL_ALLOWLIST
        .iter()
        .map(|s| s.to_string())
        .collect()
}

/// Sanitize external message content for safe injection into the LLM context.
///
/// 1. Strip null bytes
/// 2. Strip ASCII control characters (0x00-0x1F) except newline and tab
/// 3. Collapse runs of 3+ newlines to 2
/// 4. Truncate to `MAX_EXTERNAL_CONTENT_LEN` with a suffix
/// 5. Detect injection patterns (warning-only, does not reject)
pub fn sanitize_external_content(body: &str) -> String {
    // Step 1+2: Strip null bytes and control characters except \n (0x0A) and \t (0x09)
    let cleaned: String = body
        .chars()
        .filter(|c| {
            if *c == '\n' || *c == '\t' {
                true
            } else {
                // Keep anything that is NOT an ASCII control character (0x00-0x1F)
                !c.is_ascii_control()
            }
        })
        .collect();

    // Step 3: Collapse runs of 3+ newlines to 2
    let mut result = String::with_capacity(cleaned.len());
    let mut consecutive_newlines = 0u32;
    for ch in cleaned.chars() {
        if ch == '\n' {
            consecutive_newlines += 1;
            if consecutive_newlines <= 2 {
                result.push(ch);
            }
        } else {
            consecutive_newlines = 0;
            result.push(ch);
        }
    }

    // Step 4: Truncate to limit
    let output = if result.len() > MAX_EXTERNAL_CONTENT_LEN {
        // Find a safe char boundary
        let mut end = MAX_EXTERNAL_CONTENT_LEN - TRUNCATION_SUFFIX.len();
        while !result.is_char_boundary(end) && end > 0 {
            end -= 1;
        }
        let mut truncated = result[..end].to_string();
        truncated.push_str(TRUNCATION_SUFFIX);
        truncated
    } else {
        result
    };

    // Step 5: Detect injection patterns (warning-only — never reject)
    let matches = detect_injections(&output);
    if !matches.is_empty() {
        let patterns: Vec<&str> = matches.iter().map(|m| m.pattern.as_str()).collect();
        tracing::warn!(
            count = matches.len(),
            patterns = ?patterns,
            "Injection patterns detected in external message"
        );
    }

    output
}

/// Wrap external message content with random-ID boundary markers.
///
/// The random hex ID (16 characters) prevents an attacker from closing
/// the boundary block without guessing the ID.
///
/// Format:
/// ```text
/// <<<EXTERNAL_UNTRUSTED id="{random_hex}">>>
/// [content]
/// <<<END id="{random_hex}">>>
/// ```
pub fn wrap_with_boundary(content: &str) -> String {
    let boundary_id = generate_boundary_id();
    format!(
        "<<<EXTERNAL_UNTRUSTED id=\"{boundary_id}\">>>\n\
         {content}\n\
         <<<END id=\"{boundary_id}\">>>"
    )
}

/// Generate a random 16-character hex string for boundary markers.
pub fn generate_boundary_id() -> String {
    let mut rng = rand::rng();
    let bytes: [u8; 8] = rng.random();
    hex::encode(bytes)
}

/// Check if a sender key matches the parent identity key (trusted sender).
///
/// Returns `true` if the parent key is configured and matches the sender.
/// Empty parent key (dev mode) means no parent trust — all external messages
/// are untrusted.
pub fn is_parent_sender(sender: &str, parent_identity_key: &str) -> bool {
    !parent_identity_key.is_empty() && sender == parent_identity_key
}

// ---------------------------------------------------------------------------
// Leak detection — scan tool outputs for credential patterns before LLM context
// ---------------------------------------------------------------------------

/// Minimum length for the variable portion of `sk-` API keys.
const MIN_API_KEY_SUFFIX: usize = 20;

/// Minimum length for the variable portion of `Bearer ` tokens.
const MIN_BEARER_SUFFIX: usize = 20;

/// Minimum length for the variable portion of `authrite-` tokens.
const MIN_AUTHRITE_SUFFIX: usize = 10;

/// Characters valid in an API key suffix (alphanumeric, hyphen, underscore).
fn is_api_key_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'-' || b == b'_'
}

/// Characters valid in a Bearer token (alphanumeric, dot, hyphen, underscore).
fn is_bearer_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'.' || b == b'-' || b == b'_'
}

/// Characters valid in an authrite token suffix.
fn is_authrite_char(b: u8) -> bool {
    b.is_ascii_alphanumeric()
}

/// A detected credential leak in tool output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeakWarning {
    /// Human-readable pattern name (e.g. "api_key", "bearer_token").
    pub pattern: String,
    /// Byte offset where the match starts.
    pub start: usize,
    /// Byte offset where the match ends (exclusive).
    pub end: usize,
}

/// Scan tool output for credential patterns.
///
/// Returns a list of `LeakWarning` describing each match.
/// Patterns checked (4 total):
///   1. `sk-` API keys (20+ suffix chars)
///   2. `Bearer ` tokens (20+ suffix chars)
///   3. `x-bsv-auth-` headers (any length value after the prefix)
///   4. `authrite-` tokens (10+ suffix chars)
pub fn scan_for_leaks(output: &str) -> Vec<LeakWarning> {
    let bytes = output.as_bytes();
    let len = bytes.len();
    let mut warnings: Vec<LeakWarning> = Vec::new();

    let mut i = 0;
    while i < len {
        // Pattern 1: sk-<20+ api_key_chars>
        if bytes[i] == b's' && i + 3 < len && bytes[i + 1] == b'k' && bytes[i + 2] == b'-' {
            // Check word boundary: must not be preceded by an alphanumeric char
            if i == 0 || !bytes[i - 1].is_ascii_alphanumeric() {
                let start = i;
                let mut j = i + 3;
                while j < len && is_api_key_char(bytes[j]) {
                    j += 1;
                }
                let suffix_len = j - (i + 3);
                if suffix_len >= MIN_API_KEY_SUFFIX {
                    warnings.push(LeakWarning {
                        pattern: "api_key".to_string(),
                        start,
                        end: j,
                    });
                    i = j;
                    continue;
                }
            }
        }

        // Pattern 2: Bearer <20+ bearer_chars> (case-sensitive)
        if bytes[i] == b'B' && i + 7 < len && &bytes[i..i + 7] == b"Bearer " {
            let start = i;
            let mut j = i + 7;
            while j < len && is_bearer_char(bytes[j]) {
                j += 1;
            }
            let suffix_len = j - (i + 7);
            if suffix_len >= MIN_BEARER_SUFFIX {
                warnings.push(LeakWarning {
                    pattern: "bearer_token".to_string(),
                    start,
                    end: j,
                });
                i = j;
                continue;
            }
        }

        // Pattern 3: x-bsv-auth- header (case-insensitive prefix match)
        if (bytes[i] == b'x' || bytes[i] == b'X') && i + 11 < len {
            let candidate = &bytes[i..i + 11];
            if candidate.eq_ignore_ascii_case(b"x-bsv-auth-") {
                let start = i;
                // Consume until whitespace, comma, or end
                let mut j = i + 11;
                while j < len
                    && bytes[j] != b' '
                    && bytes[j] != b'\n'
                    && bytes[j] != b'\r'
                    && bytes[j] != b','
                    && bytes[j] != b'"'
                {
                    j += 1;
                }
                if j > i + 11 {
                    warnings.push(LeakWarning {
                        pattern: "auth_header".to_string(),
                        start,
                        end: j,
                    });
                    i = j;
                    continue;
                }
            }
        }

        // Pattern 4: authrite-<10+ authrite_chars> (case-insensitive prefix)
        if (bytes[i] == b'a' || bytes[i] == b'A') && i + 9 < len {
            let candidate = &bytes[i..i + 9];
            if candidate.eq_ignore_ascii_case(b"authrite-") {
                // Check word boundary
                if i == 0 || !bytes[i - 1].is_ascii_alphanumeric() {
                    let start = i;
                    let mut j = i + 9;
                    while j < len && is_authrite_char(bytes[j]) {
                        j += 1;
                    }
                    let suffix_len = j - (i + 9);
                    if suffix_len >= MIN_AUTHRITE_SUFFIX {
                        warnings.push(LeakWarning {
                            pattern: "authrite_token".to_string(),
                            start,
                            end: j,
                        });
                        i = j;
                        continue;
                    }
                }
            }
        }

        i += 1;
    }

    warnings
}

/// Redact all detected credential leaks in tool output.
///
/// Each match is replaced with `[REDACTED:pattern_name]`. Every redaction is
/// logged at WARN level. Returns the (possibly modified) string.
pub fn redact_leaks(output: &str) -> String {
    let warnings = scan_for_leaks(output);
    if warnings.is_empty() {
        return output.to_string();
    }

    let mut result = String::with_capacity(output.len());
    let mut last_end = 0;

    for w in &warnings {
        tracing::warn!(
            pattern = %w.pattern,
            start = w.start,
            end = w.end,
            "Credential leak detected in tool output — redacting"
        );
        result.push_str(&output[last_end..w.start]);
        result.push_str("[REDACTED:");
        result.push_str(&w.pattern);
        result.push(']');
        last_end = w.end;
    }

    result.push_str(&output[last_end..]);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_strips_null_bytes() {
        let input = "hello\0world\0";
        assert_eq!(sanitize_external_content(input), "helloworld");
    }

    #[test]
    fn test_sanitize_strips_control_chars() {
        // 0x01 (SOH), 0x07 (BEL), 0x1B (ESC) should be stripped
        let input = "hello\x01\x07\x1Bworld";
        assert_eq!(sanitize_external_content(input), "helloworld");
    }

    #[test]
    fn test_sanitize_preserves_newlines_and_tabs() {
        let input = "line1\nline2\tindented";
        assert_eq!(sanitize_external_content(input), "line1\nline2\tindented");
    }

    #[test]
    fn test_sanitize_collapses_excessive_newlines() {
        let input = "a\n\n\n\n\nb";
        assert_eq!(sanitize_external_content(input), "a\n\nb");
    }

    #[test]
    fn test_sanitize_preserves_double_newlines() {
        let input = "a\n\nb";
        assert_eq!(sanitize_external_content(input), "a\n\nb");
    }

    #[test]
    fn test_sanitize_truncates_long_content() {
        let long = "x".repeat(3000);
        let result = sanitize_external_content(&long);
        assert!(result.len() <= MAX_EXTERNAL_CONTENT_LEN);
        assert!(result.ends_with(TRUNCATION_SUFFIX));
    }

    #[test]
    fn test_sanitize_does_not_truncate_short_content() {
        let short = "hello world";
        assert_eq!(sanitize_external_content(short), "hello world");
    }

    #[test]
    fn test_sanitize_empty_input() {
        assert_eq!(sanitize_external_content(""), "");
    }

    #[test]
    fn test_boundary_marker_format() {
        let content = "test message";
        let wrapped = wrap_with_boundary(content);
        assert!(wrapped.starts_with("<<<EXTERNAL_UNTRUSTED id=\""));
        assert!(wrapped.contains("test message"));
        assert!(wrapped.contains("<<<END id=\""));
    }

    #[test]
    fn test_boundary_id_uniqueness() {
        let id1 = generate_boundary_id();
        let id2 = generate_boundary_id();
        assert_ne!(id1, id2);
        assert_eq!(id1.len(), 16);
        assert_eq!(id2.len(), 16);
    }

    #[test]
    fn test_boundary_id_is_hex() {
        let id = generate_boundary_id();
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_is_parent_sender_matches() {
        assert!(is_parent_sender("abc123", "abc123"));
    }

    #[test]
    fn test_is_parent_sender_no_match() {
        assert!(!is_parent_sender("abc123", "def456"));
    }

    #[test]
    fn test_is_parent_sender_empty_parent_key() {
        // Empty parent key means no parent trust — dev mode
        assert!(!is_parent_sender("abc123", ""));
    }

    #[test]
    fn test_external_tool_allowlist_contents() {
        let allowed = external_tool_allowlist();
        assert!(allowed.contains("memory_search"));
        assert!(allowed.contains("send_message"));
        assert!(allowed.contains("discover_services"));
        assert!(!allowed.contains("execute_bash"));
        assert!(!allowed.contains("file_write"));
        assert!(!allowed.contains("x402_call"));
        assert!(!allowed.contains("wallet_call"));
    }
}
