//! Structured logging setup for Dolphin Milk.
//!
//! JSON format for production, human-readable for development.
//! Uses tracing-subscriber for structured logging.
//!
//! ## Log redaction
//!
//! When enabled (default), the `RedactingWriter` scrubs sensitive patterns from
//! log output before writing to stderr. This prevents accidental leakage of API
//! keys, auth tokens, BEEF transaction data, and private keys in logs.
//!
//! Controlled by the `DOLPHIN_MILK_LOG_REDACT` env var (default: `true`).

use std::io::{self, Write};
use std::sync::LazyLock;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::EnvFilter;

// ---------------------------------------------------------------------------
// Redaction patterns — compiled once, matched on every log write
// ---------------------------------------------------------------------------

/// The replacement string for redacted content.
const REDACTED: &str = "[REDACTED]";

/// Minimum length for a base64 sequence to be considered sensitive (likely BEEF data).
const MIN_BASE64_LEN: usize = 200;

/// Minimum length for a hex string to be treated as a potential private key.
const HEX_PRIVKEY_LEN: usize = 64;

/// Minimum length for the variable portion of `sk-` API keys.
const MIN_API_KEY_SUFFIX: usize = 20;

/// Minimum length for the variable portion of `Bearer ` tokens.
const MIN_BEARER_SUFFIX: usize = 20;

/// Minimum length for the variable portion of `authrite-` tokens.
const MIN_AUTHRITE_SUFFIX: usize = 10;

/// Characters valid in a base64 string.
fn is_base64_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'+' || b == b'/' || b == b'='
}

/// Characters valid in an API key suffix (alphanumeric).
fn is_api_key_char(b: u8) -> bool {
    b.is_ascii_alphanumeric()
}

/// Characters valid in a Bearer token (alphanumeric, dot, hyphen, underscore).
fn is_bearer_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'.' || b == b'-' || b == b'_'
}

/// Characters valid in an authrite token suffix.
fn is_authrite_char(b: u8) -> bool {
    b.is_ascii_alphanumeric()
}

/// Whether the byte is a hex character (case-insensitive).
fn is_hex_char(b: u8) -> bool {
    b.is_ascii_hexdigit()
}

/// Whether the byte is a word character (for word boundary checks).
fn is_word_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

// ---------------------------------------------------------------------------
// Redaction check — used to skip redaction early
// ---------------------------------------------------------------------------

/// Whether log redaction is enabled. Defaults to true.
/// Set `DOLPHIN_MILK_LOG_REDACT=false` to disable.
static REDACTION_ENABLED: LazyLock<bool> = LazyLock::new(|| {
    let val = std::env::var("DOLPHIN_MILK_LOG_REDACT");
    match val {
        Ok(val) => !matches!(val.to_lowercase().as_str(), "false" | "0" | "no"),
        Err(_) => true,
    }
});

/// Returns true if log redaction is enabled.
pub fn is_redaction_enabled() -> bool {
    *REDACTION_ENABLED
}

// ---------------------------------------------------------------------------
// Core redaction function
// ---------------------------------------------------------------------------

/// Apply all redaction patterns to the input string.
///
/// This function processes the input left-to-right, checking for sensitive
/// patterns at each position. When a pattern matches, the matched span is
/// replaced with `[REDACTED]` and scanning continues after the replacement.
///
/// Patterns checked (in priority order at each position):
/// 1. `sk-` followed by 20+ alphanumeric chars (OpenAI-style API keys)
/// 2. `Bearer ` followed by 20+ token chars (auth tokens)
/// 3. `x-bsv-auth-` header patterns (case-insensitive)
/// 4. `authrite-` followed by 10+ alphanumeric chars
/// 5. 200+ consecutive base64 characters (likely BEEF transaction data)
/// 6. 64-char lowercase hex at word boundary (potential private keys)
pub fn redact(input: &str) -> String {
    if input.is_empty() {
        return String::new();
    }

    let bytes = input.as_bytes();
    let len = bytes.len();
    let mut result = String::with_capacity(len);
    let mut i = 0;

    while i < len {
        // Try each pattern at position i
        if let Some(skip) = try_redact_api_key(bytes, i, len) {
            result.push_str(REDACTED);
            i += skip;
        } else if let Some(skip) = try_redact_bearer(bytes, i, len) {
            result.push_str(REDACTED);
            i += skip;
        } else if let Some(skip) = try_redact_bsv_auth_header(bytes, i, len) {
            result.push_str(REDACTED);
            i += skip;
        } else if let Some(skip) = try_redact_authrite(bytes, i, len) {
            result.push_str(REDACTED);
            i += skip;
        } else if let Some(skip) = try_redact_base64_blob(bytes, i, len) {
            result.push_str(REDACTED);
            i += skip;
        } else if let Some(skip) = try_redact_hex_privkey(bytes, i, len) {
            result.push_str(REDACTED);
            i += skip;
        } else {
            // No pattern matched — copy this byte verbatim
            result.push(bytes[i] as char);
            i += 1;
        }
    }

    result
}

