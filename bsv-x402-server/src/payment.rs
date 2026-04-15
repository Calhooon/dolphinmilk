//! BRC-29 payment construction and 402 auto-retry flow.
//!
//! When a BRC-31 authenticated request gets an HTTP 402 response, the server
//! is requesting payment. This module:
//!   1. Parses the 402 response headers for payment requirements.
//!   2. Derives a payment public key via BRC-42 key derivation.
//!   3. Builds a P2PKH locking script for that derived key.
//!   4. Creates a funded transaction via the wallet (createAction).
//!   5. Packages the payment as JSON in the x-bsv-payment header.
//!   6. Retries the original request with the payment attached.

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use reqwest::header::HeaderMap;
use reqwest::{Client, Method, Response, StatusCode};
use ripemd::Ripemd160;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::error::X402Error;
use crate::traits::{AuthClient, WalletApi};

/// Payment key derivation protocol ID.
/// Matches the TS SDK AuthFetch.createPaymentContext() exactly.
pub fn payment_protocol() -> Value {
    json!([2, "3241645161d8"])
}

/// BRC-29 payment protocol version.
const PAYMENT_VERSION: &str = "1.0";

/// Maximum number of payment retry attempts.
const MAX_PAYMENT_ATTEMPTS: u32 = 3;

/// Threshold for switching from header to multipart payment transport.
/// When payment JSON exceeds this, we send it as a multipart/form-data body
/// part instead of the x-bsv-payment header (BRC-105 multipart transport).
/// 8KB gives safe margin under the ~16KB typical header limit.
const MULTIPART_THRESHOLD: usize = 8_192;

/// Build a multipart/form-data body containing the payment JSON and optional
/// original request body. Returns (body_bytes, content_type_with_boundary).
fn build_multipart_body(
    payment_json: &str,
    original_body: Option<&[u8]>,
    original_content_type: Option<&str>,
) -> (Vec<u8>, String) {
    let boundary = format!("----BsvPayment{:016x}", rand::random::<u64>());
    let mut buf = Vec::new();

    // Part 1: x-bsv-payment
    buf.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    buf.extend_from_slice(b"Content-Disposition: form-data; name=\"x-bsv-payment\"\r\n");
    buf.extend_from_slice(b"Content-Type: application/json\r\n");
    buf.extend_from_slice(b"\r\n");
    buf.extend_from_slice(payment_json.as_bytes());
    buf.extend_from_slice(b"\r\n");

    // Part 2: body (if present)
    if let Some(body) = original_body {
        let ct = original_content_type.unwrap_or("application/octet-stream");
        buf.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        buf.extend_from_slice(b"Content-Disposition: form-data; name=\"body\"\r\n");
        buf.extend_from_slice(format!("Content-Type: {ct}\r\n").as_bytes());
        buf.extend_from_slice(b"\r\n");
        buf.extend_from_slice(body);
        buf.extend_from_slice(b"\r\n");
    }

    // Closing boundary
    buf.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());

    let content_type = format!("multipart/form-data; boundary={boundary}");
    (buf, content_type)
}

/// Parsed 402 response data.
#[derive(Debug)]
pub struct PaymentRequired {
    pub satoshis: u64,
    pub derivation_prefix: String,
    pub version: String,
    /// Transports advertised by the server (e.g. ["header", "multipart"]).
    /// Empty if the server didn't send the header (legacy/third-party).
    pub transports: Vec<String>,
}

impl PaymentRequired {
    /// Whether the server advertises multipart transport support (BRC-105).
    pub fn supports_multipart(&self) -> bool {
        self.transports.iter().any(|t| t == "multipart")
    }
}

