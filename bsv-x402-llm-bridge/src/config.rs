//! Configuration for the x402 LLM bridge.
//!
//! All configuration is loaded from environment variables, with sensible defaults.
//! API keys for upstream LLM providers must be set via environment variables;
//! they are never hardcoded.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Pricing model for LLM inference.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PricingModel {
    /// Fixed price per request, regardless of token count.
    PerRequest { satoshis: u64 },
    /// Price based on token usage (prompt + completion).
    PerToken {
        /// Satoshis per 1000 prompt (input) tokens.
        prompt_sats_per_1k: u64,
        /// Satoshis per 1000 completion (output) tokens.
        completion_sats_per_1k: u64,
    },
    /// Price based on the model used, with per-model fixed rates.
    PerModel {
        /// Map of model name -> satoshis per request.
        model_prices: HashMap<String, u64>,
        /// Default price for models not in the map.
        default_satoshis: u64,
    },
}

impl Default for PricingModel {
    fn default() -> Self {
        PricingModel::PerRequest { satoshis: 500 }
    }
}

impl PricingModel {
    /// Calculate the upfront price in satoshis for a request.
    ///
    /// For per-token pricing, this returns an estimate based on max_tokens.
    /// The actual cost is calculated after the response is received.
    pub fn upfront_price(&self, model: &str, max_tokens: u32) -> u64 {
        match self {
            PricingModel::PerRequest { satoshis } => *satoshis,
            PricingModel::PerToken {
                prompt_sats_per_1k,
                completion_sats_per_1k,
            } => {
                // Estimate: assume prompt is ~1000 tokens, completion is max_tokens
                let prompt_cost = prompt_sats_per_1k;
                let completion_cost = (*completion_sats_per_1k * max_tokens as u64) / 1000;
                prompt_cost + completion_cost
            }
            PricingModel::PerModel {
                model_prices,
                default_satoshis,
            } => *model_prices.get(model).unwrap_or(default_satoshis),
        }
    }

    /// Calculate the actual cost in satoshis after receiving a response.
    ///
    /// For per-request and per-model pricing, this is the same as upfront_price.
    /// For per-token pricing, this uses the actual token counts.
    pub fn actual_price(&self, model: &str, prompt_tokens: u64, completion_tokens: u64) -> u64 {
        match self {
            PricingModel::PerRequest { satoshis } => *satoshis,
            PricingModel::PerToken {
                prompt_sats_per_1k,
                completion_sats_per_1k,
            } => {
                let prompt_cost = (*prompt_sats_per_1k * prompt_tokens) / 1000;
                let completion_cost = (*completion_sats_per_1k * completion_tokens) / 1000;
                // Minimum 1 sat to avoid free rides
                (prompt_cost + completion_cost).max(1)
            }
            PricingModel::PerModel {
                model_prices,
                default_satoshis,
            } => *model_prices.get(model).unwrap_or(default_satoshis),
        }
    }
}

/// Configuration for an upstream LLM provider.
#[derive(Debug, Clone)]
pub struct ProviderConfig {
    /// API key for the provider (from env var, never hardcoded).
    pub api_key: String,
    /// Base URL for the provider's API.
    pub base_url: String,
    /// Default model to use if the client doesn't specify one.
    pub default_model: String,
    /// Maximum tokens allowed per request.
    pub max_tokens_limit: u32,
    /// Models available through this provider.
    pub available_models: Vec<String>,
}

/// Bridge server configuration.
#[derive(Debug, Clone)]
pub struct BridgeConfig {
    /// Port to listen on.
    pub port: u16,
    /// Server identity key (compressed public key hex).
    pub identity_key: String,
    /// Pricing model for incoming requests.
    pub pricing: PricingModel,
    /// OpenAI provider config (if available).
    pub openai: Option<ProviderConfig>,
    /// Claude/Anthropic provider config (if available).
    pub claude: Option<ProviderConfig>,
}