// ---------------------------------------------------------------------------
// Individual pattern matchers
// ---------------------------------------------------------------------------

/// Match `sk-` followed by MIN_API_KEY_SUFFIX+ alphanumeric characters.
fn try_redact_api_key(bytes: &[u8], pos: usize, len: usize) -> Option<usize> {
    // Need at least "sk-" + MIN_API_KEY_SUFFIX chars
    if pos + 3 + MIN_API_KEY_SUFFIX > len {
        return None;
    }
    if bytes[pos] != b's' || bytes[pos + 1] != b'k' || bytes[pos + 2] != b'-' {
        return None;
    }
    // Count consecutive alphanumeric chars after "sk-"
    let start = pos + 3;
    let mut end = start;
    while end < len && is_api_key_char(bytes[end]) {
        end += 1;
    }
    let suffix_len = end - start;
    if suffix_len >= MIN_API_KEY_SUFFIX {
        Some(end - pos)
    } else {
        None
    }
}

/// Match `Bearer ` followed by MIN_BEARER_SUFFIX+ token characters.
fn try_redact_bearer(bytes: &[u8], pos: usize, len: usize) -> Option<usize> {
    let prefix = b"Bearer ";
    if pos + prefix.len() + MIN_BEARER_SUFFIX > len {
        return None;
    }
    if &bytes[pos..pos + prefix.len()] != prefix {
        return None;
    }
    let start = pos + prefix.len();
    let mut end = start;
    while end < len && is_bearer_char(bytes[end]) {
        end += 1;
    }
    let suffix_len = end - start;
    if suffix_len >= MIN_BEARER_SUFFIX {
        Some(end - pos)
    } else {
        None
    }
}

/// Match `x-bsv-auth-<word>:` followed by a non-whitespace value.
/// Case-insensitive for the header name.
fn try_redact_bsv_auth_header(bytes: &[u8], pos: usize, len: usize) -> Option<usize> {
    let prefix = b"x-bsv-auth-";
    if pos + prefix.len() + 2 > len {
        return None;
    }
    // Case-insensitive match on the prefix
    for (j, &expected) in prefix.iter().enumerate() {
        let actual = bytes[pos + j].to_ascii_lowercase();
        if actual != expected {
            return None;
        }
    }
    // After "x-bsv-auth-", expect alpha chars for the header name (case-insensitive)
    let name_start = pos + prefix.len();
    let mut name_end = name_start;
    while name_end < len && bytes[name_end].is_ascii_alphabetic() {
        name_end += 1;
    }
    if name_end == name_start {
        return None; // No header name
    }
    // Expect colon
    if name_end >= len || bytes[name_end] != b':' {
        return None;
    }
    let mut end = name_end + 1;
    // Skip optional whitespace after colon
    while end < len && bytes[end] == b' ' {
        end += 1;
    }
    // Consume non-whitespace value
    let value_start = end;
    while end < len && !bytes[end].is_ascii_whitespace() {
        end += 1;
    }
    if end == value_start {
        return None; // No value after colon
    }
    Some(end - pos)
}

/// Match `authrite-` followed by MIN_AUTHRITE_SUFFIX+ alphanumeric characters.
fn try_redact_authrite(bytes: &[u8], pos: usize, len: usize) -> Option<usize> {
    let prefix = b"authrite-";
    if pos + prefix.len() + MIN_AUTHRITE_SUFFIX > len {
        return None;
    }
    if &bytes[pos..pos + prefix.len()] != prefix {
        return None;
    }
    let start = pos + prefix.len();
    let mut end = start;
    while end < len && is_authrite_char(bytes[end]) {
        end += 1;
    }
    let suffix_len = end - start;
    if suffix_len >= MIN_AUTHRITE_SUFFIX {
        Some(end - pos)
    } else {
        None
    }
}