/// Parse payment requirements from a 402 response.
pub fn parse_402_response(headers: &HeaderMap) -> Result<PaymentRequired, X402Error> {
    let version = headers
        .get("x-bsv-payment-version")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| X402Error::payment("402 response missing x-bsv-payment-version header"))?;

    if version != PAYMENT_VERSION {
        return Err(X402Error::payment(format!(
            "Unsupported payment version: server={version}, client={PAYMENT_VERSION}"
        )));
    }

    let satoshis_str = headers
        .get("x-bsv-payment-satoshis-required")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| {
            X402Error::payment("402 response missing x-bsv-payment-satoshis-required header")
        })?;

    let satoshis: u64 = satoshis_str
        .parse()
        .map_err(|_| X402Error::payment(format!("Invalid satoshis value: {satoshis_str}")))?;

    if satoshis == 0 {
        return Err(X402Error::payment("satoshis-required must be > 0"));
    }

    let derivation_prefix = headers
        .get("x-bsv-payment-derivation-prefix")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| {
            X402Error::payment("402 response missing x-bsv-payment-derivation-prefix header")
        })?
        .to_string();

    let transports = headers
        .get("x-bsv-payment-transports")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.split(',').map(|t| t.trim().to_string()).collect())
        .unwrap_or_default();

    Ok(PaymentRequired {
        satoshis,
        derivation_prefix,
        version: version.to_string(),
        transports,
    })
}

/// Compute RIPEMD160(SHA256(data)) -- Bitcoin's hash160.
pub fn hash160(data: &[u8]) -> Vec<u8> {
    let sha256_digest = Sha256::digest(data);
    let ripemd_digest = Ripemd160::digest(sha256_digest);
    ripemd_digest.to_vec()
}

/// Build a P2PKH locking script from a hex public key.
pub fn build_p2pkh_script(pubkey_hex: &str) -> Result<String, X402Error> {
    if pubkey_hex.len() != 66 {
        return Err(X402Error::payment(format!(
            "Invalid public key length: expected 66 hex chars, got {}",
            pubkey_hex.len()
        )));
    }
    if !pubkey_hex.starts_with("02") && !pubkey_hex.starts_with("03") {
        return Err(X402Error::payment(format!(
            "Invalid compressed public key prefix: {}",
            &pubkey_hex[..2]
        )));
    }

    let pubkey_bytes =
        hex::decode(pubkey_hex).map_err(|e| X402Error::payment(format!("Invalid hex: {e}")))?;
    let pkh = hash160(&pubkey_bytes);

    // OP_DUP OP_HASH160 OP_PUSH20 <hash160> OP_EQUALVERIFY OP_CHECKSIG
    let mut script = vec![0x76, 0xa9, 0x14];
    script.extend_from_slice(&pkh);
    script.push(0x88);
    script.push(0xac);

    Ok(hex::encode(script))
}

/// Encode an integer as a Bitcoin CompactSize/VarInt.
fn varint(n: u64) -> Vec<u8> {
    if n < 0xFD {
        vec![n as u8]
    } else if n <= 0xFFFF {
        let mut v = vec![0xFD];
        v.extend_from_slice(&(n as u16).to_le_bytes());
        v
    } else if n <= 0xFFFFFFFF {
        let mut v = vec![0xFE];
        v.extend_from_slice(&(n as u32).to_le_bytes());
        v
    } else {
        let mut v = vec![0xFF];
        v.extend_from_slice(&n.to_le_bytes());
        v
    }
}

/// Wrap a raw transaction in BEEF format (BRC-62 v1).
pub fn raw_tx_to_beef(raw_tx: &[u8]) -> Vec<u8> {
    let mut buf = vec![0x01, 0x00, 0xBE, 0xEF]; // BEEF_V1 header
    buf.extend_from_slice(&varint(0)); // 0 BUMPs
    buf.extend_from_slice(&varint(1)); // 1 transaction
    buf.extend_from_slice(raw_tx); // raw tx
    buf.push(0x00); // hasBump = false
    buf
}

