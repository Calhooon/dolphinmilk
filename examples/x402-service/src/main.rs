//! Example x402 paid API service.
//!
//! A minimal axum server with 3 example endpoints, each protected by x402
//! payment verification. When no payment is attached, the server returns
//! HTTP 402 Payment Required with the payment terms. When a valid BRC-29
//! payment is attached via the `x-bsv-payment` header, the endpoint executes
//! and returns the result.
//!
//! Usage:
//!     cargo run -p x402-service-example
//!
//! Then test with curl:
//!     # Without payment — returns 402 with payment terms
//!     curl -v http://localhost:3402/api/joke
//!
//!     # With payment (normally constructed by a BRC-100 wallet client)
//!     curl -H "x-bsv-payment: {\"derivationPrefix\":\"...\",\"derivationSuffix\":\"...\",\"transaction\":\"...\"}" \
//!          http://localhost:3402/api/joke

use axum::extract::Query;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{get, post};
use axum::Router;
use serde::Deserialize;
use serde_json::{json, Value};

/// Payment terms for an endpoint.
#[derive(Debug, Clone)]
struct PaymentTerms {
    /// Satoshis required for this endpoint.
    satoshis: u64,
    /// Human-readable description of what the payment is for.
    description: String,
}

/// Server configuration loaded from environment variables.
#[derive(Debug, Clone)]
struct ServerConfig {
    /// Port to listen on (default: 3402).
    port: u16,
    /// Server identity key (compressed public key hex).
    /// In production, this comes from the wallet. For the example, we use a placeholder.
    identity_key: String,
}

impl ServerConfig {
    fn from_env() -> Self {
        let port = std::env::var("X402_PORT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(3402);

        // In production, this would be the server's actual identity key from the wallet.
        // For the example, use a well-known test key (secp256k1 generator point).
        let identity_key = std::env::var("X402_IDENTITY_KEY").unwrap_or_else(|_| {
            "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798".to_string()
        });

        Self { port, identity_key }
    }
}

/// Check for a valid x-bsv-payment header and return 402 if missing.
///
/// In a production service, this would:
/// 1. Parse the payment JSON from the header
/// 2. Decode the AtomicBEEF transaction
/// 3. Verify the payment output matches the expected amount and derivation
/// 4. Broadcast the payment transaction
///
/// For this example, we verify the header is present and contains valid JSON
/// with the required BRC-29 fields.
#[allow(clippy::result_large_err)]
fn verify_payment(headers: &HeaderMap, terms: &PaymentTerms) -> Result<PaymentInfo, Response> {
    let payment_header = headers.get("x-bsv-payment").and_then(|v| v.to_str().ok());

    let payment_json = match payment_header {
        Some(json_str) => json_str,
        None => {
            return Err(payment_required_response(terms));
        }
    };

    // Parse the payment JSON
    let payment: Value = match serde_json::from_str(payment_json) {
        Ok(v) => v,
        Err(_) => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "error": "Invalid x-bsv-payment JSON",
                    "code": "INVALID_PAYMENT"
                })),
            )
                .into_response());
        }
    };

    // Verify required BRC-29 fields are present
    let derivation_prefix = payment
        .get("derivationPrefix")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "error": "Missing derivationPrefix in payment",
                    "code": "INVALID_PAYMENT"
                })),
            )
                .into_response()
        })?;

    let derivation_suffix = payment
        .get("derivationSuffix")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "error": "Missing derivationSuffix in payment",
                    "code": "INVALID_PAYMENT"
                })),
            )
                .into_response()
        })?;

    let transaction = payment
        .get("transaction")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "error": "Missing transaction in payment",
                    "code": "INVALID_PAYMENT"
                })),
            )
                .into_response()
        })?;

    // In production, you would:
    // 1. Decode the base64 AtomicBEEF transaction
    // 2. Verify it pays to the correct derivation path
    // 3. Verify the amount >= required satoshis
    // 4. Broadcast the transaction via the wallet
    //
    // For this example, we accept any well-formed payment.
    tracing::info!(
        "Payment received: prefix={}..., suffix={}..., tx_size={} chars",
        &derivation_prefix[..derivation_prefix.len().min(16)],
        &derivation_suffix[..derivation_suffix.len().min(16)],
        transaction.len(),
    );

    Ok(PaymentInfo {
        derivation_prefix: derivation_prefix.to_string(),
        derivation_suffix: derivation_suffix.to_string(),
        satoshis_paid: terms.satoshis,
    })
}

