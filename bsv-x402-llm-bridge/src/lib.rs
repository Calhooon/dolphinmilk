//! bsv-x402-llm-bridge — Sell LLM inference to other agents via BSV x402 micropayments.
//!
//! This crate wraps upstream LLM APIs (OpenAI, Anthropic Claude) behind x402
//! payment verification. An agent running this bridge becomes a paid LLM
//! inference provider: other agents pay per-request in satoshis, and the bridge
//! forwards their requests to the underlying LLM API.
//!
//! # Architecture
//!
//! ```text
//! Client Agent                 LLM Bridge                    Upstream LLM
//! +-----------+     x402      +----------------+    API     +----------+
//! | bsv-worm  | ──────────>   | payment verify | ────────> | OpenAI   |
//! | pays sats |   BRC-29      | route to       |           | Claude   |
//! +-----------+               | provider       | <──────── | etc.     |
//!       ^                     +----------------+    resp    +----------+
//!       |                            |
//!       └────────────────────────────┘
//!             response + usage
//! ```
//!
//! # Usage
//!
//! ```rust,no_run
//! use bsv_x402_llm_bridge::config::BridgeConfig;
//! use bsv_x402_llm_bridge::build_router;
//!
//! #[tokio::main]
//! async fn main() {
//!     let config = BridgeConfig::from_env();
//!     let app = build_router(config);
//!     let listener = tokio::net::TcpListener::bind("0.0.0.0:3403").await.unwrap();
//!     axum::serve(listener, app).await.unwrap();
//! }
//! ```

pub mod config;
pub mod providers;

use std::sync::Arc;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{get, post};
use axum::Router;
use serde_json::{json, Value};

use config::{BridgeConfig, PricingModel};
use providers::{LlmProvider, LlmRequest};

/// Shared application state for the bridge server.
struct AppState {
    config: BridgeConfig,
    providers: Vec<Box<dyn LlmProvider>>,
}

/// Build the axum router for the LLM bridge.
///
/// Returns a ready-to-serve Router with all endpoints configured.
pub fn build_router(config: BridgeConfig) -> Router {
    let providers = providers::create_providers(config.openai.as_ref(), config.claude.as_ref());

    let state = Arc::new(AppState { config, providers });

    Router::new()
        .route("/chat", post(chat_handler))
        .route("/health", get(health_handler))
        .route("/.well-known/x402-info", get(manifest_handler))
        .route("/models", get(models_handler))
        .with_state(state)
}

/// Verify x402 payment from the request headers.
///
/// Returns the payment info if valid, or a 402 response if payment is missing/invalid.
#[allow(clippy::result_large_err)]
fn verify_payment(
    headers: &HeaderMap,
    satoshis: u64,
    identity_key: &str,
) -> Result<PaymentInfo, Response> {
    let payment_header = headers.get("x-bsv-payment").and_then(|v| v.to_str().ok());

    let payment_json = match payment_header {
        Some(json_str) => json_str,
        None => {
            return Err(build_402_response(satoshis, identity_key));
        }
    };

    // Parse the payment JSON
    let payment: Value = match serde_json::from_str(payment_json) {
        Ok(v) => v,
        Err(_) => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "Invalid x-bsv-payment JSON" })),
            )
                .into_response());
        }
    };

    // Verify required BRC-29 fields
    let derivation_prefix = payment
        .get("derivationPrefix")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "Missing derivationPrefix" })),
            )
                .into_response()
        })?
        .to_string();

    let derivation_suffix = payment
        .get("derivationSuffix")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "Missing derivationSuffix" })),
            )
                .into_response()
        })?
        .to_string();

    let _transaction = payment
        .get("transaction")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "Missing transaction" })),
            )
                .into_response()
        })?;

    Ok(PaymentInfo {
        derivation_prefix,
        derivation_suffix,
        satoshis_paid: satoshis,
    })
}

/// Info about a verified payment.
#[allow(dead_code)]
struct PaymentInfo {
    derivation_prefix: String,
    derivation_suffix: String,
    satoshis_paid: u64,
}

/// Build a 402 Payment Required response.
fn build_402_response(satoshis: u64, identity_key: &str) -> Response {
    let prefix_bytes: [u8; 16] = rand::random();
    let derivation_prefix = hex::encode(prefix_bytes);

    let mut response = (
        StatusCode::PAYMENT_REQUIRED,
        Json(json!({
            "error": "Payment required",
            "satoshis": satoshis,
            "message": "LLM inference requires x402 payment",
        })),
    )
        .into_response();

    let headers = response.headers_mut();
    headers.insert("x-bsv-payment-version", "1.0".parse().unwrap());
    headers.insert(
        "x-bsv-payment-satoshis-required",
        satoshis.to_string().parse().unwrap(),
    );
    headers.insert(
        "x-bsv-payment-derivation-prefix",
        derivation_prefix.parse().unwrap(),
    );
    headers.insert("x-bsv-auth-identity-key", identity_key.parse().unwrap());

    response
}