/// Wrap a raw transaction in AtomicBEEF format (raw -> BEEF -> AtomicBEEF).
pub fn raw_tx_to_atomic_beef(raw_tx: &[u8], txid_hex: &str) -> Vec<u8> {
    beef_to_atomic_beef(&raw_tx_to_beef(raw_tx), txid_hex)
}

/// Wrap BEEF bytes in AtomicBEEF format (prepend header + reversed txid).
pub fn beef_to_atomic_beef(beef: &[u8], txid_hex: &str) -> Vec<u8> {
    let mut buf = vec![0x01, 0x01, 0x01, 0x01]; // AtomicBEEF header
    let txid_bytes = hex::decode(txid_hex).unwrap_or_default();
    let reversed: Vec<u8> = txid_bytes.into_iter().rev().collect();
    buf.extend_from_slice(&reversed);
    buf.extend_from_slice(beef);
    buf
}

/// Create a BRC-29 payment. Returns (payment_json, txid).
pub async fn create_payment(
    wallet: &dyn WalletApi,
    derivation_prefix: &str,
    server_identity_key: &str,
    satoshis: u64,
    _server_url: &str,
) -> Result<(Value, String), X402Error> {
    // Step 1: Generate a random derivation suffix (base64)
    let suffix_bytes: [u8; 32] = rand::random();
    let derivation_suffix = BASE64.encode(suffix_bytes);

    tracing::info!(
        "Creating payment: {} sats to {}... (prefix={}...)",
        satoshis,
        &server_identity_key[..16],
        &derivation_prefix[..derivation_prefix.len().min(16)],
    );

    // Step 2: Derive payment public key via BRC-42
    let protocol_id = payment_protocol();
    let key_id = format!("{derivation_prefix} {derivation_suffix}");
    let payment_pubkey = wallet
        .get_public_key(&protocol_id, &key_id, server_identity_key, false)
        .await
        .map_err(|e| X402Error::payment(format!("Failed to derive payment key: {e}")))?;

    // Step 3: Build P2PKH locking script
    let locking_script = build_p2pkh_script(&payment_pubkey)?;

    // Step 4: Create the transaction via wallet
    let description = format!("BRC-29 payment: {satoshis} sats");

    let outputs = vec![json!({
        "lockingScript": locking_script,
        "satoshis": satoshis,
        "outputDescription": "BRC-29 payment output",
    })];

    // Retry wallet createAction on transient errors (e.g., SQLite "database is locked").
    // The wallet uses SQLite with WAL mode and a 5s busy_timeout, but under rapid sequential
    // payment requests the write lock can still be contended.
    let result = {
        let mut last_err = String::new();
        let mut succeeded = None;
        for retry in 0..3u32 {
            match wallet
                .create_action(&outputs, &description, false, false)
                .await
            {
                Ok(r) => {
                    if retry > 0 {
                        tracing::info!("create_action succeeded on retry {retry}");
                    }
                    succeeded = Some(r);
                    break;
                }
                Err(e) => {
                    last_err = format!("{e}");
                    let is_transient = last_err.contains("database is locked")
                        || last_err.contains("SQLITE_BUSY")
                        || last_err.contains("code: 517");
                    if is_transient && retry < 2 {
                        let delay_ms = 500 * (retry + 1) as u64;
                        tracing::warn!(
                            "create_action transient error (retry {}/2, backoff {}ms): {e}",
                            retry + 1,
                            delay_ms
                        );
                        tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                        continue;
                    }
                    break;
                }
            }
        }
        succeeded
            .ok_or_else(|| X402Error::payment(format!("Failed to create payment tx: {last_err}")))?
    };

    let mut tx_bytes = result.tx;
    let txid = &result.txid;

    let header_hex = if tx_bytes.len() >= 8 {
        tx_bytes[..8]
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    } else {
        tx_bytes
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    };
    tracing::info!(
        "Payment tx created: txid={}, size={} bytes, header={}",
        txid,
        tx_bytes.len(),
        header_hex
    );

    // Step 5: Ensure output is AtomicBEEF format.
    if tx_bytes.len() >= 4 {
        if tx_bytes[..4] == [0x01, 0x01, 0x01, 0x01] {
            tracing::info!("Payment format: AtomicBEEF (pass-through, no wrapping needed)");
        } else if tx_bytes[..4] == [0x01, 0x00, 0xBE, 0xEF] {
            tracing::info!("Payment format: BEEF -> wrapping in AtomicBEEF");
            tx_bytes = beef_to_atomic_beef(&tx_bytes, txid);
        } else if tx_bytes[..4] == [0x01, 0x00, 0x00, 0x00] {
            tracing::info!("Payment format: raw tx -> wrapping in BEEF+AtomicBEEF");
            tx_bytes = raw_tx_to_atomic_beef(&tx_bytes, txid);
        } else {
            tracing::warn!(
                "Payment format: UNKNOWN (header: {:02x}{:02x}{:02x}{:02x}) -> wrapping as raw tx (best-effort)",
                tx_bytes[0], tx_bytes[1], tx_bytes[2], tx_bytes[3]
            );
            tx_bytes = raw_tx_to_atomic_beef(&tx_bytes, txid);
        }
    } else {
        tracing::error!(
            "Payment tx has only {} bytes -- too small to be valid!",
            tx_bytes.len()
        );
    }

    // Step 6: Base64-encode
    let tx_b64 = BASE64.encode(&tx_bytes);

    // Step 7: Build payment JSON
    Ok((
        json!({
            "derivationPrefix": derivation_prefix,
            "derivationSuffix": derivation_suffix,
            "transaction": tx_b64,
        }),
        txid.clone(),
    ))
}