/// Info about a verified payment.
#[derive(Debug)]
#[allow(dead_code)]
struct PaymentInfo {
    derivation_prefix: String,
    derivation_suffix: String,
    satoshis_paid: u64,
}

/// Build a 402 Payment Required response with the standard x402 headers.
fn payment_required_response(terms: &PaymentTerms) -> Response {
    let config = ServerConfig::from_env();

    // Generate a derivation prefix for this payment session
    let prefix_bytes: [u8; 16] = rand::random();
    let derivation_prefix = hex::encode(prefix_bytes);

    let mut response = (
        StatusCode::PAYMENT_REQUIRED,
        Json(json!({
            "error": "Payment required",
            "message": terms.description,
            "satoshis": terms.satoshis,
            "code": "PAYMENT_REQUIRED"
        })),
    )
        .into_response();

    let headers = response.headers_mut();
    headers.insert("x-bsv-payment-version", "1.0".parse().unwrap());
    headers.insert(
        "x-bsv-payment-satoshis-required",
        terms.satoshis.to_string().parse().unwrap(),
    );
    headers.insert(
        "x-bsv-payment-derivation-prefix",
        derivation_prefix.parse().unwrap(),
    );
    headers.insert(
        "x-bsv-auth-identity-key",
        config.identity_key.parse().unwrap(),
    );

    response
}

// ---------------------------------------------------------------------------
// Endpoint handlers
// ---------------------------------------------------------------------------

/// GET /api/joke — returns a random joke (costs 100 sats)
async fn joke_handler(headers: HeaderMap) -> Result<Json<Value>, Response> {
    let terms = PaymentTerms {
        satoshis: 100,
        description: "Get a random joke".to_string(),
    };

    let payment = verify_payment(&headers, &terms)?;

    let jokes = [
        "Why do programmers prefer dark mode? Because light attracts bugs.",
        "There are only 10 types of people in the world: those who understand binary and those who don't.",
        "A SQL query walks into a bar, sees two tables, and asks: 'Can I join you?'",
        "Why do Java developers wear glasses? Because they don't C#.",
        "What's a cryptographer's favorite dance? The algo-rhythm.",
        "How many satoshis does it take to change a lightbulb? None, they just fork the protocol.",
    ];

    let index = rand::random::<u32>() as usize % jokes.len();

    tracing::info!(
        "Joke served, paid {} sats (prefix={}...)",
        payment.satoshis_paid,
        &payment.derivation_prefix[..payment.derivation_prefix.len().min(8)],
    );

    Ok(Json(json!({
        "joke": jokes[index],
        "category": "programming",
        "satoshis_paid": payment.satoshis_paid,
    })))
}

/// Query parameters for the echo endpoint.
#[derive(Debug, Deserialize)]
struct EchoQuery {
    /// The message to echo back.
    message: Option<String>,
}

/// Echo request body.
#[derive(Debug, Deserialize)]
struct EchoBody {
    /// The message to echo back.
    message: Option<String>,
}

/// POST /api/echo — echoes back the input message (costs 50 sats)
async fn echo_handler(
    headers: HeaderMap,
    Query(query): Query<EchoQuery>,
    body: Option<Json<EchoBody>>,
) -> Result<Json<Value>, Response> {
    let terms = PaymentTerms {
        satoshis: 50,
        description: "Echo back a message".to_string(),
    };

    let payment = verify_payment(&headers, &terms)?;

    // Get message from query params or body
    let message = query
        .message
        .or_else(|| body.and_then(|b| b.message.clone()))
        .unwrap_or_else(|| "Hello, x402!".to_string());

    tracing::info!(
        "Echo served: '{}', paid {} sats",
        &message[..message.len().min(50)],
        payment.satoshis_paid,
    );

    Ok(Json(json!({
        "echo": message,
        "length": message.len(),
        "satoshis_paid": payment.satoshis_paid,
    })))
}