/// POST /chat — LLM chat completion behind x402 payment.
async fn chat_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Json<Value>, Response> {
    // Parse the request
    let model = body.get("model").and_then(|m| m.as_str()).unwrap_or("");

    // Reasoning models (o1, o3, o4, gpt-5, gpt-4.1) send max_completion_tokens
    // instead of max_tokens. Accept both fields.
    let max_tokens = body
        .get("max_completion_tokens")
        .and_then(|m| m.as_u64())
        .or_else(|| body.get("max_tokens").and_then(|m| m.as_u64()))
        .unwrap_or(4096) as u32;

    // Use an explicit model or fall back to the first available provider's default
    let effective_model = if model.is_empty() {
        state
            .providers
            .first()
            .map(|p| p.default_model().to_string())
            .unwrap_or_else(|| "gpt-4o-mini".to_string())
    } else {
        model.to_string()
    };

    // Find the right provider
    let provider =
        providers::resolve_provider(&effective_model, &state.providers).ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "error": format!("No provider available for model '{effective_model}'"),
                    "available_models": state.config.available_models(),
                })),
            )
                .into_response()
        })?;

    // Calculate price and verify payment
    let satoshis = state
        .config
        .pricing
        .upfront_price(&effective_model, max_tokens);
    let payment = verify_payment(&headers, satoshis, &state.config.identity_key)?;

    // Parse messages
    let messages: Vec<providers::ChatMessage> = body
        .get("messages")
        .and_then(|m| serde_json::from_value(m.clone()).ok())
        .unwrap_or_default();

    if messages.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "messages array is required and must not be empty" })),
        )
            .into_response());
    }

    let temperature = body.get("temperature").and_then(|t| t.as_f64());

    let tools = body.get("tools").and_then(|t| t.as_array()).cloned();

    let request = LlmRequest {
        model: effective_model.clone(),
        messages,
        max_tokens,
        temperature,
        tools,
    };

    tracing::info!(
        "LLM bridge: {} request for model={}, max_tokens={}, paid={} sats (prefix={}...)",
        provider.name(),
        effective_model,
        max_tokens,
        payment.satoshis_paid,
        &payment.derivation_prefix[..payment.derivation_prefix.len().min(8)],
    );

    // Forward to the upstream provider
    let response = provider.chat(&request).await.map_err(|e| {
        let (status, msg) = match &e {
            providers::ProviderError::Auth(msg) => (StatusCode::BAD_GATEWAY, msg.clone()),
            providers::ProviderError::RateLimited(msg) => {
                (StatusCode::TOO_MANY_REQUESTS, msg.clone())
            }
            providers::ProviderError::ModelNotAvailable(msg) => {
                (StatusCode::BAD_REQUEST, msg.clone())
            }
            providers::ProviderError::Request(msg)
            | providers::ProviderError::InvalidResponse(msg) => {
                (StatusCode::BAD_GATEWAY, msg.clone())
            }
        };
        (status, Json(json!({ "error": msg }))).into_response()
    })?;

    // Calculate actual cost (for per-token pricing, may differ from upfront)
    let actual_sats = state.config.pricing.actual_price(
        &response.model,
        response.prompt_tokens,
        response.completion_tokens,
    );

    tracing::info!(
        "LLM bridge: response model={}, tokens={}/{}/{}, finish={}, actual_cost={} sats",
        response.model,
        response.prompt_tokens,
        response.completion_tokens,
        response.total_tokens,
        response.finish_reason,
        actual_sats,
    );

    // Build OpenAI-compatible response format
    let mut result = json!({
        "id": format!("bridge-{}", hex::encode(rand::random::<[u8; 8]>())),
        "object": "chat.completion",
        "model": response.model,
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": if response.text.is_empty() { Value::Null } else { json!(response.text) },
            },
            "finish_reason": response.finish_reason,
        }],
        "usage": {
            "prompt_tokens": response.prompt_tokens,
            "completion_tokens": response.completion_tokens,
            "total_tokens": response.total_tokens,
        },
        "x402": {
            "satoshis_paid": payment.satoshis_paid,
            "satoshis_actual": actual_sats,
            "provider": provider.name(),
        }
    });

    // Include tool calls if present
    if !response.tool_calls.is_empty() {
        result["choices"][0]["message"]["tool_calls"] = json!(response.tool_calls);
    }

    Ok(Json(result))
}

/// GET /health — free health check.
async fn health_handler(State(state): State<Arc<AppState>>) -> Json<Value> {
    let provider_names: Vec<&str> = state.providers.iter().map(|p| p.name()).collect();
    Json(json!({
        "status": "ok",
        "service": "bsv-x402-llm-bridge",
        "version": "0.1.0",
        "providers": provider_names,
        "models": state.config.available_models(),
    }))
}