/// Make a request, handling 402 payment automatically.
///
/// Returns (response, payment_txid, sats_paid).
pub async fn paid_request(
    wallet: &dyn WalletApi,
    method: Method,
    url: &str,
    headers: Option<HeaderMap>,
    body: Option<String>,
) -> Result<(Response, Option<String>, Option<u64>), X402Error> {
    let client = Client::new();
    let extra_headers = headers.unwrap_or_default();

    tracing::info!("paid_request: {} {}", method, url);

    let mut req = client.request(method.clone(), url);
    req = req.headers(extra_headers.clone());
    if let Some(ref b) = body {
        req = req.body(b.clone());
    }

    let response = req
        .send()
        .await
        .map_err(|e| X402Error::payment(format!("Request failed: {e}")))?;

    if response.status() != StatusCode::PAYMENT_REQUIRED {
        return Ok((response, None, None));
    }

    tracing::info!("paid_request: got 402 Payment Required");

    let server_identity_key = response
        .headers()
        .get("x-bsv-auth-identity-key")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    let parsed_url =
        url::Url::parse(url).map_err(|e| X402Error::payment(format!("Invalid URL: {e}")))?;
    let server_base = format!(
        "{}://{}",
        parsed_url.scheme(),
        parsed_url.host_str().unwrap_or("unknown")
    );

    let mut last_response = response;

    for attempt in 1..=MAX_PAYMENT_ATTEMPTS {
        tracing::info!(
            "paid_request: payment attempt {}/{}",
            attempt,
            MAX_PAYMENT_ATTEMPTS
        );

        let payment_req = parse_402_response(last_response.headers())?;
        let payment_sats = payment_req.satoshis;

        let (payment, payment_txid) = create_payment(
            wallet,
            &payment_req.derivation_prefix,
            &server_identity_key,
            payment_req.satoshis,
            &server_base,
        )
        .await?;

        let payment_json = serde_json::to_string(&payment)
            .map_err(|e| X402Error::payment(format!("Failed to serialize payment: {e}")))?;

        let mut retry_headers = extra_headers.clone();
        let mut req = client.request(method.clone(), url);

        if payment_json.len() > MULTIPART_THRESHOLD && payment_req.supports_multipart() {
            tracing::info!(
                "Payment JSON {} bytes > {} threshold, using multipart transport",
                payment_json.len(),
                MULTIPART_THRESHOLD
            );
            let original_ct = extra_headers
                .get("content-type")
                .and_then(|v| v.to_str().ok());
            let (multipart_body, multipart_ct) = build_multipart_body(
                &payment_json,
                body.as_deref().map(|s| s.as_bytes()),
                original_ct,
            );
            retry_headers.insert(
                "content-type",
                multipart_ct
                    .parse()
                    .map_err(|_| X402Error::payment("Invalid multipart content-type"))?,
            );
            req = req.headers(retry_headers);
            req = req.body(multipart_body);
        } else {
            retry_headers.insert(
                "x-bsv-payment",
                payment_json.parse().map_err(|_| {
                    X402Error::payment("Payment JSON contains invalid header characters")
                })?,
            );
            req = req.headers(retry_headers);
            if let Some(ref b) = body {
                req = req.body(b.clone());
            }
        }

        let response = req
            .send()
            .await
            .map_err(|e| X402Error::payment(format!("Paid request failed: {e}")))?;

        if response.status() == StatusCode::PAYMENT_REQUIRED {
            tracing::warn!(
                "Server returned 402 again after payment (attempt {}/{})",
                attempt,
                MAX_PAYMENT_ATTEMPTS
            );
            last_response = response;
            if attempt >= MAX_PAYMENT_ATTEMPTS {
                return Err(X402Error::payment(format!(
                    "Still getting 402 after {attempt} payment attempts"
                )));
            }
            continue;
        }

        tracing::info!(
            "paid_request: payment accepted! status={} (attempt {}/{})",
            response.status(),
            attempt,
            MAX_PAYMENT_ATTEMPTS
        );

        return Ok((response, Some(payment_txid), Some(payment_sats)));
    }

    Err(X402Error::payment(format!(
        "Paid request to {url} exhausted all {MAX_PAYMENT_ATTEMPTS} attempts"
    )))
}