impl BridgeConfig {
    /// Load configuration from environment variables.
    ///
    /// Environment variables:
    /// - `X402_BRIDGE_PORT`: Server port (default: 3403)
    /// - `X402_BRIDGE_IDENTITY_KEY`: Server identity key
    /// - `X402_BRIDGE_PRICING`: Pricing model ("per-request", "per-token", "per-model")
    /// - `X402_BRIDGE_PRICE_SATS`: Default price in satoshis (default: 500)
    /// - `OPENAI_API_KEY`: OpenAI API key
    /// - `OPENAI_BASE_URL`: OpenAI API base URL
    /// - `OPENAI_DEFAULT_MODEL`: Default OpenAI model
    /// - `ANTHROPIC_API_KEY`: Anthropic API key
    /// - `ANTHROPIC_BASE_URL`: Anthropic API base URL
    /// - `ANTHROPIC_DEFAULT_MODEL`: Default Anthropic model
    pub fn from_env() -> Self {
        let port = std::env::var("X402_BRIDGE_PORT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(3403);

        let identity_key = std::env::var("X402_BRIDGE_IDENTITY_KEY").unwrap_or_else(|_| {
            "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798".to_string()
        });

        let default_sats: u64 = std::env::var("X402_BRIDGE_PRICE_SATS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(500);

        let pricing_model =
            std::env::var("X402_BRIDGE_PRICING").unwrap_or_else(|_| "per-request".to_string());

        let pricing = match pricing_model.as_str() {
            "per-token" => PricingModel::PerToken {
                prompt_sats_per_1k: std::env::var("X402_BRIDGE_PROMPT_SATS_PER_1K")
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(10),
                completion_sats_per_1k: std::env::var("X402_BRIDGE_COMPLETION_SATS_PER_1K")
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(30),
            },
            "per-model" => {
                let mut model_prices = HashMap::new();
                // Parse model prices from env: X402_BRIDGE_MODEL_PRICE_<MODEL>=<sats>
                for (key, val) in std::env::vars() {
                    if let Some(model_name) = key.strip_prefix("X402_BRIDGE_MODEL_PRICE_") {
                        if let Ok(sats) = val.parse::<u64>() {
                            model_prices.insert(model_name.to_lowercase().replace('_', "-"), sats);
                        }
                    }
                }
                PricingModel::PerModel {
                    model_prices,
                    default_satoshis: default_sats,
                }
            }
            _ => PricingModel::PerRequest {
                satoshis: default_sats,
            },
        };

        let openai = std::env::var("OPENAI_API_KEY")
            .ok()
            .map(|api_key| ProviderConfig {
                api_key,
                base_url: std::env::var("OPENAI_BASE_URL")
                    .unwrap_or_else(|_| "https://api.openai.com/v1".to_string()),
                default_model: std::env::var("OPENAI_DEFAULT_MODEL")
                    .unwrap_or_else(|_| "gpt-4o-mini".to_string()),
                max_tokens_limit: std::env::var("OPENAI_MAX_TOKENS")
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(4096),
                available_models: vec![
                    "gpt-4o".to_string(),
                    "gpt-4o-mini".to_string(),
                    "gpt-4-turbo".to_string(),
                    "o1".to_string(),
                    "o3-mini".to_string(),
                ],
            });

        let claude = std::env::var("ANTHROPIC_API_KEY")
            .ok()
            .map(|api_key| ProviderConfig {
                api_key,
                base_url: std::env::var("ANTHROPIC_BASE_URL")
                    .unwrap_or_else(|_| "https://api.anthropic.com/v1".to_string()),
                default_model: std::env::var("ANTHROPIC_DEFAULT_MODEL")
                    .unwrap_or_else(|_| "claude-sonnet-4-20250514".to_string()),
                max_tokens_limit: std::env::var("ANTHROPIC_MAX_TOKENS")
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(4096),
                available_models: vec![
                    "claude-sonnet-4-20250514".to_string(),
                    "claude-opus-4-20250514".to_string(),
                    "claude-3-5-haiku-20241022".to_string(),
                ],
            });

        Self {
            port,
            identity_key,
            pricing,
            openai,
            claude,
        }
    }

    /// Check if any providers are configured.
    pub fn has_providers(&self) -> bool {
        self.openai.is_some() || self.claude.is_some()
    }

    /// List all available models across all configured providers.
    pub fn available_models(&self) -> Vec<String> {
        let mut models = Vec::new();
        if let Some(ref openai) = self.openai {
            models.extend(openai.available_models.clone());
        }
        if let Some(ref claude) = self.claude {
            models.extend(claude.available_models.clone());
        }
        models
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_per_request_pricing() {
        let pricing = PricingModel::PerRequest { satoshis: 500 };
        assert_eq!(pricing.upfront_price("gpt-4o", 4096), 500);
        assert_eq!(pricing.actual_price("gpt-4o", 100, 200), 500);
    }

    #[test]
    fn test_per_token_pricing() {
        let pricing = PricingModel::PerToken {
            prompt_sats_per_1k: 10,
            completion_sats_per_1k: 30,
        };
        // Upfront: estimate based on max_tokens
        let upfront = pricing.upfront_price("gpt-4o", 4096);
        assert!(upfront > 0);

        // Actual: based on real token counts
        let actual = pricing.actual_price("gpt-4o", 1000, 500);
        // 10 * 1000/1000 + 30 * 500/1000 = 10 + 15 = 25
        assert_eq!(actual, 25);
    }

    #[test]
    fn test_per_token_minimum() {
        let pricing = PricingModel::PerToken {
            prompt_sats_per_1k: 1,
            completion_sats_per_1k: 1,
        };
        // Very small request — should still cost at least 1 sat
        let actual = pricing.actual_price("gpt-4o", 1, 1);
        assert_eq!(actual, 1);
    }

    #[test]
    fn test_per_model_pricing() {
        let mut model_prices = HashMap::new();
        model_prices.insert("gpt-4o".to_string(), 1000);
        model_prices.insert("gpt-4o-mini".to_string(), 200);

        let pricing = PricingModel::PerModel {
            model_prices,
            default_satoshis: 500,
        };

        assert_eq!(pricing.upfront_price("gpt-4o", 4096), 1000);
        assert_eq!(pricing.upfront_price("gpt-4o-mini", 4096), 200);
        assert_eq!(pricing.upfront_price("unknown-model", 4096), 500);
    }

    #[test]
    fn test_default_pricing() {
        let pricing = PricingModel::default();
        assert_eq!(pricing.upfront_price("any-model", 1000), 500);
    }
}