/// Match MIN_BASE64_LEN+ consecutive base64 characters.
/// Only triggers if the sequence is preceded by a non-base64 char or start-of-string,
/// and followed by a non-base64 char or end-of-string (boundary check).
fn try_redact_base64_blob(bytes: &[u8], pos: usize, len: usize) -> Option<usize> {
    if pos + MIN_BASE64_LEN > len {
        return None;
    }
    if !is_base64_char(bytes[pos]) {
        return None;
    }
    // Check left boundary: must not be preceded by a base64 char
    // (avoids matching in the middle of an already-started sequence)
    if pos > 0 && is_base64_char(bytes[pos - 1]) {
        return None;
    }
    let mut end = pos;
    while end < len && is_base64_char(bytes[end]) {
        end += 1;
    }
    let match_len = end - pos;
    if match_len >= MIN_BASE64_LEN {
        Some(match_len)
    } else {
        None
    }
}

/// Match exactly HEX_PRIVKEY_LEN hex characters at a word boundary.
/// Case-insensitive hex matching, but requires word boundaries on both sides.
fn try_redact_hex_privkey(bytes: &[u8], pos: usize, len: usize) -> Option<usize> {
    if pos + HEX_PRIVKEY_LEN > len {
        return None;
    }
    // Word boundary check on the left
    if pos > 0 && is_word_char(bytes[pos - 1]) {
        return None;
    }
    // Check exactly HEX_PRIVKEY_LEN hex chars
    for j in 0..HEX_PRIVKEY_LEN {
        if !is_hex_char(bytes[pos + j]) {
            return None;
        }
    }
    let end = pos + HEX_PRIVKEY_LEN;
    // Word boundary check on the right (must NOT be followed by a word char)
    if end < len && is_word_char(bytes[end]) {
        return None;
    }
    Some(HEX_PRIVKEY_LEN)
}

// ---------------------------------------------------------------------------
// RedactingWriter — wraps stderr with pattern-based redaction
// ---------------------------------------------------------------------------

/// A writer that buffers log output and applies redaction before forwarding
/// to stderr. Created by `RedactingMakeWriter`.
pub struct RedactingWriter {
    buffer: Vec<u8>,
}

impl RedactingWriter {
    fn new() -> Self {
        Self {
            buffer: Vec::with_capacity(512),
        }
    }
}

impl Write for RedactingWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        // Buffer the bytes — tracing may call write() multiple times per line
        self.buffer.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.buffer.is_empty() {
            return Ok(());
        }
        // Convert to string, redact, write to stderr
        let text = String::from_utf8_lossy(&self.buffer);
        let redacted = redact(&text);
        let mut stderr = io::stderr();
        stderr.write_all(redacted.as_bytes())?;
        stderr.flush()?;
        self.buffer.clear();
        Ok(())
    }
}

impl Drop for RedactingWriter {
    fn drop(&mut self) {
        if !self.buffer.is_empty() {
            // Flush remaining buffer on drop
            let text = String::from_utf8_lossy(&self.buffer);
            let redacted = redact(&text);
            let _ = io::stderr().write_all(redacted.as_bytes());
        }
    }
}

/// Factory that produces `RedactingWriter` instances for the tracing subscriber.
#[derive(Clone)]
pub struct RedactingMakeWriter;

impl<'a> MakeWriter<'a> for RedactingMakeWriter {
    type Writer = RedactingWriter;

    fn make_writer(&'a self) -> Self::Writer {
        RedactingWriter::new()
    }
}

// ---------------------------------------------------------------------------
// Public setup function
// ---------------------------------------------------------------------------

/// Initialize tracing subscriber based on config.
///
/// When `DOLPHIN_MILK_LOG_REDACT` is not `false`/`0`/`no` (the default), all log
/// output passes through `RedactingWriter` which scrubs sensitive patterns.
pub fn setup_logging(level: &str, format: &str) {
    let filter = EnvFilter::try_new(format!("dolphin_milk={level},warn"))
        .unwrap_or_else(|_| EnvFilter::new("dolphin_milk=info,warn"));

    if is_redaction_enabled() {
        match format {
            "json" => {
                tracing_subscriber::fmt()
                    .json()
                    .with_env_filter(filter)
                    .with_target(true)
                    .with_writer(RedactingMakeWriter)
                    .init();
            }
            _ => {
                tracing_subscriber::fmt()
                    .with_env_filter(filter)
                    .with_target(true)
                    .with_writer(RedactingMakeWriter)
                    .init();
            }
        }
    } else {
        match format {
            "json" => {
                tracing_subscriber::fmt()
                    .json()
                    .with_env_filter(filter)
                    .with_target(true)
                    .with_writer(std::io::stderr)
                    .init();
            }
            _ => {
                tracing_subscriber::fmt()
                    .with_env_filter(filter)
                    .with_target(true)
                    .with_writer(std::io::stderr)
                    .init();
            }
        }
    }
}
