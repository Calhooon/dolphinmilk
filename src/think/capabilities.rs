//! Model capability detection and limits.
//!
//! Provides hardcoded fallback values for model context windows and output limits
//! when x402-info discovery is unavailable, plus the `ModelCapabilities` struct
//! for discovered capabilities.

/// Max input tokens for a model (context_window - max_output_tokens).
/// Hardcoded fallback when x402-info discovery is unavailable.
/// Values sourced from x402-info manifests (verified 2026-03-24).
pub fn model_input_limit(model: &str) -> usize {
    match model {
        // OpenAI models: 400K context, 128K output → 272K input
        m if m.starts_with("gpt-5") => 272_000,
        // o-series: 200K context, 100K output → 100K input
        m if m.starts_with("o4") || m.starts_with("o3") => 100_000,
        // Claude Haiku: 200K context, 64K output → 136K input
        m if m.starts_with("claude-haiku") => 136_000,
        // Claude Sonnet: 1M context, 64K output → 936K input
        m if m.starts_with("claude-sonnet") => 936_000,
        // Claude Opus: 1M context, 128K output → 872K input
        m if m.starts_with("claude-opus") => 872_000,
        // Conservative fallback for unknown models
        _ => 128_000,
    }
}

/// Max output tokens for a model.
/// Hardcoded fallback when x402-info discovery is unavailable.
pub fn model_output_limit(model: &str) -> usize {
    match model {
        m if m.starts_with("gpt-5") => 128_000,
        m if m.starts_with("o4") || m.starts_with("o3") => 100_000,
        m if m.starts_with("claude-haiku") => 64_000,
        m if m.starts_with("claude-sonnet") => 64_000,
        m if m.starts_with("claude-opus") => 128_000,
        _ => 16_384,
    }
}

/// Capabilities discovered for a specific model from x402-info manifests.
///
/// Resolution chain: x402-info discovery → hardcoded fallback → config cap.
/// Config values always act as caps (user can lower for cost control).
#[derive(Debug, Clone)]
pub struct ModelCapabilities {
    pub model: String,
    pub context_window: usize,
    pub max_output_tokens: usize,
    pub max_input_tokens: usize,
    pub supports_tools: bool,
    pub supports_vision: bool,
}

impl ModelCapabilities {
    /// Create from x402-info manifest data.
    pub fn from_manifest(
        model: &str,
        context_window: usize,
        max_output_tokens: usize,
        supports_tools: bool,
        supports_vision: bool,
    ) -> Self {
        Self {
            model: model.to_string(),
            context_window,
            max_output_tokens,
            max_input_tokens: context_window.saturating_sub(max_output_tokens),
            supports_tools,
            supports_vision,
        }
    }

    /// Create from hardcoded fallback values.
    pub fn from_hardcoded(model: &str) -> Self {
        let input = model_input_limit(model);
        let output = model_output_limit(model);
        Self {
            model: model.to_string(),
            context_window: input + output,
            max_output_tokens: output,
            max_input_tokens: input,
            supports_tools: true,
            supports_vision: true,
        }
    }
}