/// GET /api/time — returns the current server time (costs 25 sats)
async fn time_handler(headers: HeaderMap) -> Result<Json<Value>, Response> {
    let terms = PaymentTerms {
        satoshis: 25,
        description: "Get the current server time".to_string(),
    };

    let payment = verify_payment(&headers, &terms)?;

    let now = chrono::Utc::now();

    tracing::info!("Time served, paid {} sats", payment.satoshis_paid);

    Ok(Json(json!({
        "utc": now.to_rfc3339(),
        "unix_timestamp": now.timestamp(),
        "unix_millis": now.timestamp_millis(),
        "satoshis_paid": payment.satoshis_paid,
    })))
}

/// GET /health — free health check endpoint (no payment required)
async fn health_handler() -> Json<Value> {
    Json(json!({
        "status": "ok",
        "service": "x402-service-example",
        "version": "0.1.0",
    }))
}

/// GET /.well-known/x402-info — service manifest (no payment required)
///
/// Describes the service's endpoints, pricing, and capabilities
/// following the x402 manifest format.
async fn manifest_handler() -> Json<Value> {
    let config = ServerConfig::from_env();

    Json(json!({
        "name": "x402-service-example",
        "description": "Example x402 paid API service with jokes, echo, and time endpoints",
        "server_identity_key": config.identity_key,
        "auth_protocol": "BRC-31",
        "endpoints": [
            {
                "path": "/api/joke",
                "method": "GET",
                "description": "Get a random programming joke",
                "auth": false,
                "payment": { "satoshis": 100 },
                "input": {},
                "output": {
                    "joke": "string",
                    "category": "string",
                    "satoshis_paid": "integer"
                }
            },
            {
                "path": "/api/echo",
                "method": "POST",
                "description": "Echo back a message with metadata",
                "auth": false,
                "payment": { "satoshis": 50 },
                "input": {
                    "message": { "type": "string", "description": "Message to echo back", "required": false }
                },
                "output": {
                    "echo": "string",
                    "length": "integer",
                    "satoshis_paid": "integer"
                }
            },
            {
                "path": "/api/time",
                "method": "GET",
                "description": "Get the current server time in multiple formats",
                "auth": false,
                "payment": { "satoshis": 25 },
                "input": {},
                "output": {
                    "utc": "string",
                    "unix_timestamp": "integer",
                    "unix_millis": "integer",
                    "satoshis_paid": "integer"
                }
            },
            {
                "path": "/health",
                "method": "GET",
                "description": "Health check (free, no payment required)",
                "auth": false,
                "payment": false,
                "input": {},
                "output": {
                    "status": "string",
                    "service": "string",
                    "version": "string"
                }
            }
        ]
    }))
}

#[tokio::main]
async fn main() {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".parse().unwrap()),
        )
        .init();

    let config = ServerConfig::from_env();

    let app = Router::new()
        .route("/api/joke", get(joke_handler))
        .route("/api/echo", post(echo_handler))
        .route("/api/time", get(time_handler))
        .route("/health", get(health_handler))
        .route("/.well-known/x402-info", get(manifest_handler));

    let addr = format!("0.0.0.0:{}", config.port);
    tracing::info!("x402 example service starting on {}", addr);
    tracing::info!("  GET  /api/joke  — random joke (100 sats)");
    tracing::info!("  POST /api/echo  — echo message (50 sats)");
    tracing::info!("  GET  /api/time  — server time (25 sats)");
    tracing::info!("  GET  /health    — health check (free)");
    tracing::info!("  GET  /.well-known/x402-info — service manifest");

    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