/// Result of an authenticated + paid request.
pub struct AuthPaidResponse {
    pub body: Vec<u8>,
    pub status: StatusCode,
    pub payment_txid: Option<String>,
    pub sats_paid: Option<u64>,
}

/// BRC-31 authenticated request with automatic 402 payment retry.
pub async fn authenticated_paid_request(
    auth: &dyn AuthClient,
    method: &str,
    url: &str,
    headers: &[(String, String)],
    body: Option<&[u8]>,
) -> Result<AuthPaidResponse, X402Error> {
    let mut response = auth
        .authenticated_request(method, url, headers, body)
        .await?;

    let mut status = response.status();

    if status == StatusCode::UNAUTHORIZED {
        tracing::warn!(
            "authenticated_paid_request: got 401 for {url}, clearing stale session and retrying"
        );
        let base = auth.base_url_from(url);
        auth.clear_session(&base);

        let retry_response = auth
            .authenticated_request(method, url, headers, body)
            .await?;

        status = retry_response.status();
        response = retry_response;
    }

    if status != StatusCode::PAYMENT_REQUIRED {
        let resp_bytes = response
            .bytes()
            .await
            .map_err(|e| X402Error::payment(format!("Failed to read response: {e}")))?
            .to_vec();
        return Ok(AuthPaidResponse {
            body: resp_bytes,
            status,
            payment_txid: None,
            sats_paid: None,
        });
    }

    tracing::info!("authenticated_paid_request: got 402 for {url}");

    let server_identity_key = response
        .headers()
        .get("x-bsv-auth-identity-key")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    let mut last_headers = response.headers().clone();
    let mut auth_retried = false;

    for attempt in 1..=MAX_PAYMENT_ATTEMPTS {
        tracing::info!(
            "authenticated_paid_request: payment attempt {}/{}",
            attempt,
            MAX_PAYMENT_ATTEMPTS
        );

        let payment_req = parse_402_response(&last_headers)?;
        let payment_sats = payment_req.satoshis;

        let (payment_val, payment_txid) = create_payment(
            auth.wallet(),
            &payment_req.derivation_prefix,
            &server_identity_key,
            payment_req.satoshis,
            url,
        )
        .await?;

        let payment_json = serde_json::to_string(&payment_val)
            .map_err(|e| X402Error::payment(format!("Failed to serialize payment: {e}")))?;

        let response =
            if payment_json.len() > MULTIPART_THRESHOLD && payment_req.supports_multipart() {
                tracing::info!(
                    "Payment JSON {} bytes > {} threshold, using multipart transport",
                    payment_json.len(),
                    MULTIPART_THRESHOLD
                );
                let original_ct = headers
                    .iter()
                    .find(|(k, _)| k.to_lowercase() == "content-type")
                    .map(|(_, v)| v.as_str());
                let (multipart_body, multipart_ct) =
                    build_multipart_body(&payment_json, body, original_ct);
                let mut retry_headers: Vec<(String, String)> = headers
                    .iter()
                    .filter(|(k, _)| k.to_lowercase() != "content-type")
                    .cloned()
                    .collect();
                retry_headers.push(("content-type".into(), multipart_ct));
                auth.authenticated_request(method, url, &retry_headers, Some(&multipart_body))
                    .await?
            } else {
                let mut retry_headers: Vec<(String, String)> = headers.to_vec();
                retry_headers.push(("x-bsv-payment".into(), payment_json));
                auth.authenticated_request(method, url, &retry_headers, body)
                    .await?
            };

        if response.status() == StatusCode::UNAUTHORIZED && !auth_retried {
            tracing::warn!(
                "authenticated_paid_request: got 401 during payment attempt {}/{}, clearing session and retrying",
                attempt, MAX_PAYMENT_ATTEMPTS
            );
            let base = auth.base_url_from(url);
            auth.clear_session(&base);
            auth_retried = true;
            let fresh_response = auth
                .authenticated_request(method, url, headers, body)
                .await?;
            if fresh_response.status() == StatusCode::PAYMENT_REQUIRED {
                last_headers = fresh_response.headers().clone();
                continue;
            }
            let final_status = fresh_response.status();
            let resp_bytes = fresh_response
                .bytes()
                .await
                .map_err(|e| X402Error::payment(format!("Failed to read response: {e}")))?
                .to_vec();
            return Ok(AuthPaidResponse {
                body: resp_bytes,
                status: final_status,
                payment_txid: None,
                sats_paid: None,
            });
        }

        if response.status() == StatusCode::PAYMENT_REQUIRED {
            tracing::warn!(
                "Server returned 402 again after payment (attempt {}/{})",
                attempt,
                MAX_PAYMENT_ATTEMPTS
            );
            last_headers = response.headers().clone();
            if attempt >= MAX_PAYMENT_ATTEMPTS {
                return Err(X402Error::payment(format!(
                    "Still getting 402 after {attempt} payment attempts"
                )));
            }
            continue;
        }

        let final_status = response.status();
        tracing::info!(
            "authenticated_paid_request: payment accepted! status={} (attempt {}/{})",
            final_status,
            attempt,
            MAX_PAYMENT_ATTEMPTS
        );

        let resp_bytes = response
            .bytes()
            .await
            .map_err(|e| X402Error::payment(format!("Failed to read response: {e}")))?
            .to_vec();

        return Ok(AuthPaidResponse {
            body: resp_bytes,
            status: final_status,
            payment_txid: Some(payment_txid),
            sats_paid: Some(payment_sats),
        });
    }

    Err(X402Error::payment(format!(
        "authenticated_paid_request to {url} exhausted all {MAX_PAYMENT_ATTEMPTS} attempts"
    )))
}