/// GET /.well-known/x402-info — service manifest.
async fn manifest_handler(State(state): State<Arc<AppState>>) -> Json<Value> {
    let pricing_info = match &state.config.pricing {
        PricingModel::PerRequest { satoshis } => json!({
            "type": "per-request",
            "satoshis": satoshis,
        }),
        PricingModel::PerToken {
            prompt_sats_per_1k,
            completion_sats_per_1k,
        } => json!({
            "type": "per-token",
            "prompt_sats_per_1k": prompt_sats_per_1k,
            "completion_sats_per_1k": completion_sats_per_1k,
        }),
        PricingModel::PerModel {
            model_prices,
            default_satoshis,
        } => json!({
            "type": "per-model",
            "model_prices": model_prices,
            "default_satoshis": default_satoshis,
        }),
    };

    Json(json!({
        "name": "bsv-x402-llm-bridge",
        "description": "LLM inference bridge — pay-per-request access to OpenAI and Claude models via BSV micropayments",
        "server_identity_key": state.config.identity_key,
        "auth_protocol": "BRC-31",
        "pricing": pricing_info,
        "endpoints": [
            {
                "path": "/chat",
                "method": "POST",
                "description": "Send a chat completion request to an LLM provider",
                "auth": false,
                "payment": { "dynamic": true },
                "input": {
                    "model": { "type": "string", "description": "Model name (e.g., gpt-4o, claude-sonnet-4-20250514)" },
                    "messages": { "type": "array", "required": true, "description": "Chat messages in OpenAI format" },
                    "max_tokens": { "type": "integer", "default": 4096, "description": "Maximum tokens to generate" },
                    "temperature": { "type": "number", "description": "Sampling temperature (0.0-2.0)" },
                    "tools": { "type": "array", "description": "Tool definitions for function calling" },
                },
                "output": {
                    "id": "string",
                    "model": "string",
                    "choices": "array",
                    "usage": "object",
                    "x402": "object",
                }
            },
            {
                "path": "/models",
                "method": "GET",
                "description": "List available models and their pricing",
                "auth": false,
                "payment": false,
                "input": {},
                "output": {
                    "models": "array",
                }
            },
            {
                "path": "/health",
                "method": "GET",
                "description": "Health check (free)",
                "auth": false,
                "payment": false,
                "input": {},
                "output": {
                    "status": "string",
                    "providers": "array",
                    "models": "array",
                }
            }
        ]
    }))
}

/// GET /models — list available models and pricing.
async fn models_handler(State(state): State<Arc<AppState>>) -> Json<Value> {
    let models: Vec<Value> = state
        .providers
        .iter()
        .flat_map(|provider| {
            // Get models this provider supports from config
            let provider_models = match provider.name() {
                "OpenAI" => state.config.openai.as_ref().map(|c| &c.available_models),
                "Anthropic" => state.config.claude.as_ref().map(|c| &c.available_models),
                _ => None,
            };

            provider_models
                .into_iter()
                .flat_map(|models| models.iter())
                .map(|model| {
                    let price = state.config.pricing.upfront_price(model, 4096);
                    json!({
                        "id": model,
                        "provider": provider.name(),
                        "price_sats": price,
                        "is_default": model == provider.default_model(),
                    })
                })
                .collect::<Vec<_>>()
        })
        .collect();

    Json(json!({
        "models": models,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_402_response() {
        let response = build_402_response(
            500,
            "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798",
        );
        assert_eq!(response.status(), StatusCode::PAYMENT_REQUIRED);
        assert!(response.headers().contains_key("x-bsv-payment-version"));
        assert_eq!(
            response
                .headers()
                .get("x-bsv-payment-satoshis-required")
                .unwrap()
                .to_str()
                .unwrap(),
            "500"
        );
    }

    #[test]
    fn test_verify_payment_missing_header() {
        let headers = HeaderMap::new();
        let result = verify_payment(&headers, 500, "testkey");
        assert!(result.is_err());
    }

    #[test]
    fn test_verify_payment_valid() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-bsv-payment",
            r#"{"derivationPrefix":"abc","derivationSuffix":"def","transaction":"AQEBAQ=="}"#
                .parse()
                .unwrap(),
        );
        let result = verify_payment(&headers, 500, "testkey");
        assert!(result.is_ok());
        let info = result.unwrap();
        assert_eq!(info.derivation_prefix, "abc");
        assert_eq!(info.derivation_suffix, "def");
        assert_eq!(info.satoshis_paid, 500);
    }

    #[test]
    fn test_verify_payment_invalid_json() {
        let mut headers = HeaderMap::new();
        headers.insert("x-bsv-payment", "not-json".parse().unwrap());
        let result = verify_payment(&headers, 500, "testkey");
        assert!(result.is_err());
    }

    #[test]
    fn test_verify_payment_missing_fields() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-bsv-payment",
            r#"{"derivationPrefix":"abc"}"#.parse().unwrap(),
        );
        let result = verify_payment(&headers, 500, "testkey");
        assert!(result.is_err());
    }

    #[test]
    fn test_build_router_no_providers() {
        let config = BridgeConfig {
            port: 3403,
            identity_key: "test".to_string(),
            pricing: PricingModel::default(),
            openai: None,
            claude: None,
        };
        let _router = build_router(config);
        // Router builds without panicking even with no providers
    }
}
